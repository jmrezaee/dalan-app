//! How a database connection reaches its server (spec section 3, "Transport"). Both routes end as one
//! `AsyncRead + AsyncWrite` stream that is handed to the Redis client:
//!   - direct: a TCP connection from this computer;
//!   - through a host: a `direct-tcpip` channel on an SSH connection to that host, so the server sees the
//!     connection coming from the host itself.
//!
//! TLS, when the database asks for it, runs inside either stream.
//!
//! There is deliberately **no local listener**: a loopback port would let any program on this computer talk to a
//! production Redis.

use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::rustls::{self, ClientConfig, RootCertStore};
use tokio_rustls::TlsConnector;

use crate::error::{AppError, AppResult, ErrorCode};
use crate::ssh::Connected;

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Anything the Redis client can read from and write to. A trait object, so the two routes and TLS all look alike.
pub trait Stream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Stream for T {}
pub type BoxStream = Box<dyn Stream>;

/// Where Redis is, as seen from the route's last hop.
#[derive(Debug, Clone)]
pub struct Target {
    pub address: String,
    pub port: u16,
    pub tls: bool,
    /// The name the certificate must be for; the address when empty.
    pub tls_server_name: Option<String>,
}

impl Target {
    pub fn display(&self) -> String {
        if self.address.contains(':') {
            format!("[{}]:{}", self.address, self.port)
        } else {
            format!("{}:{}", self.address, self.port)
        }
    }
}

fn unreachable(target: &Target, from: Option<&str>) -> AppError {
    AppError::new(
        ErrorCode::RedisUnreachable,
        match from {
            Some(host) => format!(
                "Can't reach Redis from {host}. Nothing answered at {}.",
                target.display()
            ),
            None => format!(
                "Can't reach Redis. Nothing answered at {}.",
                target.display()
            ),
        },
    )
}

/// A TCP connection from this computer, then TLS if the database uses it.
pub async fn direct(target: &Target) -> AppResult<BoxStream> {
    let tcp = tokio::time::timeout(
        CONNECT_TIMEOUT,
        TcpStream::connect((target.address.as_str(), target.port)),
    )
    .await
    .map_err(|_| unreachable(target, None))?
    .map_err(|_| unreachable(target, None))?;
    let _ = tcp.set_nodelay(true);
    finish(Box::new(tcp), target).await
}

/// A channel from the host to Redis, then TLS if the database uses it. `ssh` must stay alive as long as the stream.
pub async fn through_host(ssh: &Connected, target: &Target) -> AppResult<BoxStream> {
    let channel = tokio::time::timeout(
        CONNECT_TIMEOUT,
        // The last two arguments are the "originator" the SSH server logs; there is no local socket to name.
        ssh.handle.channel_open_direct_tcpip(
            target.address.clone(),
            u32::from(target.port),
            "127.0.0.1",
            0,
        ),
    )
    .await
    .map_err(|_| unreachable(target, Some(&ssh.label)))?
    .map_err(|_| unreachable(target, Some(&ssh.label)))?;
    finish(Box::new(channel.into_stream()), target).await
}

async fn finish(stream: BoxStream, target: &Target) -> AppResult<BoxStream> {
    if !target.tls {
        return Ok(stream);
    }
    let name = target
        .tls_server_name
        .clone()
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| target.address.clone());
    let server_name = ServerName::try_from(name.clone())
        .map_err(|_| AppError::validation(format!("{name} can't be used as a TLS server name.")))?;
    let connector = TlsConnector::from(tls_config()?);
    let tls = tokio::time::timeout(CONNECT_TIMEOUT, connector.connect(server_name, stream))
        .await
        .map_err(|_| {
            AppError::new(
                ErrorCode::RedisUnreachable,
                "The TLS handshake took too long. Is this port really TLS?",
            )
        })?
        .map_err(|e| tls_error(&e, &name))?;
    Ok(Box::new(tls))
}

/// The certificate is always checked against the authorities this computer trusts. There is no "skip" switch.
fn tls_config() -> AppResult<Arc<ClientConfig>> {
    let mut roots = RootCertStore::empty();
    let found = rustls_native_certs::load_native_certs();
    for cert in found.certs {
        // A certificate the store can't parse is skipped; the rest still count.
        let _ = roots.add(cert);
    }
    if roots.is_empty() {
        return Err(AppError::internal(
            "This computer's list of trusted certificate authorities couldn't be read.",
        ));
    }
    // Named explicitly: with two crypto libraries in the dependency tree rustls can't pick one by itself.
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| AppError::internal(format!("TLS setup failed: {e}")))?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(Arc::new(config))
}

/// Turns a failed handshake into our codes. A certificate problem is `RedisTlsUntrusted`; the rest is "unreachable".
fn tls_error(e: &std::io::Error, name: &str) -> AppError {
    let cert = e
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<rustls::Error>())
        .and_then(|r| match r {
            rustls::Error::InvalidCertificate(c) => Some(c.clone()),
            _ => None,
        });
    match cert {
        Some(rustls::CertificateError::NotValidForName)
        | Some(rustls::CertificateError::NotValidForNameContext { .. }) => AppError::new(
            ErrorCode::RedisTlsUntrusted,
            format!("This server's certificate is for another name, not {name}."),
        ),
        Some(rustls::CertificateError::Expired)
        | Some(rustls::CertificateError::ExpiredContext { .. }) => AppError::new(
            ErrorCode::RedisTlsUntrusted,
            "This server's certificate has expired.",
        ),
        Some(_) => AppError::new(
            ErrorCode::RedisTlsUntrusted,
            "This server's certificate isn't trusted. It was issued by an authority this computer does not know.",
        ),
        None => AppError::new(
            ErrorCode::RedisUnreachable,
            "The TLS handshake failed. Is this port really TLS?",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_target_is_shown_with_brackets_for_ipv6() {
        let t = |a: &str| Target {
            address: a.into(),
            port: 6379,
            tls: false,
            tls_server_name: None,
        };
        assert_eq!(t("127.0.0.1").display(), "127.0.0.1:6379");
        assert_eq!(t("cache.internal").display(), "cache.internal:6379");
        assert_eq!(t("::1").display(), "[::1]:6379");
    }

    #[test]
    fn the_tls_setup_reads_this_computers_authorities() {
        // No network: only that the config builds (a provider is named, roots are found).
        assert!(tls_config().is_ok());
    }

    #[tokio::test]
    async fn nothing_listening_is_unreachable_and_says_where() {
        // Port 1 on loopback: refused at once on every system we build for.
        let err = match direct(&Target {
            address: "127.0.0.1".into(),
            port: 1,
            tls: false,
            tls_server_name: None,
        })
        .await
        {
            Ok(_) => panic!("nothing should listen on port 1"),
            Err(e) => e,
        };
        assert_eq!(err.code, ErrorCode::RedisUnreachable);
        assert!(err.message.contains("127.0.0.1:1"), "{}", err.message);
    }
}
