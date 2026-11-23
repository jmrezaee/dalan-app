//! MySQL and MariaDB (D7): our own protocol client on `mysql_common`'s packet types, over the stream Dalan hands it
//! (direct or an SSH `direct-tcpip` channel): `mysql_async` can't take a stream (D0). This file is the connection:
//! greeting, TLS on the same stream, authentication (`mysql_native_password`, `caching_sha2_password`), the text
//! protocol for queries (every value arrives as the server's own text, so no digit is lost), cancel from a second
//! connection (`KILL QUERY`), and the server's own error text.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use mysql_common::constants::CapabilityFlags;
use mysql_common::io::ParseBuf;
use mysql_common::packets::{AuthPlugin, HandshakePacket, HandshakeResponse};
use mysql_common::proto::{MyDeserialize, MySerialize};
use mysql_common::scramble::{scramble_native, scramble_sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_rustls::rustls;
use zeroize::Zeroizing;

use super::pg::{Reopen, SqlServerInfo};
use super::values::CellKind;
use crate::dbs::transport::{tls_client_config, tls_error, BoxStream, TlsMode, CONNECT_TIMEOUT};
use crate::dbs::Engine;
use crate::error::{AppError, AppResult, ErrorCode};

const MAX_PACKET: usize = 0x00ff_ffff;
/// Rows kept in memory for one result when the caller says no limit (a ceiling, not a feature).
const HARD_ROW_CAP: usize = 1_000_000;
/// One packet assembled from continuations, and one whole result, are never larger than this.
const MAX_ASSEMBLED: usize = 64 * 1024 * 1024;
const MAX_RESULT_BYTES: usize = 256 * 1024 * 1024;

#[derive(Clone)]
pub struct MyLogin {
    pub user: String,
    pub password: Option<Zeroizing<String>>,
    pub database: Option<String>,
    pub tls: TlsMode,
    pub server_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavour {
    MySql,
    MariaDb,
}

#[derive(Debug, Clone)]
pub struct MyColumn {
    pub name: String,
    pub type_name: String,
    pub kind: CellKind,
}

#[derive(Debug, Clone, Default)]
pub struct MyResult {
    pub columns: Vec<MyColumn>,
    pub rows: Vec<Vec<Option<String>>>,
    pub affected: u64,
    /// The row limit cut the result off (the rest was read and dropped).
    pub more: bool,
}

pub struct MyConn {
    stream: tokio::sync::Mutex<BoxStream>,
    pub connection_id: u32,
    pub flavour: Flavour,
    pub version: String,
    reopen: Reopen,
    login: MyLogin,
    closed: AtomicBool,
}

fn closed() -> AppError {
    AppError::new(ErrorCode::DbUnreachable, "The connection to the database closed before it answered. Is this really a MySQL or MariaDB port?")
}

async fn read_packet(s: &mut BoxStream) -> AppResult<(u8, Vec<u8>)> {
    let mut out = Vec::new();
    let mut seq;
    loop {
        let mut head = [0u8; 4];
        s.read_exact(&mut head).await.map_err(|_| closed())?;
        let len = u32::from_le_bytes([head[0], head[1], head[2], 0]) as usize;
        seq = head[3];
        let start = out.len();
        if start + len > MAX_ASSEMBLED {
            return Err(AppError::new(ErrorCode::DbRefused, "The server sent a packet larger than Dalan will read (64 MB). Select fewer or smaller columns."));
        }
        out.resize(start + len, 0);
        s.read_exact(&mut out[start..]).await.map_err(|_| closed())?;
        // A payload of exactly 16 MB − 1 continues in the next packet.
        if len < MAX_PACKET {
            return Ok((seq, out));
        }
    }
}

async fn write_packet(s: &mut BoxStream, mut seq: u8, payload: &[u8]) -> AppResult<()> {
    let mut rest = payload;
    loop {
        let n = rest.len().min(MAX_PACKET);
        let mut out = (n as u32).to_le_bytes()[..3].to_vec();
        out.push(seq);
        out.extend_from_slice(&rest[..n]);
        s.write_all(&out).await.map_err(|_| closed())?;
        rest = &rest[n..];
        seq = seq.wrapping_add(1);
        if n < MAX_PACKET {
            break;
        }
    }
    s.flush().await.map_err(|_| closed())
}

/// A cursor over a packet that never panics: every read says whether it had the bytes.
struct Cur<'a>(&'a [u8], usize);
impl<'a> Cur<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.0.get(self.1..self.1.checked_add(n)?)?;
        self.1 += n;
        Some(s)
    }
    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|b| b[0])
    }
    fn u16(&mut self) -> Option<u16> {
        self.take(2).map(|b| u16::from_le_bytes([b[0], b[1]]))
    }
    /// A length-encoded integer; `None` for NULL (0xfb) or a truncated packet.
    fn lenenc_int(&mut self) -> Option<Option<u64>> {
        Some(match self.u8()? {
            0xfb => None,
            0xfc => Some(u64::from(self.u16()?)),
            0xfd => {
                let b = self.take(3)?;
                Some(u64::from(u32::from_le_bytes([b[0], b[1], b[2], 0])))
            }
            0xfe => {
                let b = self.take(8)?;
                Some(u64::from_le_bytes(b.try_into().ok()?))
            }
            n => Some(u64::from(n)),
        })
    }
    fn lenenc_bytes(&mut self) -> Option<Option<&'a [u8]>> {
        match self.lenenc_int()? {
            None => Some(None),
            Some(n) => Some(Some(self.take(usize::try_from(n).ok()?)?)),
        }
    }
    fn rest(&mut self) -> &'a [u8] {
        let s = self.0.get(self.1..).unwrap_or_default();
        self.1 = self.0.len();
        s
    }
}

/// The server's error packet as ours: the code's meaning where it matters, its own words always.
pub fn server_error(payload: &[u8]) -> AppError {
    let mut c = Cur(payload, 1);
    let code = c.u16().unwrap_or(0);
    let rest = c.rest();
    let msg = if rest.first() == Some(&b'#') { rest.get(6..).unwrap_or_default() } else { rest };
    let text = String::from_utf8_lossy(msg).into_owned();
    match code {
        // ER_ACCESS_DENIED_ERROR, ER_DBACCESS_DENIED_ERROR, ER_ACCESS_DENIED_NO_PASSWORD_ERROR
        1045 | 1698 => AppError::new(ErrorCode::DbAuthFailed, format!("Wrong username or password. The server says: {text}")),
        _ => AppError::new(ErrorCode::DbRefused, text),
    }
}

/// Column type code to what the window shows. `charset` 63 is "binary".
pub fn kind_of(code: u8, flags: u16, charset: u16) -> (CellKind, &'static str) {
    match code {
        0 | 246 => (CellKind::Decimal, "decimal"),
        1 => (CellKind::Int, "tinyint"),
        2 => (CellKind::Int, "smallint"),
        3 => (CellKind::Int, "int"),
        8 => (CellKind::Int, "bigint"),
        9 => (CellKind::Int, "mediumint"),
        13 => (CellKind::Int, "year"),
        16 => (CellKind::Bytes, "bit"),
        4 => (CellKind::Float, "float"),
        5 => (CellKind::Float, "double"),
        7 => (CellKind::Datetime, "timestamp"),
        10 | 14 => (CellKind::Datetime, "date"),
        12 => (CellKind::Datetime, "datetime"),
        11 => (CellKind::Interval, "time"),
        245 => (CellKind::Json, "json"),
        247 => (CellKind::Enum, "enum"),
        248 => (CellKind::Enum, "set"),
        249..=252 => if charset == 63 { (CellKind::Bytes, "blob") } else { (CellKind::Text, "text") },
        253 | 254 | 15 => {
            if flags & 256 != 0 {
                (CellKind::Enum, "enum")
            } else if flags & 2048 != 0 {
                (CellKind::Enum, "set")
            } else if charset == 63 {
                (CellKind::Bytes, "binary")
            } else {
                (CellKind::Text, "varchar")
            }
        }
        255 => (CellKind::Bytes, "geometry"),
        _ => (CellKind::Unknown, "unknown"),
    }
}

/// A value's text. Binary columns show as `\x` and hex, like PostgreSQL's bytea; everything else is UTF-8 (lossy).
fn text(kind: CellKind, b: &[u8]) -> String {
    if kind == CellKind::Bytes {
        let mut s = String::from("\\x");
        for x in b {
            s.push_str(&format!("{x:02x}"));
        }
        s
    } else {
        String::from_utf8_lossy(b).into_owned()
    }
}

fn scramble(plugin: &str, nonce: &[u8], pass: &str) -> AppResult<Vec<u8>> {
    if pass.is_empty() {
        return Ok(Vec::new());
    }
    match plugin {
        "mysql_native_password" => Ok(scramble_native(nonce, pass.as_bytes()).map(|x| x.to_vec()).unwrap_or_default()),
        "caching_sha2_password" => Ok(scramble_sha256(nonce, pass.as_bytes()).map(|x| x.to_vec()).unwrap_or_default()),
        other => Err(AppError::new(ErrorCode::DbRefused, format!("This server wants the {other} sign-in, which Dalan doesn't speak yet. Use a user with mysql_native_password or caching_sha2_password."))),
    }
}

impl MyConn {
    /// Greeting, optional TLS, sign-in. `reopen` is the way back to the server (for a cancel).
    pub async fn connect(stream: BoxStream, login: MyLogin, reopen: Reopen) -> AppResult<(MyConn, SqlServerInfo)> {
        let started = Instant::now();
        let mut s = stream;
        let (_, greeting) = tokio::time::timeout(CONNECT_TIMEOUT, read_packet(&mut s)).await.map_err(|_| closed())??;
        if greeting.first() == Some(&0xff) {
            return Err(server_error(&greeting));
        }
        let hs = HandshakePacket::deserialize((), &mut ParseBuf(&greeting)).map_err(|_| closed())?;
        let version = hs.server_version_str().to_string();
        let flavour = if version.to_ascii_lowercase().contains("mariadb") { Flavour::MariaDb } else { Flavour::MySql };
        let offered = hs.capabilities();
        let connection_id = hs.connection_id();
        let nonce = hs.nonce();
        let mut plugin = String::from_utf8_lossy(hs.auth_plugin_name_ref().unwrap_or_default()).into_owned();
        let mut caps = CapabilityFlags::CLIENT_PROTOCOL_41
            | CapabilityFlags::CLIENT_SECURE_CONNECTION
            | CapabilityFlags::CLIENT_LONG_PASSWORD
            | CapabilityFlags::CLIENT_TRANSACTIONS
            | CapabilityFlags::CLIENT_PLUGIN_AUTH;
        caps &= offered;
        if login.database.as_deref().is_some_and(|d| !d.is_empty()) {
            caps |= CapabilityFlags::CLIENT_CONNECT_WITH_DB;
        }
        let mut seq = 1u8;
        let mut encrypted = false;
        if login.tls != TlsMode::Off {
            if !offered.contains(CapabilityFlags::CLIENT_SSL) {
                return Err(AppError::new(ErrorCode::DbRefused, "This server doesn't offer TLS. Turn TLS off for this database, or ask for it to be enabled."));
            }
            caps |= CapabilityFlags::CLIENT_SSL;
            // SSLRequest: the first 32 bytes of the login packet, then TLS starts on the same stream.
            let mut req = caps.bits().to_le_bytes().to_vec();
            req.extend_from_slice(&(1u32 << 24).to_le_bytes());
            req.push(45); // utf8mb4_general_ci
            req.extend_from_slice(&[0u8; 23]);
            write_packet(&mut s, seq, &req).await?;
            seq += 1;
            let name = rustls::pki_types::ServerName::try_from(login.server_name.clone()).map_err(|_| AppError::validation(format!("{} can't be used as a TLS server name.", login.server_name)))?;
            let connector = tokio_rustls::TlsConnector::from(tls_client_config(login.tls)?);
            let tls = tokio::time::timeout(CONNECT_TIMEOUT, connector.connect(name, s))
                .await
                .map_err(|_| AppError::new(ErrorCode::DbUnreachable, "The TLS handshake took too long. Is this port really TLS?"))?
                .map_err(|e| tls_error(Engine::Mysql, &e, &login.server_name))?;
            s = Box::new(tls);
            encrypted = true;
        }
        let pass = Zeroizing::new(login.password.as_ref().map_or("", |p| p.as_str()).to_string());
        let auth = match plugin.as_str() {
            "mysql_native_password" => AuthPlugin::MysqlNativePassword,
            "caching_sha2_password" => AuthPlugin::CachingSha2Password,
            _ => AuthPlugin::MysqlNativePassword,
        };
        let first = if matches!(plugin.as_str(), "mysql_native_password" | "caching_sha2_password") { scramble(&plugin, &nonce, &pass)? } else { Vec::new() };
        let response = HandshakeResponse::new(
            Some(first),
            (8, 0, 0),
            Some(login.user.as_bytes().to_vec()),
            login.database.clone().filter(|d| !d.is_empty()).map(String::into_bytes),
            Some(auth),
            caps,
            None,
            1 << 24,
        );
        let mut out = Vec::new();
        response.serialize(&mut out);
        write_packet(&mut s, seq, &out).await?;
        // The answer: OK, error, an auth switch (0xfe), or caching_sha2's status (0x01 + byte).
        loop {
            let (rseq, reply) = tokio::time::timeout(CONNECT_TIMEOUT, read_packet(&mut s)).await.map_err(|_| closed())??;
            seq = rseq.wrapping_add(1);
            match reply.first() {
                Some(0x00) => break,
                Some(0xff) => return Err(server_error(&reply)),
                Some(0xfe) => {
                    let mut c = Cur(&reply, 1);
                    let rest = c.rest();
                    let nul = rest.iter().position(|b| *b == 0).unwrap_or(rest.len());
                    plugin = String::from_utf8_lossy(&rest[..nul]).into_owned();
                    let nonce_now: Vec<u8> = rest.get(nul + 1..).unwrap_or_default().iter().copied().take_while(|b| *b != 0).collect();
                    write_packet(&mut s, seq, &scramble(&plugin, &nonce_now, &pass)?).await?;
                }
                Some(0x01) if plugin == "caching_sha2_password" => match reply.get(1) {
                    // Fast path: the server had the password cached.
                    Some(0x03) => {}
                    // Full authentication: over TLS the password may be sent as it is (a NUL at the end).
                    Some(0x04) if encrypted => {
                        let mut clear = Zeroizing::new(pass.as_bytes().to_vec());
                        clear.push(0);
                        write_packet(&mut s, seq, &clear).await?;
                    }
                    Some(0x04) => return Err(AppError::new(ErrorCode::DbRefused, "This server wants full authentication for this user, which needs TLS. Turn TLS on (Require or Verify) for this database.")),
                    _ => return Err(closed()),
                },
                _ => return Err(closed()),
            }
        }
        let conn = MyConn { stream: tokio::sync::Mutex::new(s), connection_id, flavour, version: version.clone(), reopen, login: login.clone(), closed: AtomicBool::new(false) };
        let row = tokio::time::timeout(CONNECT_TIMEOUT, async {
            conn.query("SET NAMES utf8mb4", 0).await?;
            conn.query("select database(), current_user()", 1).await
        })
        .await
        .map_err(|_| closed())??;
        let get = |i: usize| row.rows.first().and_then(|r| r.get(i)).cloned().flatten().unwrap_or_default();
        let info = SqlServerInfo {
            engine: if flavour == Flavour::MariaDb { "mariadb" } else { "mysql" }.into(),
            version: format!("{} {version}", if flavour == Flavour::MariaDb { "MariaDB" } else { "MySQL" }),
            database: get(0),
            user: get(1),
            latency_ms: started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32,
        };
        Ok((conn, info))
    }

    /// One statement in the text protocol. `limit` 0 is no limit (up to a ceiling); past a limit the rest is read and
    /// dropped (the server sends it all), and `more` says so. A connection that fails mid-way is closed for good.
    pub async fn query(&self, sql: &str, limit: usize) -> AppResult<MyResult> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(closed());
        }
        let mut s = self.stream.lock().await;
        // If this future is dropped half way, the stream is mid-result: the flag stays set and the connection is dead.
        self.closed.store(true, Ordering::SeqCst);
        let out = Self::exchange(&mut s, sql, if limit == 0 { HARD_ROW_CAP } else { limit }).await;
        if !matches!(&out, Err(e) if e.code == ErrorCode::DbUnreachable) {
            self.closed.store(false, Ordering::SeqCst);
        }
        out
    }

    async fn exchange(s: &mut BoxStream, sql: &str, limit: usize) -> AppResult<MyResult> {
        let mut q = vec![0x03];
        q.extend_from_slice(sql.as_bytes());
        write_packet(s, 0, &q).await?;
        let (_, head) = read_packet(s).await?;
        match head.first() {
            Some(0xff) => return Err(server_error(&head)),
            Some(0x00) => {
                let mut c = Cur(&head, 1);
                let affected = c.lenenc_int().flatten().unwrap_or(0);
                return Ok(MyResult { affected, ..MyResult::default() });
            }
            Some(0xfb) => {
                // An empty packet says "no file"; the server then answers with its final OK or error.
                write_packet(s, 2, &[]).await?;
                let _ = read_packet(s).await?;
                return Err(AppError::new(ErrorCode::DbRefused, "The server asked for a local file (LOAD DATA LOCAL). Dalan never sends local files."));
            }
            None => return Err(closed()),
            _ => {}
        }
        let n = Cur(&head, 0).lenenc_int().flatten().ok_or_else(closed)? as usize;
        let mut columns = Vec::with_capacity(n.min(4096));
        for _ in 0..n {
            let (_, def) = read_packet(s).await?;
            let mut c = Cur(&def, 0);
            for _ in 0..4 {
                c.lenenc_bytes().ok_or_else(closed)?;
            }
            let name = String::from_utf8_lossy(c.lenenc_bytes().flatten().ok_or_else(closed)?).into_owned();
            c.lenenc_bytes().ok_or_else(closed)?;
            c.u8().ok_or_else(closed)?; // 0x0c
            let charset = c.u16().ok_or_else(closed)?;
            c.take(4).ok_or_else(closed)?;
            let code = c.u8().ok_or_else(closed)?;
            let flags = c.u16().ok_or_else(closed)?;
            let (kind, type_name) = kind_of(code, flags, charset);
            columns.push(MyColumn { name, type_name: type_name.into(), kind });
        }
        let (_, eof) = read_packet(s).await?;
        if eof.first() != Some(&0xfe) {
            return Err(closed());
        }
        let (mut rows, mut more, mut total) = (Vec::new(), false, 0usize);
        loop {
            let (_, row) = read_packet(s).await?;
            total += row.len();
            if total > MAX_RESULT_BYTES {
                return Err(AppError::new(ErrorCode::DbUnreachable, "The result is larger than Dalan will read (256 MB), so the connection was closed. Select fewer rows or columns."));
            }
            match row.first() {
                Some(0xfe) if row.len() < 9 => break,
                Some(0xff) => return Err(server_error(&row)),
                None => return Err(closed()),
                _ => {}
            }
            if rows.len() >= limit {
                more = true;
                continue;
            }
            let mut c = Cur(&row, 0);
            rows.push(columns.iter().map(|col| c.lenenc_bytes().map(|v| v.map(|b| text(col.kind, b)))).collect::<Option<Vec<_>>>().ok_or_else(closed)?);
        }
        Ok(MyResult { columns, rows, affected: 0, more })
    }

    /// Stops what this connection is running, from a second connection through the same route.
    pub async fn cancel_running(&self) -> AppResult<()> {
        let stream = (self.reopen)().await?;
        let (other, _) = MyConn::connect(stream, self.login.clone(), std::sync::Arc::clone(&self.reopen)).await?;
        other.query(&format!("KILL QUERY {}", self.connection_id), 0).await?;
        other.close().await;
        Ok(())
    }

    /// A query's time limit: MySQL counts milliseconds (selects only), MariaDB seconds.
    pub async fn set_timeout(&self, ms: u32) -> AppResult<()> {
        let sql = match self.flavour {
            Flavour::MySql => format!("SET SESSION max_execution_time = {ms}"),
            Flavour::MariaDb => format!("SET SESSION max_statement_time = {}", f64::from(ms) / 1000.0),
        };
        self.query(&sql, 0).await.map(|_| ())
    }

    pub async fn close(&self) {
        if self.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        // A connection stuck in a query can't be asked to quit politely: don't wait for it.
        if let Ok(mut s) = tokio::time::timeout(std::time::Duration::from_secs(1), self.stream.lock()).await {
            let _ = tokio::time::timeout(std::time::Duration::from_secs(1), write_packet(&mut s, 0, &[0x01])).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cursor_never_panics_on_short_or_odd_packets() {
        let mut c = Cur(&[0xfc, 0x01], 0);
        assert_eq!(c.lenenc_int(), None);
        let mut c = Cur(&[0xfb, 3, b'a', b'b', b'c', 9, b'x'], 0);
        assert_eq!(c.lenenc_bytes(), Some(None));
        assert_eq!(c.lenenc_bytes(), Some(Some(&b"abc"[..])));
        assert_eq!(c.lenenc_bytes(), None, "9 bytes announced, 1 there");
        assert_eq!(Cur(&[0xfd, 1, 0, 1], 0).lenenc_int(), Some(Some(65537)));
        assert_eq!(Cur(&[], 0).u8(), None);
    }

    #[test]
    fn errors_keep_the_servers_words_and_name_a_bad_sign_in() {
        let mut p = vec![0xff, 0x15, 0x04];
        p.extend_from_slice(b"#28000Access denied for user 'app'@'h'");
        let e = server_error(&p);
        assert_eq!(e.code, ErrorCode::DbAuthFailed);
        assert!(e.message.contains("Access denied for user 'app'@'h'"));
        let mut q = vec![0xff, 0x7a, 0x04];
        q.extend_from_slice(b"#42S02Table 'shop.nope' doesn't exist");
        let e = server_error(&q);
        assert_eq!((e.code, e.message.as_str()), (ErrorCode::DbRefused, "Table 'shop.nope' doesn't exist"));
        assert_eq!(server_error(&[0xff]).code, ErrorCode::DbRefused);
        // 1044 is a database permission, not a wrong password.
        let mut d = vec![0xff, 0x14, 0x04];
        d.extend_from_slice(b"#42000Access denied for user 'app'@'h' to database 'x'");
        assert_eq!(server_error(&d).code, ErrorCode::DbRefused);
    }

    #[test]
    fn types_map_to_kinds_and_binary_text_is_hex() {
        assert_eq!(kind_of(8, 0, 63).0, CellKind::Int);
        assert_eq!(kind_of(246, 0, 63).0, CellKind::Decimal);
        assert_eq!(kind_of(252, 0, 63).0, CellKind::Bytes);
        assert_eq!((kind_of(16, 0, 63).0, kind_of(255, 0, 63).0), (CellKind::Bytes, CellKind::Bytes));
        assert_eq!(kind_of(252, 0, 45).0, CellKind::Text);
        assert_eq!(kind_of(254, 256, 45).0, CellKind::Enum);
        assert_eq!(kind_of(245, 0, 45).0, CellKind::Json);
        assert_eq!(text(CellKind::Bytes, &[0xde, 0xad]), "\\xdead");
        assert_eq!(text(CellKind::Text, "né".as_bytes()), "né");
    }
}

#[cfg(test)]
mod wire_tests {
    use super::*;
    use tokio::io::duplex;

    /// A server stub: sends `packets` (payload, sequence) and keeps the other end open.
    async fn stub(packets: Vec<Vec<u8>>) -> (BoxStream, tokio::task::JoinHandle<Vec<u8>>) {
        let (client, mut server) = duplex(1 << 20);
        let h = tokio::spawn(async move {
            // Read the client's query first, then answer.
            let mut head = [0u8; 4];
            let _ = server.read_exact(&mut head).await;
            let len = u32::from_le_bytes([head[0], head[1], head[2], 0]) as usize;
            let mut q = vec![0u8; len];
            let _ = server.read_exact(&mut q).await;
            for (i, p) in packets.iter().enumerate() {
                let mut out = (p.len() as u32).to_le_bytes()[..3].to_vec();
                out.push(i as u8 + 1);
                out.extend_from_slice(p);
                let _ = server.write_all(&out).await;
            }
            // Whatever the client sends next (an empty packet answering LOCAL INFILE).
            let mut rest = vec![0u8; 4];
            let _ = tokio::time::timeout(std::time::Duration::from_millis(300), server.read_exact(&mut rest)).await;
            rest
        });
        (Box::new(client), h)
    }

    fn coldef(name: &str, code: u8) -> Vec<u8> {
        let mut p = Vec::new();
        for s in ["def", "db", "t", "t"] {
            p.push(s.len() as u8);
            p.extend_from_slice(s.as_bytes());
        }
        for _ in 0..2 {
            p.push(name.len() as u8);
            p.extend_from_slice(name.as_bytes());
        }
        p.push(0x0c);
        p.extend_from_slice(&[45, 0, 10, 0, 0, 0, code, 0, 0, 0, 0, 0]);
        p
    }

    #[tokio::test]
    async fn a_result_set_with_a_long_row_that_starts_with_0xfe_is_a_row_not_the_end() {
        let big = vec![b'x'; 300];
        let mut row = vec![0xfe];
        row.extend_from_slice(&(big.len() as u64).to_le_bytes());
        row.extend_from_slice(&big);
        assert!(row.len() >= 9);
        let (mut s, h) = stub(vec![vec![1], coldef("a", 253), vec![0xfe, 0, 0, 2, 0], row, vec![3, b'a', b'b', b'c'], vec![0xfe, 0, 0, 2, 0]]).await;
        let r = MyConn::exchange(&mut s, "select a", 100).await.unwrap();
        assert_eq!(r.rows.len(), 2);
        assert_eq!(r.rows[0][0].as_deref().map(str::len), Some(300));
        assert_eq!(r.rows[1][0].as_deref(), Some("abc"));
        h.abort();
    }

    #[tokio::test]
    async fn truncated_definitions_and_rows_are_errors_not_panics() {
        let (mut s, h) = stub(vec![vec![1], vec![3, b'd', b'e', b'f']]).await;
        assert!(MyConn::exchange(&mut s, "x", 10).await.is_err());
        h.abort();
        let (mut s, h) = stub(vec![vec![1], coldef("a", 253), vec![0xfe, 0, 0, 2, 0], vec![5, b'a']]).await;
        assert!(MyConn::exchange(&mut s, "x", 10).await.is_err());
        h.abort();
    }

    #[tokio::test]
    async fn a_request_for_a_local_file_is_answered_with_an_empty_packet_and_refused() {
        let (mut s, h) = stub(vec![vec![0xfb, b'/', b'e', b't', b'c'], vec![0xff, 0x14, 0x04, b'n', b'o']]).await;
        let e = MyConn::exchange(&mut s, "load data local infile '/etc/passwd' into table t", 10).await.unwrap_err();
        assert_eq!(e.code, ErrorCode::DbRefused);
        assert!(e.message.contains("local file"));
        let sent = h.await.unwrap();
        // 3 bytes of length 0, then sequence 2: no file content was sent.
        assert_eq!(&sent[..3], &[0, 0, 0]);
        assert_eq!(sent[3], 2);
    }

    #[tokio::test]
    async fn a_dropped_query_marks_the_connection_dead_and_ok_packets_report_rows() {
        let (client, mut server) = duplex(1 << 16);
        let conn = MyConn { stream: tokio::sync::Mutex::new(Box::new(client)), connection_id: 1, flavour: Flavour::MySql, version: String::new(), reopen: std::sync::Arc::new(|| Box::pin(async { Err(closed()) })), login: MyLogin { user: String::new(), password: None, database: None, tls: TlsMode::Off, server_name: String::new() }, closed: AtomicBool::new(false) };
        // The server never answers: the caller gives up.
        let r = tokio::time::timeout(std::time::Duration::from_millis(100), conn.query("select sleep(9)", 0)).await;
        assert!(r.is_err());
        assert_eq!(conn.query("select 1", 0).await.unwrap_err().code, ErrorCode::DbUnreachable, "the stream is mid-query: unusable");
        drop(server.write_all(&[]).await);
        // An OK packet carries the affected rows.
        let (mut s, h) = stub(vec![vec![0x00, 7, 0, 2, 0, 0, 0]]).await;
        assert_eq!(MyConn::exchange(&mut s, "update t set a = 1", 10).await.unwrap().affected, 7);
        h.abort();
    }
}

#[cfg(test)]
mod docker_tests {
    use super::*;
    use crate::dbs::transport::{self, Target};

    fn reopen_to(port: u16) -> Reopen {
        std::sync::Arc::new(move || {
            Box::pin(async move {
                let t = Target { address: "127.0.0.1".into(), port, tls: false, tls_server_name: None };
                transport::direct_for(Engine::Mysql, &t).await
            })
        })
    }

    async fn connect(port: u16, pass: &str, tls: TlsMode) -> AppResult<(MyConn, SqlServerInfo)> {
        let stream = (reopen_to(port))().await?;
        let login = MyLogin { user: "app".into(), password: Some(Zeroizing::new(pass.into())), database: Some("shop".into()), tls, server_name: "127.0.0.1".into() };
        MyConn::connect(stream, login, reopen_to(port)).await
    }

    const SERVERS: [(u16, &str, &str, Flavour); 2] = [(13306, "test-mysql", "MySQL", Flavour::MySql), (13307, "test-mariadb", "MariaDB", Flavour::MariaDb)];

    #[tokio::test]
    #[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
    async fn l3_my_both_servers_sign_in_plain_and_with_tls_and_report_what_they_are() {
        for (port, pw, label, flavour) in SERVERS {
            for tls in [TlsMode::Off, TlsMode::Require] {
                let (c, info) = connect(port, pw, tls).await.unwrap_or_else(|e| panic!("{label} {tls:?}: {}", e.message));
                assert_eq!(c.flavour, flavour, "{label}");
                assert!(info.version.starts_with(label), "{}", info.version);
                assert_eq!((info.database.as_str(), info.engine.as_str()), ("shop", label.to_lowercase().as_str()));
                assert!(info.user.starts_with("app@"), "{}", info.user);
                c.close().await;
            }
            // A wrong password says so, with the server's words.
            // (MySQL 8 without TLS asks for full authentication before it looks at the password: that has its own message.)
            let e = connect(port, "nope", TlsMode::Require).await.err().unwrap();
            assert_eq!(e.code, ErrorCode::DbAuthFailed, "{label}: {}", e.message);
            assert!(e.message.contains("Access denied"), "{}", e.message);
        }
        // Nothing listens on this port: unreachable, not a hang.
        assert!(connect(1, "x", TlsMode::Off).await.is_err());
    }

    #[tokio::test]
    #[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
    async fn l3_my_queries_keep_every_value_and_the_servers_errors_and_limits_work() {
        for (port, pw, label, _) in SERVERS {
            let (c, _) = connect(port, pw, TlsMode::Off).await.unwrap();
            let r = c.query("select 9223372036854775807 as big, 12345678901234567890.123456789 as dec_, 1.5e0 as f, 'né 😀' as u, null as n, x'deadbeef' as b, cast('{\"a\": [1]}' as json) as j, true as t, 'it''s' as q", 0).await;
            let r = match r {
                Ok(r) => r,
                // MariaDB's JSON is a text alias: the same statement without the cast.
                Err(_) => c.query("select 9223372036854775807 as big, 12345678901234567890.123456789 as dec_, 1.5e0 as f, 'né 😀' as u, null as n, x'deadbeef' as b, '{\"a\": [1]}' as j, true as t, 'it''s' as q", 0).await.unwrap(),
            };
            let row = &r.rows[0];
            assert_eq!(row[0].as_deref(), Some("9223372036854775807"), "{label}");
            assert_eq!(row[1].as_deref(), Some("12345678901234567890.123456789"), "{label}");
            assert_eq!(row[3].as_deref(), Some("né 😀"));
            assert_eq!(row[4], None);
            assert_eq!(row[5].as_deref(), Some("\\xdeadbeef"));
            assert_eq!(row[8].as_deref(), Some("it's"));
            assert_eq!(r.columns[0].kind, CellKind::Int);
            assert_eq!(r.columns[1].kind, CellKind::Decimal);
            assert_eq!(r.columns[5].kind, CellKind::Bytes);
            // The server's own error text, and the connection still works.
            let e = c.query("select * from shop.nope_not_here", 0).await.unwrap_err();
            assert_eq!(e.code, ErrorCode::DbRefused);
            assert!(e.message.contains("nope_not_here"), "{}", e.message);
            let e = c.query("selec 1", 0).await.unwrap_err();
            assert!(e.message.to_lowercase().contains("syntax"), "{}", e.message);
            // A limit: that many rows, `more` says the rest was dropped, and the connection is in step afterwards.
            let big = "select 1 as n from information_schema.columns a, information_schema.columns b limit 3000";
            let r = c.query(big, 100).await.unwrap();
            assert_eq!((r.rows.len(), r.more), (100, true), "{label}");
            let r = c.query(big, 0).await.unwrap();
            assert_eq!((r.rows.len(), r.more), (3000, false));
            // A statement that changes something reports how many rows.
            c.query("create temporary table t1 (a int)", 0).await.unwrap();
            assert_eq!(c.query("insert into t1 values (1), (2), (3)", 0).await.unwrap().affected, 3);
            c.close().await;
        }
    }

    #[tokio::test]
    #[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
    async fn l3_my_stop_kills_the_running_query_from_a_second_connection_and_a_time_limit_ends_a_select() {
        for (port, pw, label, flavour) in SERVERS {
            let (c, _) = connect(port, pw, TlsMode::Off).await.unwrap();
            let c = std::sync::Arc::new(c);
            let c2 = c.clone();
            let started = Instant::now();
            let job = tokio::spawn(async move { c2.query("select sleep(30)", 0).await });
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            c.cancel_running().await.unwrap();
            let r = job.await.unwrap();
            // MySQL answers an interrupted sleep() with 1; MariaDB with an error: either way it ended early.
            let _ = r;
            assert!(started.elapsed() < std::time::Duration::from_secs(10), "{label}: not cut short");
            // Still usable afterwards.
            assert_eq!(c.query("select 1", 0).await.unwrap().rows[0][0].as_deref(), Some("1"));
            c.set_timeout(1000).await.unwrap();
            let t = Instant::now();
            let _ = c.query("select sleep(20)", 0).await;
            let _ = flavour;
            assert!(t.elapsed() < std::time::Duration::from_secs(10), "{label}: the time limit did nothing");
            assert_eq!(c.query("select 2", 0).await.unwrap().rows[0][0].as_deref(), Some("2"));
        }
    }

    #[tokio::test]
    #[ignore = "needs the Docker test servers: scripts/test-servers.sh up"]
    async fn l3_my_through_a_host_both_servers_answer_with_tls() {
        use crate::dbs::spike::sign_in_to_a;
        use crate::ssh::tests::harness;
        let h = harness(None).await;
        let ssh = sign_in_to_a(&h).await;
        for (name, pw, label) in [("mysql", "test-mysql", "MySQL"), ("mariadb", "test-mariadb", "MariaDB")] {
            let target = Target { address: name.into(), port: 3306, tls: false, tls_server_name: None };
            let stream = transport::through_host_for(Engine::Mysql, &ssh, &target).await.unwrap();
            let login = MyLogin { user: "app".into(), password: Some(Zeroizing::new(pw.into())), database: Some("shop".into()), tls: TlsMode::Require, server_name: name.into() };
            let (c, info) = MyConn::connect(stream, login, reopen_to(1)).await.unwrap_or_else(|e| panic!("{label}: {}", e.message));
            assert!(info.version.starts_with(label), "{}", info.version);
            assert_eq!(c.query("select 41 + 1", 0).await.unwrap().rows[0][0].as_deref(), Some("42"));
        }
    }
}
