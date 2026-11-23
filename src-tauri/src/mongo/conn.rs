//! One MongoDB connection: `OP_MSG` framing, the `hello` handshake, TLS, SCRAM-SHA-256 sign-in, and `command`, which
//! sends a BSON command and returns the reply document or the server's own error.

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::time::Instant;

use base64::Engine as _;
use bson::{doc, Bson, Document};
use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_rustls::rustls;
use zeroize::Zeroizing;

use crate::dbs::transport::{tls_client_config, tls_error, BoxStream, TlsMode, CONNECT_TIMEOUT};
use crate::dbs::Engine;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::sql::pg::Reopen;

/// Largest message Dalan reads (MongoDB's own limit is 48 MB).
const MAX_MESSAGE: usize = 64 * 1024 * 1024;
const OP_MSG: i32 = 2013;
/// One command may take this long (`find`/`aggregate` can be slow); a hung server then frees the connection.
const COMMAND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

#[derive(Clone)]
pub struct MongoLogin {
    pub user: String,
    pub password: Option<Zeroizing<String>>,
    /// The database the user is defined in (`admin` unless the connection says otherwise).
    pub auth_db: String,
    pub tls: TlsMode,
    pub server_name: String,
}

#[derive(Debug, Clone)]
pub struct MongoInfo {
    pub version: String,
    pub user: String,
    pub latency_ms: u32,
    /// `Some` when the server is part of a replica set.
    pub replica_set: Option<String>,
    pub is_primary: bool,
    /// The primary's `host:port` as a secondary reports it.
    pub primary: Option<String>,
}

pub struct MongoConn {
    stream: tokio::sync::Mutex<BoxStream>,
    next_id: AtomicI32,
    pub wire_version: i32,
    pub version: String,
    reopen: Reopen,
    login: MongoLogin,
    closed: AtomicBool,
}

fn closed() -> AppError {
    AppError::new(
        ErrorCode::DbUnreachable,
        "The connection to MongoDB closed before it answered. Is this really a MongoDB port?",
    )
}

fn number(b: Option<&Bson>) -> i64 {
    match b {
        Some(Bson::Int32(i)) => i64::from(*i),
        Some(Bson::Int64(i)) => *i,
        Some(Bson::Double(d)) => *d as i64,
        _ => 0,
    }
}

/// The server's error document as ours: its own words always, `DB_AUTH_FAILED` for the authentication codes.
pub fn server_error(reply: &Document) -> AppError {
    let msg = reply
        .get_str("errmsg")
        .unwrap_or("The server refused that.")
        .to_string();
    match number(reply.get("code")) {
        18 => AppError::new(
            ErrorCode::DbAuthFailed,
            "Wrong username or password. MongoDB refused the sign-in.",
        ),
        code => AppError::new(
            ErrorCode::DbRefused,
            if code == 13 {
                format!("{msg} (the user isn't allowed to do that)")
            } else {
                msg
            },
        ),
    }
}

async fn read_message(s: &mut BoxStream) -> AppResult<(i32, Document)> {
    let mut len = [0u8; 4];
    s.read_exact(&mut len).await.map_err(|_| closed())?;
    let total = i32::from_le_bytes(len);
    if !(21..=MAX_MESSAGE as i32).contains(&total) {
        return Err(AppError::new(
            ErrorCode::DbRefused,
            "MongoDB sent a message Dalan won't read (too large or malformed).",
        ));
    }
    let mut rest = vec![0u8; total as usize - 4];
    s.read_exact(&mut rest).await.map_err(|_| closed())?;
    // requestID, responseTo, opCode, flagBits, then sections: kind 0 is one document.
    let word = |at: usize| i32::from_le_bytes([rest[at], rest[at + 1], rest[at + 2], rest[at + 3]]);
    let (id, to, op, flags) = (word(0), word(4), word(8), word(12) as u32);
    let bad = |why: &str| {
        AppError::new(
            ErrorCode::DbRefused,
            format!("MongoDB answered in a form Dalan doesn't read ({why})."),
        )
    };
    if op != OP_MSG || rest[16] != 0 {
        return Err(bad("not a plain OP_MSG reply"));
    }
    // Bit 1 (moreToCome) would leave replies unread on the stream; bit 0 (checksum) adds 4 trailing bytes.
    if flags & 0b10 != 0 {
        return Err(bad("moreToCome"));
    }
    let mut body = &rest[17..];
    if flags & 1 != 0 {
        body = &body[..body.len().saturating_sub(4)];
    }
    let doc = Document::from_reader(&mut body).map_err(|_| bad("a damaged document"))?;
    if !body.is_empty() {
        return Err(bad("trailing bytes"));
    }
    let _ = id;
    Ok((to, doc))
}

async fn send(s: &mut BoxStream, request_id: i32, command: &Document) -> AppResult<()> {
    let mut body = Vec::new();
    command
        .to_writer(&mut body)
        .map_err(|e| AppError::validation(format!("That command can't be sent: {e}")))?;
    let mut msg = Vec::with_capacity(21 + body.len());
    msg.extend_from_slice(&((21 + body.len()) as i32).to_le_bytes());
    msg.extend_from_slice(&request_id.to_le_bytes());
    msg.extend_from_slice(&0i32.to_le_bytes());
    msg.extend_from_slice(&OP_MSG.to_le_bytes());
    msg.extend_from_slice(&0u32.to_le_bytes());
    msg.push(0);
    msg.extend_from_slice(&body);
    s.write_all(&msg).await.map_err(|_| closed())?;
    s.flush().await.map_err(|_| closed())
}

fn hmac(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut m = <Hmac<Sha256> as KeyInit>::new_from_slice(key)
        .unwrap_or_else(|_| unreachable!("HMAC takes any key length"));
    m.update(data);
    m.finalize().into_bytes().to_vec()
}

/// SCRAM-SHA-256 (RFC 5802 and 7677), MongoDB's flavour: the password is used as typed (SASLprep isn't applied: an
/// ASCII password is unaffected).
struct Scram {
    password: Zeroizing<String>,
    nonce: String,
    first_bare: String,
    auth_message: String,
    server_key: Zeroizing<Vec<u8>>,
}

impl Scram {
    fn start(user: &str, password: &str) -> (Scram, Vec<u8>) {
        use rand::RngExt;
        let mut raw = [0u8; 24];
        rand::rng().fill(&mut raw);
        let nonce = base64::engine::general_purpose::STANDARD.encode(raw);
        let first_bare = format!(
            "n={},r={nonce}",
            user.replace('=', "=3D").replace(',', "=2C")
        );
        let payload = format!("n,,{first_bare}").into_bytes();
        (
            Scram {
                password: Zeroizing::new(password.into()),
                nonce,
                first_bare,
                auth_message: String::new(),
                server_key: Zeroizing::new(vec![]),
            },
            payload,
        )
    }

    /// The server's first message to the client's final one.
    fn proof(&mut self, server_first: &str) -> AppResult<Vec<u8>> {
        let bad = || {
            AppError::new(
                ErrorCode::DbRefused,
                "MongoDB's sign-in answer wasn't in the form Dalan expects.",
            )
        };
        let part = |k: &str| server_first.split(',').find_map(|p| p.strip_prefix(k));
        let (r, salt, iters) = (
            part("r=").ok_or_else(bad)?,
            part("s=").ok_or_else(bad)?,
            part("i=").ok_or_else(bad)?,
        );
        if !r.starts_with(&self.nonce) || r.len() == self.nonce.len() {
            return Err(AppError::new(ErrorCode::DbRefused, "MongoDB's sign-in answer didn't carry our nonce: the connection may be tampered with."));
        }
        let iters: u32 = iters.parse().map_err(|_| bad())?;
        if !(4096..=1_000_000).contains(&iters) {
            return Err(AppError::new(
                ErrorCode::DbRefused,
                "MongoDB asked for an unusual number of password rounds.",
            ));
        }
        let salt = base64::engine::general_purpose::STANDARD
            .decode(salt)
            .map_err(|_| bad())?;
        let mut salted = Zeroizing::new([0u8; 32]);
        pbkdf2::pbkdf2_hmac::<Sha256>(self.password.as_bytes(), &salt, iters, &mut *salted);
        let client_key = Zeroizing::new(hmac(&*salted, b"Client Key"));
        let stored = Sha256::digest(&client_key);
        let without_proof = format!("c=biws,r={r}");
        self.auth_message = format!("{},{server_first},{without_proof}", self.first_bare);
        let sig = hmac(&stored, self.auth_message.as_bytes());
        let proof: Vec<u8> = client_key.iter().zip(&sig).map(|(a, b)| a ^ b).collect();
        self.server_key = Zeroizing::new(hmac(&*salted, b"Server Key"));
        Ok(format!(
            "{without_proof},p={}",
            base64::engine::general_purpose::STANDARD.encode(proof)
        )
        .into_bytes())
    }

    /// The server's final message must prove it knows the password too.
    fn verify(&self, server_final: &str) -> AppResult<()> {
        let v = server_final
            .split(',')
            .find_map(|p| p.strip_prefix("v="))
            .and_then(|v| base64::engine::general_purpose::STANDARD.decode(v).ok());
        let mut m = <Hmac<Sha256> as KeyInit>::new_from_slice(&self.server_key)
            .unwrap_or_else(|_| unreachable!("HMAC takes any key length"));
        m.update(self.auth_message.as_bytes());
        // verify_slice compares in constant time.
        if v.is_some_and(|v| m.verify_slice(&v).is_ok()) {
            Ok(())
        } else {
            Err(AppError::new(
                ErrorCode::DbRefused,
                "MongoDB's sign-in proof was wrong: this may not be the server it claims to be.",
            ))
        }
    }
}

fn binary(b: Vec<u8>) -> Bson {
    Bson::Binary(bson::Binary {
        subtype: bson::spec::BinarySubtype::Generic,
        bytes: b,
    })
}
fn payload(d: &Document) -> AppResult<String> {
    match d.get("payload") {
        Some(Bson::Binary(b)) => Ok(String::from_utf8_lossy(&b.bytes).into_owned()),
        _ => Err(AppError::new(
            ErrorCode::DbRefused,
            "MongoDB's sign-in answer had no payload.",
        )),
    }
}

impl MongoConn {
    /// TLS (if asked), `hello`, then the SCRAM sign-in when there are credentials.
    pub async fn connect(
        stream: BoxStream,
        login: MongoLogin,
        reopen: Reopen,
    ) -> AppResult<(MongoConn, MongoInfo)> {
        let started = Instant::now();
        let mut s = stream;
        if login.tls != TlsMode::Off {
            let name = rustls::pki_types::ServerName::try_from(login.server_name.clone()).map_err(
                |_| {
                    AppError::validation(format!(
                        "{} can't be used as a TLS server name.",
                        login.server_name
                    ))
                },
            )?;
            let connector = tokio_rustls::TlsConnector::from(tls_client_config(login.tls)?);
            let tls = tokio::time::timeout(CONNECT_TIMEOUT, connector.connect(name, s))
                .await
                .map_err(|_| {
                    AppError::new(
                        ErrorCode::DbUnreachable,
                        "The TLS handshake took too long. Is this port really TLS?",
                    )
                })?
                .map_err(|e| tls_error(Engine::Mongodb, &e, &login.server_name))?;
            s = Box::new(tls);
        }
        let conn = MongoConn {
            stream: tokio::sync::Mutex::new(s),
            next_id: AtomicI32::new(1),
            wire_version: 0,
            version: String::new(),
            reopen,
            login: login.clone(),
            closed: AtomicBool::new(false),
        };
        let mut hello_cmd = doc! { "hello": 1, "client": { "application": { "name": "Dalan" }, "driver": { "name": "dalan", "version": env!("CARGO_PKG_VERSION") }, "os": { "type": std::env::consts::OS } } };
        if !login.user.is_empty() {
            hello_cmd.insert(
                "saslSupportedMechs",
                format!("{}.{}", login.auth_db, login.user),
            );
        }
        let hello = tokio::time::timeout(CONNECT_TIMEOUT, conn.command("admin", hello_cmd))
            .await
            .map_err(|_| closed())??;
        let wire = hello.get_i32("maxWireVersion").unwrap_or(0);
        if wire < 6 {
            return Err(AppError::new(
                ErrorCode::DbRefused,
                "This MongoDB server is older than 3.6, which Dalan can't talk to.",
            ));
        }
        let mut conn = conn;
        conn.wire_version = wire;
        if !login.user.is_empty() {
            let mechs: Vec<&str> = hello
                .get_array("saslSupportedMechs")
                .map(|a| a.iter().filter_map(Bson::as_str).collect())
                .unwrap_or_default();
            if !mechs.is_empty() && !mechs.contains(&"SCRAM-SHA-256") {
                return Err(AppError::new(ErrorCode::DbRefused, format!("This user signs in with {}, which Dalan doesn't speak yet (it does SCRAM-SHA-256).", mechs.join(" or "))));
            }
            let signed = tokio::time::timeout(CONNECT_TIMEOUT * 2, conn.scram())
                .await
                .map_err(|_| closed())?;
            if let Err(mut e) = signed {
                // SASLprep isn't applied, so a non-ASCII password may differ from what the server stored.
                if e.code == ErrorCode::DbAuthFailed
                    && login.password.as_ref().is_some_and(|p| !p.is_ascii())
                {
                    e.message.push_str(
                        " The password has non-ASCII characters, which Dalan sends as typed.",
                    );
                }
                return Err(e);
            }
        }
        let build = conn.command("admin", doc! { "buildInfo": 1 }).await.ok();
        let version = build
            .as_ref()
            .and_then(|b| b.get_str("version").ok())
            .unwrap_or("?")
            .to_string();
        let info = MongoInfo {
            version: format!("MongoDB {version}"),
            user: if login.user.is_empty() {
                String::new()
            } else {
                format!("{}@{}", login.user, login.auth_db)
            },
            latency_ms: started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32,
            replica_set: hello.get_str("setName").ok().map(str::to_string),
            is_primary: hello.get_bool("isWritablePrimary").unwrap_or(false),
            primary: hello.get_str("primary").ok().map(str::to_string),
        };
        let mut conn = conn;
        conn.version = version;
        Ok((conn, info))
    }

    async fn scram(&self) -> AppResult<()> {
        let pass = self.login.password.as_ref().map_or("", |p| p.as_str());
        let (mut scram, first) = Scram::start(&self.login.user, pass);
        let db = self.login.auth_db.clone();
        let r1 = self.command(&db, doc! { "saslStart": 1, "mechanism": "SCRAM-SHA-256", "payload": binary(first), "autoAuthorize": 1, "options": { "skipEmptyExchange": true } }).await?;
        let id = r1.get("conversationId").cloned().unwrap_or(Bson::Int32(1));
        let final_msg = scram.proof(&payload(&r1)?)?;
        let r2 = self.command(&db, doc! { "saslContinue": 1, "conversationId": id.clone(), "payload": binary(final_msg) }).await?;
        scram.verify(&payload(&r2)?)?;
        if !r2.get_bool("done").unwrap_or(false) {
            let r3 = self
                .command(
                    &db,
                    doc! { "saslContinue": 1, "conversationId": id, "payload": binary(vec![]) },
                )
                .await?;
            if !r3.get_bool("done").unwrap_or(false) {
                return Err(AppError::new(
                    ErrorCode::DbRefused,
                    "MongoDB didn't finish the sign-in.",
                ));
            }
        }
        Ok(())
    }

    /// One command on `db`. The reply is returned when `ok` is 1; otherwise the server's own error.
    pub async fn command(&self, db: &str, mut cmd: Document) -> AppResult<Document> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(closed());
        }
        cmd.insert("$db", db);
        let mut s = self.stream.lock().await;
        // A future dropped half way leaves the stream mid-exchange: the flag stays set and the connection is dead.
        self.closed.store(true, Ordering::SeqCst);
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let exchange = async {
            send(&mut s, id, &cmd).await?;
            let (to, reply) = read_message(&mut s).await?;
            if to != id {
                return Err(AppError::new(
                    ErrorCode::DbRefused,
                    "MongoDB answered a different request than the one sent.",
                ));
            }
            Ok::<Document, AppError>(reply)
        };
        // Any failure of the exchange itself leaves the stream in an unknown place: the connection stays dead.
        // Only a whole exchange resets the flag (a server error below is a complete exchange).
        let out = tokio::time::timeout(COMMAND_TIMEOUT, exchange)
            .await
            .unwrap_or_else(|_| {
                Err(AppError::new(
                    ErrorCode::DbUnreachable,
                    "MongoDB didn't answer in time; the connection was closed.",
                ))
            });
        let reply = out?;
        self.closed.store(false, Ordering::SeqCst);
        let ok = match reply.get("ok") {
            Some(Bson::Double(d)) => *d >= 1.0,
            Some(Bson::Int32(i)) => *i >= 1,
            Some(Bson::Int64(i)) => *i >= 1,
            _ => false,
        };
        if !ok {
            return Err(server_error(&reply));
        }
        // Writes answer ok:1 and list what failed beside it.
        if let Some(Bson::Document(w)) = reply.get_array("writeErrors").ok().and_then(|a| a.first())
        {
            return Err(server_error(w));
        }
        if let Ok(w) = reply.get_document("writeConcernError") {
            return Err(server_error(w));
        }
        Ok(reply)
    }

    /// A second connection by the same route, signed in the same way (for `killOp`).
    pub async fn open_another(&self) -> AppResult<MongoConn> {
        let stream = (self.reopen)().await?;
        Ok(MongoConn::connect(
            stream,
            self.login.clone(),
            std::sync::Arc::clone(&self.reopen),
        )
        .await?
        .0)
    }

    pub async fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 7677's example exchange (SCRAM-SHA-256), with the nonces fixed.
    #[test]
    fn scram_sha_256_matches_the_rfc_example() {
        let mut s = Scram {
            password: Zeroizing::new("pencil".into()),
            nonce: "rOprNGfwEbeRWgbNEkqO".into(),
            first_bare: "n=user,r=rOprNGfwEbeRWgbNEkqO".into(),
            auth_message: String::new(),
            server_key: Zeroizing::new(vec![]),
        };
        let server_first = "r=rOprNGfwEbeRWgbNEkqO%hvYDpWUa2RaTCAfuxFIlj)hNlF$k0,s=W22ZaJ0SNY7soEsUEjb6gQ==,i=4096";
        let fin = String::from_utf8(s.proof(server_first).unwrap()).unwrap();
        assert_eq!(fin, "c=biws,r=rOprNGfwEbeRWgbNEkqO%hvYDpWUa2RaTCAfuxFIlj)hNlF$k0,p=dHzbZapWIk4jUhN+Ute9ytag9zjfMHgsqmmiz7AndVQ=");
        assert!(s
            .verify("v=6rriTRBi23WpRR/wtup+mMhUZUn/dB5nLTJRsjl95G4=")
            .is_ok());
        assert!(s.verify("v=AAAA").is_err());
        // A server that doesn't know our nonce is refused.
        assert!(s
            .proof("r=other,s=W22ZaJ0SNY7soEsUEjb6gQ==,i=4096")
            .is_err());
        assert!(s
            .proof("r=rOprNGfwEbeRWgbNEkqOx,s=W22ZaJ0SNY7soEsUEjb6gQ==,i=1")
            .is_err());
    }

    #[tokio::test]
    async fn framing_refuses_bad_replies_and_a_failed_exchange_kills_the_connection() {
        use tokio::io::AsyncWriteExt;
        async fn reply(bytes: Vec<u8>) -> AppResult<(i32, Document)> {
            let (mut a, b) = tokio::io::duplex(1 << 16);
            a.write_all(&bytes).await.unwrap();
            drop(a);
            let mut s: BoxStream = Box::new(b);
            read_message(&mut s).await
        }
        fn msg(op: i32, flags: u32, kind: u8, d: &Document) -> Vec<u8> {
            let mut body = Vec::new();
            d.to_writer(&mut body).unwrap();
            let mut m = ((21 + body.len()) as i32).to_le_bytes().to_vec();
            for w in [
                7i32.to_le_bytes(),
                3i32.to_le_bytes(),
                op.to_le_bytes(),
                flags.to_le_bytes(),
            ] {
                m.extend_from_slice(&w);
            }
            m.push(kind);
            m.extend_from_slice(&body);
            m
        }
        let d = doc! { "ok": 1.0 };
        let (to, got) = reply(msg(OP_MSG, 0, 0, &d)).await.unwrap();
        assert_eq!((to, got), (3, d.clone()));
        assert!(reply(msg(2012, 0, 0, &d)).await.is_err(), "wrong opcode");
        assert!(
            reply(msg(OP_MSG, 0, 1, &d)).await.is_err(),
            "kind 1 section"
        );
        assert!(reply(msg(OP_MSG, 2, 0, &d)).await.is_err(), "moreToCome");
        assert!(
            reply(10i32.to_le_bytes().to_vec()).await.is_err(),
            "too short"
        );
        assert!(
            reply(i32::MAX.to_le_bytes().to_vec()).await.is_err(),
            "too long"
        );
        let mut cut = msg(OP_MSG, 0, 0, &d);
        cut.truncate(cut.len() - 3);
        assert!(reply(cut).await.is_err(), "truncated");
        // A malformed reply leaves the stream in an unknown place: the connection is dead afterwards.
        let (a, mut peer) = tokio::io::duplex(1 << 16);
        peer.write_all(&10i32.to_le_bytes()).await.unwrap();
        let conn = MongoConn {
            stream: tokio::sync::Mutex::new(Box::new(a)),
            next_id: AtomicI32::new(1),
            wire_version: 17,
            version: String::new(),
            reopen: std::sync::Arc::new(|| Box::pin(async { Err(closed()) })),
            login: MongoLogin {
                user: String::new(),
                password: None,
                auth_db: "admin".into(),
                tls: TlsMode::Off,
                server_name: String::new(),
            },
            closed: AtomicBool::new(false),
        };
        assert!(conn.command("admin", doc! { "ping": 1 }).await.is_err());
        assert!(conn.closed.load(Ordering::SeqCst));
    }

    #[test]
    fn errors_keep_the_servers_words_and_name_a_bad_sign_in() {
        let e = server_error(&doc! { "ok": 0.0, "code": 18, "errmsg": "Authentication failed." });
        assert_eq!(e.code, ErrorCode::DbAuthFailed);
        let e = server_error(
            &doc! { "ok": 0.0, "code": 13, "errmsg": "not authorized on app to execute command" },
        );
        assert_eq!(e.code, ErrorCode::DbRefused);
        assert!(e
            .message
            .starts_with("not authorized on app to execute command"));
        assert_eq!(server_error(&doc! { "ok": 0.0 }).code, ErrorCode::DbRefused);
        assert_eq!(
            server_error(&doc! { "code": 18i64, "errmsg": "x" }).code,
            ErrorCode::DbAuthFailed
        );
    }
}

#[cfg(test)]
mod docker_tests {
    use super::*;
    use crate::dbs::transport::{self, Target};

    fn reopen_to(port: u16) -> Reopen {
        std::sync::Arc::new(move || {
            Box::pin(async move {
                let t = Target {
                    address: "127.0.0.1".into(),
                    port,
                    tls: false,
                    tls_server_name: None,
                };
                transport::direct_for(Engine::Mongodb, &t).await
            })
        })
    }

    async fn connect(pass: &str, tls: TlsMode) -> AppResult<(MongoConn, MongoInfo)> {
        let stream = (reopen_to(17017))().await?;
        let login = MongoLogin {
            user: "app".into(),
            password: Some(Zeroizing::new(pass.into())),
            auth_db: "admin".into(),
            tls,
            server_name: "127.0.0.1".into(),
        };
        MongoConn::connect(stream, login, reopen_to(17017)).await
    }

    #[tokio::test]
    #[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
    async fn l3_mongo_signs_in_plain_and_with_tls_runs_commands_and_names_a_bad_password() {
        for tls in [TlsMode::Off, TlsMode::Require] {
            let (c, info) = connect("test-mongo", tls)
                .await
                .unwrap_or_else(|e| panic!("{tls:?}: {}", e.message));
            assert!(info.version.starts_with("MongoDB 7"), "{}", info.version);
            assert_eq!(info.user, "app@admin");
            assert!(c.wire_version >= 17);
            let dbs = c
                .command("admin", doc! { "listDatabases": 1, "nameOnly": true })
                .await
                .unwrap();
            assert!(dbs
                .get_array("databases")
                .unwrap()
                .iter()
                .any(|d| d.as_document().and_then(|d| d.get_str("name").ok()) == Some("admin")));
            // Write and read back through our own client: types survive (Int64, Decimal128, DateTime, ObjectId, null).
            let id = bson::oid::ObjectId::new();
            c.command(
                "d8",
                doc! { "delete": "types", "deletes": [{ "q": {}, "limit": 0 }] },
            )
            .await
            .unwrap();
            c.command("d8", doc! { "insert": "types", "documents": [{ "_id": id, "big": 9_007_199_254_740_993i64, "dec": bson::Decimal128::from_bytes([0u8; 16]), "when": bson::DateTime::from_millis(1_700_000_000_000), "nothing": Bson::Null, "name": "né 😀" }] }).await.unwrap();
            let found = c
                .command("d8", doc! { "find": "types", "filter": { "_id": id } })
                .await
                .unwrap();
            let batch = found
                .get_document("cursor")
                .unwrap()
                .get_array("firstBatch")
                .unwrap();
            let d = batch[0].as_document().unwrap();
            assert_eq!(d.get_i64("big").unwrap(), 9_007_199_254_740_993);
            assert_eq!(d.get_str("name").unwrap(), "né 😀");
            assert!(
                matches!(d.get("when"), Some(Bson::DateTime(_)))
                    && matches!(d.get("nothing"), Some(Bson::Null))
                    && matches!(d.get("dec"), Some(Bson::Decimal128(_)))
            );
            // The server's own error comes back whole, and the connection is in step afterwards.
            let e = c
                .command("d8", doc! { "find": "types", "filter": { "$bogus": 1 } })
                .await
                .unwrap_err();
            assert_eq!(e.code, ErrorCode::DbRefused);
            assert!(e.message.contains("$bogus"), "{}", e.message);
            assert!(c.command("admin", doc! { "ping": 1 }).await.is_ok());
            // A failed write answers ok:1 with writeErrors: it must be an error here.
            let dup = c
                .command(
                    "d8",
                    doc! { "insert": "types", "documents": [{ "_id": id }] },
                )
                .await
                .unwrap_err();
            assert!(dup.message.contains("E11000"), "{}", dup.message);
            assert!(c.command("admin", doc! { "ping": 1 }).await.is_ok());
        }
        let e = connect("nope", TlsMode::Off).await.err().unwrap();
        assert_eq!(e.code, ErrorCode::DbAuthFailed, "{}", e.message);
        // Nothing but hello before signing in: a command without credentials is refused by the server.
        let stream = (reopen_to(17017))().await.unwrap();
        let (anon, _) = MongoConn::connect(
            stream,
            MongoLogin {
                user: String::new(),
                password: None,
                auth_db: "admin".into(),
                tls: TlsMode::Off,
                server_name: String::new(),
            },
            reopen_to(17017),
        )
        .await
        .unwrap();
        assert_eq!(
            anon.command("admin", doc! { "listDatabases": 1 })
                .await
                .unwrap_err()
                .code,
            ErrorCode::DbRefused
        );
    }

    #[tokio::test]
    #[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
    async fn l3_mongo_through_a_host_with_tls() {
        use crate::dbs::spike::sign_in_to_a;
        use crate::ssh::tests::harness;
        let h = harness(None).await;
        let ssh = sign_in_to_a(&h).await;
        let target = Target {
            address: "mongo".into(),
            port: 27017,
            tls: false,
            tls_server_name: None,
        };
        let stream = transport::through_host_for(Engine::Mongodb, &ssh, &target)
            .await
            .unwrap();
        let login = MongoLogin {
            user: "app".into(),
            password: Some(Zeroizing::new("test-mongo".into())),
            auth_db: "admin".into(),
            tls: TlsMode::Require,
            server_name: "mongo".into(),
        };
        let (c, info) = MongoConn::connect(stream, login, reopen_to(1))
            .await
            .unwrap_or_else(|e| panic!("{}", e.message));
        assert!(info.version.starts_with("MongoDB 7"));
        assert!(c.command("admin", doc! { "ping": 1 }).await.is_ok());
    }
}
