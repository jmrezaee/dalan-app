//! D0, the driver spike (replica/databases/spec.md section 3, "The spike question"): can each engine's client run over
//! a stream Dalan hands it (an SSH `direct-tcpip` channel, or a plain TCP connection), with no local listener?
//!
//! Findings, from the crates' sources and from these tests (all tests need the Docker servers:
//! `scripts/test-servers.sh up`, then `cargo test -- --ignored l3_spike`):
//!
//! - **PostgreSQL, `tokio-postgres` 0.7.18:** yes, as it is. `Config::connect_raw(stream, tls)` takes any
//!   `AsyncRead + AsyncWrite + Unpin` stream, and TLS is negotiated inside the protocol (SSLRequest) through the
//!   `tls` argument (`tokio-postgres-rustls` with our own rustls config).
//! - **MySQL and MariaDB, `mysql_async` 0.37.1:** no. Its connection type (`io::Endpoint`) is crate-private and only
//!   knows a TCP socket, TLS over a TCP socket, or a Unix socket; `Opts` has no connector or stream hook. Path 2 of
//!   the spec works: drive the protocol ourselves on `mysql_common` 0.37.3 (packet types, capability flags,
//!   `scramble_native` / `scramble_sha256`) over our stream. Shown below: greeting, SSLRequest and TLS, native and
//!   caching_sha2 authentication, a text query.
//! - **MongoDB, `mongodb` 3.10.2:** no. Its `AsyncStream` is crate-private (TCP, TLS over TCP, Unix socket, and a
//!   SOCKS5 proxy, which would itself need a local listener); there is no connector option. Path 3 of the spec:
//!   `OP_MSG` and BSON on our stream. Shown below: `hello` over the stream. SCRAM, replica set discovery (the
//!   topology monitor, server selection), SRV, cursors and sessions would be ours to write and maintain.

use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_postgres::tls::MakeTlsConnect;
use tokio_postgres::{config::SslMode, NoTls};
use tokio_rustls::rustls::pki_types::pem::PemObject;
use tokio_rustls::rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use tokio_rustls::rustls::{self, ClientConfig, RootCertStore};
use tokio_rustls::TlsConnector;

use crate::hosts;
use crate::redis::transport::BoxStream;
use crate::ssh::tests::{harness, host_input, Harness};
use crate::ssh::{
    connect_authenticated, Connected, PromptAnswer, Prompts, SessionEvent, SessionIo,
};

// ---- the two ways to get a stream (the same two the Redis transport has)

/// Signs in to the `a` container (the bastion) as a person would, trusting its host key.
pub(crate) async fn sign_in_to_a(h: &Harness) -> Connected {
    let a = crate::testservers::server("a");
    let mut input = host_input("a", a.port.expect("a has a port on this computer"), None);
    input.id = Some(h.host_id.clone());
    input.username = a.user.clone();
    input.password = a.password.clone();
    h.state
        .store()
        .with_db_session(move |c, s| hosts::save(c, s, &input))
        .await
        .unwrap();
    let prompts = Arc::new(Prompts::default());
    let answering = Arc::clone(&prompts);
    let io = Arc::new(SessionIo {
        event: Box::new(move |e| {
            if let SessionEvent::Prompt { prompt_id, .. } = e {
                let _ = answering.answer(
                    &prompt_id,
                    PromptAnswer {
                        accept: true,
                        text: None,
                        remember: true,
                    },
                );
            }
        }),
        data: Box::new(|_| {}),
    });
    connect_authenticated(&h.state.store(), &io, &prompts, &h.host_id, None, false)
        .await
        .expect("signing in to the test bastion")
}

/// A channel from the host to `host:port`: the server sees the connection coming from the host. No local port.
async fn through_host(ssh: &Connected, host: &str, port: u16) -> BoxStream {
    let channel = ssh
        .handle
        .channel_open_direct_tcpip(host.to_string(), u32::from(port), "127.0.0.1", 0)
        .await
        .expect("the host should reach the server");
    Box::new(channel.into_stream())
}

async fn direct(port: u16) -> BoxStream {
    Box::new(
        TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("the published port"),
    )
}

fn container_file(container: &str, path: &str) -> String {
    let out = std::process::Command::new("docker")
        .args(["exec", container, "cat", path])
        .output()
        .expect("docker");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

/// Runs a command in a test container and checks it worked.
fn docker_exec(container: &str, args: &[&str]) {
    let out = std::process::Command::new("docker")
        .arg("exec")
        .arg(container)
        .args(args)
        .output()
        .expect("docker");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    // Named explicitly: with two crypto libraries in the dependency tree rustls can't pick one by itself.
    Arc::new(rustls::crypto::aws_lc_rs::default_provider())
}

/// "verify": the chain must lead to a root we hold. `pem` is a certificate to trust; `None` is this computer's own list.
fn verifying(pem: Option<&str>) -> Arc<ClientConfig> {
    let mut roots = RootCertStore::empty();
    match pem {
        Some(pem) => {
            for cert in CertificateDer::pem_slice_iter(pem.as_bytes()) {
                roots.add(cert.unwrap()).unwrap();
            }
        }
        None => {
            for cert in rustls_native_certs::load_native_certs().certs {
                let _ = roots.add(cert);
            }
        }
    }
    Arc::new(
        ClientConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    )
}

/// "require": encrypt, but accept any certificate (spec section 2: `tls` is off, require or verify). MySQL's
/// auto-generated certificates have a common name and no subject alternative name, which rustls refuses to match.
#[derive(Debug)]
struct AcceptAny;

impl rustls::client::danger::ServerCertVerifier for AcceptAny {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            m,
            c,
            d,
            &provider().signature_verification_algorithms,
        )
    }
    fn verify_tls13_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            m,
            c,
            d,
            &provider().signature_verification_algorithms,
        )
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

fn requiring() -> Arc<ClientConfig> {
    Arc::new(
        ClientConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()
            .unwrap()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AcceptAny))
            .with_no_client_auth(),
    )
}

// ---- PostgreSQL: the driver as it is

/// Connects `tokio-postgres` over `stream`. `tls`: `None` is plain, `Some` negotiates TLS inside the protocol.
/// Answers "version | ssl=true/false", where `ssl` is what the *server* says about this very connection.
async fn postgres(stream: BoxStream, tls: Option<Arc<ClientConfig>>) -> Result<String, String> {
    let mut cfg = tokio_postgres::Config::new();
    cfg.user("app").password("test-pg").dbname("shop");
    let client = match tls {
        None => {
            cfg.ssl_mode(SslMode::Disable);
            let (client, connection) = cfg
                .connect_raw(stream, NoTls)
                .await
                .map_err(|e| e.to_string())?;
            tokio::spawn(connection);
            client
        }
        Some(config) => {
            cfg.ssl_mode(SslMode::Require);
            let mut make = tokio_postgres_rustls::MakeRustlsConnect::new((*config).clone());
            let tls = MakeTlsConnect::<BoxStream>::make_tls_connect(&mut make, "pg")
                .map_err(|e| e.to_string())?;
            let (client, connection) = cfg
                .connect_raw(stream, tls)
                .await
                .map_err(|e| format!("{e:?}"))?;
            tokio::spawn(connection);
            client
        }
    };
    let row = client
        .query_one(
            "select version(), coalesce((select ssl from pg_stat_ssl where pid = pg_backend_pid()), false)",
            &[],
        )
        .await
        .map_err(|e| e.to_string())?;
    Ok(format!(
        "{} | ssl={}",
        row.get::<_, String>(0),
        row.get::<_, bool>(1)
    ))
}

#[tokio::test]
#[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
async fn l3_spike_postgres_takes_our_stream_plain_and_with_tls_through_the_host_and_direct() {
    let h = harness(None).await;
    let ssh = sign_in_to_a(&h).await;

    // Through the host, plain: the server's own words, and it says this connection is not encrypted.
    let plain = postgres(through_host(&ssh, "pg", 5432).await, None)
        .await
        .unwrap();
    assert!(
        plain.starts_with("PostgreSQL 16") && plain.ends_with("ssl=false"),
        "{plain}"
    );

    // Through the host with TLS ("verify": the container's own CA is the root we trust).
    let cert = container_file("dalan-test-pg", "/var/lib/postgresql/ca.crt");
    let secure = postgres(
        through_host(&ssh, "pg", 5432).await,
        Some(verifying(Some(&cert))),
    )
    .await
    .unwrap();
    assert!(secure.ends_with("ssl=true"), "{secure}");

    // The certificate really is checked: with only this computer's authorities the same server is refused.
    let refused = postgres(through_host(&ssh, "pg", 5432).await, Some(verifying(None)))
        .await
        .unwrap_err();
    assert!(
        (refused.contains("UnknownIssuer") || refused.to_lowercase().contains("certificate")),
        "{refused}"
    );

    // "require": encrypted without checking, for a certificate nobody can verify.
    let required = postgres(through_host(&ssh, "pg", 5432).await, Some(requiring()))
        .await
        .unwrap();
    assert!(required.ends_with("ssl=true"), "{required}");

    // Direct, from this computer to the published port.
    let direct_plain = postgres(direct(15432).await, None).await.unwrap();
    assert!(direct_plain.starts_with("PostgreSQL 16"), "{direct_plain}");
}

// ---- MySQL and MariaDB: our own protocol on `mysql_common`, over the stream

use mysql_common::constants::CapabilityFlags;
use mysql_common::io::ParseBuf;
use mysql_common::packets::{AuthPlugin, HandshakePacket, HandshakeResponse};
use mysql_common::proto::{MyDeserialize, MySerialize};
use mysql_common::scramble::{scramble_native, scramble_sha256};

/// One MySQL packet: 3 bytes of length, 1 of sequence number, then the payload.
async fn read_packet<S: AsyncReadExt + Unpin>(s: &mut S) -> (u8, Vec<u8>) {
    let mut head = [0u8; 4];
    s.read_exact(&mut head).await.unwrap();
    let len = u32::from_le_bytes([head[0], head[1], head[2], 0]) as usize;
    let mut payload = vec![0u8; len];
    s.read_exact(&mut payload).await.unwrap();
    (head[3], payload)
}

async fn write_packet<S: AsyncWriteExt + Unpin>(s: &mut S, seq: u8, payload: &[u8]) {
    let mut out = (payload.len() as u32).to_le_bytes()[..3].to_vec();
    out.push(seq);
    out.extend_from_slice(payload);
    s.write_all(&out).await.unwrap();
    s.flush().await.unwrap();
}

/// A "length-encoded" string from a text-protocol row; `None` is NULL.
fn lenenc(buf: &[u8], at: &mut usize) -> Option<String> {
    let first = buf[*at];
    *at += 1;
    if first == 0xfb {
        return None;
    }
    let len = match first {
        0xfc => {
            let n = u16::from_le_bytes([buf[*at], buf[*at + 1]]) as usize;
            *at += 2;
            n
        }
        0xfd => {
            let n = u32::from_le_bytes([buf[*at], buf[*at + 1], buf[*at + 2], 0]) as usize;
            *at += 3;
            n
        }
        n => n as usize,
    };
    let text = String::from_utf8_lossy(&buf[*at..*at + len]).into_owned();
    *at += len;
    Some(text)
}

fn err_text(payload: &[u8]) -> String {
    // 0xff, code (2), then "#" + state (5) when the client speaks protocol 4.1, then the message.
    let code = u16::from_le_bytes([payload[1], payload[2]]);
    let msg = if payload.get(3) == Some(&b'#') {
        &payload[9..]
    } else {
        &payload[3..]
    };
    format!("ERROR {code}: {}", String::from_utf8_lossy(msg))
}

/// Greeting, optional TLS, authentication, one text query. Answers "server | plugin | tls | first row".
/// `tls`: ask for TLS when the server offers it. Everything runs on the stream we were given.
async fn mysql(stream: BoxStream, user: &str, pass: &str, tls: bool) -> Result<String, String> {
    let mut s = stream;
    let (_, greeting) = read_packet(&mut s).await;
    if greeting.first() == Some(&0xff) {
        return Err(err_text(&greeting));
    }
    let hs =
        HandshakePacket::deserialize((), &mut ParseBuf(&greeting)).map_err(|e| e.to_string())?;
    let server = hs.server_version_str().to_string();
    let nonce = hs.nonce();
    let plugin =
        String::from_utf8_lossy(hs.auth_plugin_name_ref().unwrap_or_default()).into_owned();
    let offered = hs.capabilities();

    let mut caps = CapabilityFlags::CLIENT_PROTOCOL_41
        | CapabilityFlags::CLIENT_SECURE_CONNECTION
        | CapabilityFlags::CLIENT_LONG_PASSWORD
        | CapabilityFlags::CLIENT_TRANSACTIONS
        | CapabilityFlags::CLIENT_PLUGIN_AUTH
        | CapabilityFlags::CLIENT_CONNECT_WITH_DB;
    caps &= offered | CapabilityFlags::CLIENT_CONNECT_WITH_DB;
    let mut seq = 1u8;
    let mut encrypted = false;
    if tls {
        assert!(
            offered.contains(CapabilityFlags::CLIENT_SSL),
            "{server} does not offer TLS"
        );
        caps |= CapabilityFlags::CLIENT_SSL;
        // SSLRequest: the first 32 bytes of the login packet, then TLS starts on the same stream.
        let mut req = caps.bits().to_le_bytes().to_vec();
        req.extend_from_slice(&(1u32 << 24).to_le_bytes());
        req.push(45); // utf8mb4_general_ci
        req.extend_from_slice(&[0u8; 23]);
        write_packet(&mut s, seq, &req).await;
        seq += 1;
        let connector = TlsConnector::from(requiring());
        let name = ServerName::try_from("mysql").unwrap();
        let secured = connector
            .connect(name, s)
            .await
            .map_err(|e| format!("TLS: {e}"))?;
        s = Box::new(secured);
        encrypted = true;
    }

    let scramble = |plugin: &str, nonce: &[u8]| -> Vec<u8> {
        match plugin {
            "mysql_native_password" => scramble_native(nonce, pass.as_bytes()).map(|x| x.to_vec()),
            "caching_sha2_password" => scramble_sha256(nonce, pass.as_bytes()).map(|x| x.to_vec()),
            _ => None,
        }
        .unwrap_or_default()
    };
    let auth = match plugin.as_str() {
        "mysql_native_password" => AuthPlugin::MysqlNativePassword,
        "caching_sha2_password" => AuthPlugin::CachingSha2Password,
        other => {
            return Err(format!(
                "an authentication plugin this spike does not do: {other}"
            ))
        }
    };
    let response = HandshakeResponse::new(
        Some(scramble(&plugin, &nonce)),
        (8, 0, 0),
        Some(user.as_bytes().to_vec()),
        Some(b"shop".to_vec()),
        Some(auth),
        caps,
        None,
        1 << 24,
    );
    let mut login = Vec::new();
    response.serialize(&mut login);
    write_packet(&mut s, seq, &login).await;
    seq += 1;

    // The answer: OK (0x00), error (0xff), or, for caching_sha2_password, 0x01 + a status byte.
    let (_, mut reply) = read_packet(&mut s).await;
    if plugin == "caching_sha2_password" && reply.first() == Some(&0x01) {
        match reply.get(1) {
            Some(0x03) => reply = read_packet(&mut s).await.1, // fast path: the server had the password cached
            Some(0x04) if encrypted => {
                // Full authentication: over TLS the password may be sent as it is (a NUL at the end).
                let mut clear = pass.as_bytes().to_vec();
                clear.push(0);
                write_packet(&mut s, seq + 1, &clear).await;
                reply = read_packet(&mut s).await.1;
            }
            Some(0x04) => {
                return Err("caching_sha2_password wants full authentication: it needs TLS (or an RSA key exchange)".into())
            }
            other => return Err(format!("unexpected authentication status {other:?}")),
        }
    }
    match reply.first() {
        Some(0x00) => {}
        Some(0xff) => return Err(err_text(&reply)),
        other => return Err(format!("unexpected answer to the login: {other:?}")),
    }

    // COM_QUERY, text protocol: column count, the column definitions, EOF, the rows, EOF.
    let mut query = vec![0x03];
    query.extend_from_slice(b"select current_user(), @@version_comment");
    write_packet(&mut s, 0, &query).await;
    let (_, head) = read_packet(&mut s).await;
    if head.first() == Some(&0xff) {
        return Err(err_text(&head));
    }
    let columns = head[0] as usize;
    for _ in 0..columns {
        read_packet(&mut s).await;
    }
    let (_, eof) = read_packet(&mut s).await;
    assert_eq!(
        eof.first(),
        Some(&0xfe),
        "the end of the column definitions"
    );
    let (_, row) = read_packet(&mut s).await;
    let mut at = 0;
    let values: Vec<String> = (0..columns)
        .map(|_| lenenc(&row, &mut at).unwrap_or("NULL".into()))
        .collect();
    // COM_QUIT, so the server doesn't log an aborted connection.
    write_packet(&mut s, 0, &[0x01]).await;
    Ok(format!(
        "{server} | {plugin} | tls={encrypted} | {}",
        values.join(" / ")
    ))
}

#[tokio::test]
#[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
async fn l3_spike_mysql_and_mariadb_run_on_our_stream_with_mysql_common() {
    let h = harness(None).await;
    let ssh = sign_in_to_a(&h).await;

    // MariaDB through the host: mysql_native_password, no TLS.
    let maria = mysql(
        through_host(&ssh, "mariadb", 3306).await,
        "app",
        "test-mariadb",
        false,
    )
    .await
    .unwrap();
    assert!(
        maria.contains("MariaDB")
            && maria.contains("mysql_native_password")
            && maria.contains("tls=false"),
        "{maria}"
    );

    // MySQL 8.4 through the host: caching_sha2_password. The server caches a successful full authentication, so the
    // test empties that cache first (FLUSH PRIVILEGES) and the three logins below are deterministic:
    //   1. cold, no TLS: the server asks for full authentication, which it only accepts over TLS (or an RSA exchange),
    //      so the spike reports that instead of sending the password in clear;
    //   2. with TLS (SSLRequest, then TLS on the same channel): full authentication works;
    //   3. no TLS again: the server now has it cached, and the "fast path" works.
    docker_exec(
        "dalan-test-mysql",
        &["mysql", "-uroot", "-ptest-mysql", "-e", "FLUSH PRIVILEGES"],
    );
    let cold = mysql(
        through_host(&ssh, "mysql", 3306).await,
        "app",
        "test-mysql",
        false,
    )
    .await
    .unwrap_err();
    assert!(cold.contains("full authentication"), "{cold}");
    let secure = mysql(
        through_host(&ssh, "mysql", 3306).await,
        "app",
        "test-mysql",
        true,
    )
    .await
    .unwrap();
    assert!(
        secure.contains("caching_sha2_password") && secure.contains("tls=true"),
        "{secure}"
    );
    let warm = mysql(
        through_host(&ssh, "mysql", 3306).await,
        "app",
        "test-mysql",
        false,
    )
    .await
    .unwrap();
    assert!(
        warm.contains("caching_sha2_password") && warm.contains("tls=false"),
        "{warm}"
    );

    // MariaDB over TLS, and MySQL direct from this computer.
    let maria_tls = mysql(
        through_host(&ssh, "mariadb", 3306).await,
        "app",
        "test-mariadb",
        true,
    )
    .await
    .unwrap();
    assert!(
        maria_tls.contains("MariaDB") && maria_tls.contains("tls=true"),
        "{maria_tls}"
    );
    let direct_mysql = mysql(direct(13306).await, "app", "test-mysql", true)
        .await
        .unwrap();
    assert!(direct_mysql.contains("tls=true"), "{direct_mysql}");
    // A wrong password is the server's own error.
    let wrong = mysql(
        through_host(&ssh, "mariadb", 3306).await,
        "app",
        "nope",
        false,
    )
    .await
    .unwrap_err();
    assert!(wrong.contains("Access denied"), "{wrong}");
}

// ---- MongoDB: OP_MSG and BSON on our stream

/// One command as an `OP_MSG` (opcode 2013), and the reply document.
async fn mongo_command(
    s: &mut BoxStream,
    request_id: i32,
    command: bson::Document,
) -> bson::Document {
    let mut body = Vec::new();
    command.to_writer(&mut body).unwrap();
    let mut msg = Vec::with_capacity(21 + body.len());
    msg.extend_from_slice(&0i32.to_le_bytes()); // total length, set below
    msg.extend_from_slice(&request_id.to_le_bytes());
    msg.extend_from_slice(&0i32.to_le_bytes()); // responseTo
    msg.extend_from_slice(&2013i32.to_le_bytes()); // OP_MSG
    msg.extend_from_slice(&0u32.to_le_bytes()); // flagBits
    msg.push(0); // section kind 0: one document
    msg.extend_from_slice(&body);
    let len = msg.len() as i32;
    msg[..4].copy_from_slice(&len.to_le_bytes());
    s.write_all(&msg).await.unwrap();
    s.flush().await.unwrap();

    let mut len_bytes = [0u8; 4];
    s.read_exact(&mut len_bytes).await.unwrap();
    let total = i32::from_le_bytes(len_bytes) as usize;
    let mut rest = vec![0u8; total - 4];
    s.read_exact(&mut rest).await.unwrap();
    // After the length: requestID, responseTo, opCode (12 bytes), flagBits (4), the section kind (1), the document.
    assert_eq!(
        i32::from_le_bytes(rest[8..12].try_into().unwrap()),
        2013,
        "the server answers in OP_MSG"
    );
    bson::Document::from_reader(&mut &rest[17..]).unwrap()
}

#[tokio::test]
#[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
async fn l3_spike_mongodb_speaks_op_msg_on_our_stream() {
    let h = harness(None).await;
    let ssh = sign_in_to_a(&h).await;
    let ca = container_file("dalan-test-mongo", "/data/tls/ca.crt");
    // MongoDB's TLS starts as soon as the connection opens, so it is the stream wrapped in rustls, then the same
    // OP_MSG. "verify" trusts the container's CA; "require" accepts any certificate.
    let tls = |stream: BoxStream, config: Arc<ClientConfig>| async move {
        let secured = TlsConnector::from(config)
            .connect(ServerName::try_from("mongo").unwrap(), stream)
            .await
            .map_err(|e| e.to_string())?;
        Ok::<BoxStream, String>(Box::new(secured))
    };
    let cases: Vec<(&str, BoxStream)> = vec![
        ("through the host", through_host(&ssh, "mongo", 27017).await),
        ("direct", direct(17017).await),
        (
            "through the host, TLS verify",
            tls(
                through_host(&ssh, "mongo", 27017).await,
                verifying(Some(&ca)),
            )
            .await
            .unwrap(),
        ),
        (
            "through the host, TLS require",
            tls(through_host(&ssh, "mongo", 27017).await, requiring())
                .await
                .unwrap(),
        ),
        (
            "direct, TLS verify",
            tls(direct(17017).await, verifying(Some(&ca)))
                .await
                .unwrap(),
        ),
    ];
    // The certificate is really checked: this computer's own authorities do not know the container's CA.
    let refused = tls(through_host(&ssh, "mongo", 27017).await, verifying(None))
        .await
        .err()
        .expect("refused");
    assert!(
        (refused.contains("UnknownIssuer") || refused.to_lowercase().contains("certificate")),
        "{refused}"
    );
    for (route, mut stream) in cases {
        let hello = mongo_command(
            &mut stream,
            1,
            bson::doc! { "hello": 1, "$db": "admin", "saslSupportedMechs": "admin.app" },
        )
        .await;
        assert_eq!(
            hello.get_f64("ok").unwrap_or(0.0),
            1.0,
            "{route}: {hello:?}"
        );
        assert!(
            hello.get_bool("isWritablePrimary").unwrap_or(false),
            "{route}: {hello:?}"
        );
        let wire = hello.get_i32("maxWireVersion").unwrap();
        assert!(
            wire >= 17,
            "{route}: MongoDB 7 speaks wire version 21, this said {wire}"
        );
        // The server says how to sign in: SCRAM is what a client has to implement.
        let mechs: Vec<&str> = hello
            .get_array("saslSupportedMechs")
            .unwrap()
            .iter()
            .filter_map(|m| m.as_str())
            .collect();
        assert!(mechs.contains(&"SCRAM-SHA-256"), "{route}: {mechs:?}");
        // Nothing but `hello` is allowed before signing in: the server's own error, as a document.
        let denied = mongo_command(
            &mut stream,
            2,
            bson::doc! { "listDatabases": 1, "$db": "admin" },
        )
        .await;
        assert_eq!(
            denied.get_f64("ok").unwrap_or(1.0),
            0.0,
            "{route}: {denied:?}"
        );
        assert_eq!(
            denied.get_i32("code").unwrap_or(0),
            13,
            "Unauthorized: {denied:?}"
        );
    }
}
