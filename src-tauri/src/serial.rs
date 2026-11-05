//! Serial connections (could-have M4): a device like `/dev/tty.usbserial-1410` or `COM3` shown in a
//! normal terminal tab, for routers, switches, microcontrollers and similar. The device path is the
//! host's `address`; baud rate and line settings are in `serial_settings`. Bytes go both ways
//! untouched (no newline conversion), through the same `Cmd` / `SessionIo` plumbing as SSH.

use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serialport::{DataBits, FlowControl, Parity, StopBits};
use specta::Type;
use tokio::sync::mpsc;
use tokio::time::{interval, MissedTickBehavior};

use crate::error::{AppError, AppResult, ErrorCode};
use crate::ssh::{Cmd, SessionEvent, SessionIo, SessionState};

const FLUSH_EVERY: Duration = Duration::from_millis(8);
const FLUSH_AT: usize = 64 * 1024;
const MAX_UNACKED: usize = 1024 * 1024;
/// How long a read waits before checking whether the tab was closed.
const POLL: Duration = Duration::from_millis(50);

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct SerialConfig {
    pub baud: u32,
    /// 5 to 8.
    pub data_bits: u8,
    /// 1 or 2.
    pub stop_bits: u8,
    /// `none`, `odd` or `even`.
    pub parity: String,
    /// `none`, `hardware` (RTS/CTS) or `software` (XON/XOFF).
    pub flow: String,
}

impl Default for SerialConfig {
    fn default() -> Self {
        SerialConfig {
            baud: 9600,
            data_bits: 8,
            stop_bits: 1,
            parity: "none".into(),
            flow: "none".into(),
        }
    }
}

fn validate(c: &SerialConfig) -> AppResult<()> {
    let ok = (50..=4_000_000).contains(&c.baud)
        && (5..=8).contains(&c.data_bits)
        && matches!(c.stop_bits, 1 | 2)
        && matches!(c.parity.as_str(), "none" | "odd" | "even")
        && matches!(c.flow.as_str(), "none" | "hardware" | "software");
    if ok {
        Ok(())
    } else {
        Err(AppError::validation(
            "Check the baud rate (50 to 4000000) and the line settings.",
        ))
    }
}

pub fn get(conn: &Connection, vault_id: &str, host_id: &str) -> AppResult<SerialConfig> {
    let row = conn
        .query_row(
            "SELECT s.baud, s.data_bits, s.stop_bits, s.parity, s.flow FROM serial_settings s JOIN hosts h ON h.id = s.host_id WHERE s.host_id = ?1 AND h.vault_id = ?2",
            params![host_id, vault_id],
            |r| Ok(SerialConfig { baud: r.get(0)?, data_bits: r.get(1)?, stop_bits: r.get(2)?, parity: r.get(3)?, flow: r.get(4)? }),
        )
        .optional()?;
    Ok(row.unwrap_or_default())
}

pub fn set(conn: &Connection, vault_id: &str, host_id: &str, c: &SerialConfig) -> AppResult<()> {
    validate(c)?;
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM hosts WHERE id = ?1 AND vault_id = ?2)",
        params![host_id, vault_id],
        |r| r.get(0),
    )?;
    if !exists {
        return Err(AppError::not_found("That host no longer exists."));
    }
    conn.execute(
        "INSERT INTO serial_settings (host_id, baud, data_bits, stop_bits, parity, flow) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(host_id) DO UPDATE SET baud = ?2, data_bits = ?3, stop_bits = ?4, parity = ?5, flow = ?6",
        params![host_id, c.baud, c.data_bits, c.stop_bits, c.parity, c.flow],
    )?;
    Ok(())
}

/// Serial devices this computer reports, for the host editor's picker.
pub fn list_ports() -> Vec<String> {
    let mut v: Vec<String> = serialport::available_ports()
        .map(|p| p.into_iter().map(|i| i.port_name).collect())
        .unwrap_or_default();
    // macOS lists each adapter twice (`tty.` and `cu.`); `cu.` is the one to dial out with.
    v.sort();
    v
}

fn state(io: &SessionIo, s: SessionState) {
    (io.event)(SessionEvent::State { state: s });
}

fn fail(io: &SessionIo, code: ErrorCode, message: String) {
    (io.event)(SessionEvent::Error { code, message });
}

/// Opens `path` and runs until the device goes away or the tab closes.
pub async fn run(
    io: Arc<SessionIo>,
    path: String,
    cfg: SerialConfig,
    cmds: mpsc::UnboundedReceiver<Cmd>,
) {
    state(&io, SessionState::Connecting);
    let builder = serialport::new(path.clone(), cfg.baud)
        .data_bits(match cfg.data_bits {
            5 => DataBits::Five,
            6 => DataBits::Six,
            7 => DataBits::Seven,
            _ => DataBits::Eight,
        })
        .stop_bits(if cfg.stop_bits == 2 {
            StopBits::Two
        } else {
            StopBits::One
        })
        .parity(match cfg.parity.as_str() {
            "odd" => Parity::Odd,
            "even" => Parity::Even,
            _ => Parity::None,
        })
        .flow_control(match cfg.flow.as_str() {
            "hardware" => FlowControl::Hardware,
            "software" => FlowControl::Software,
            _ => FlowControl::None,
        })
        .timeout(POLL);
    let opened = tokio::task::spawn_blocking(move || builder.open()).await;
    let port = match opened {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => {
            let code = if e.to_string().to_lowercase().contains("permission") {
                ErrorCode::PermissionDenied
            } else {
                ErrorCode::HostUnreachable
            };
            return fail(&io, code, format!("Can't open {path}: {e}. Is the device plugged in, and not used by another program?"));
        }
        Err(_) => {
            return fail(
                &io,
                ErrorCode::Internal,
                "Couldn't open the serial port.".into(),
            )
        }
    };
    let reader = match port.try_clone() {
        Ok(r) => r,
        Err(e) => {
            return fail(
                &io,
                ErrorCode::Internal,
                format!("Couldn't open {path}: {e}"),
            )
        }
    };

    run_io(io, path, reader, port, cmds).await;
}

/// The byte loop, separate from opening the device so tests can drive it with plain pipes.
/// `reader` must give up reads now and then (a timeout) so the thread notices when the tab closes.
async fn run_io<R, W>(
    io: Arc<SessionIo>,
    path: String,
    mut reader: R,
    mut port: W,
    mut cmds: mpsc::UnboundedReceiver<Cmd>,
) where
    R: Read + Send + 'static,
    W: Write,
{
    // Reads block (with a short timeout), so a thread does them. The channel is small: when the UI
    // is behind and we stop receiving, the thread blocks and the device's buffer fills (backpressure).
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, mut out) = mpsc::channel::<Vec<u8>>(16);
    let stop_r = Arc::clone(&stop);
    std::thread::spawn(move || {
        let mut chunk = [0u8; 4096];
        while !stop_r.load(Ordering::Relaxed) {
            match reader.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    if tx.blocking_send(chunk[..n].to_vec()).is_err() {
                        break;
                    }
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::TimedOut
                            | std::io::ErrorKind::WouldBlock
                            | std::io::ErrorKind::Interrupted
                    ) => {}
                Err(_) => break, // unplugged
            }
        }
    });

    state(&io, SessionState::Ready);
    let unacked = AtomicUsize::new(0);
    let mut buf: Vec<u8> = Vec::with_capacity(FLUSH_AT);
    let mut tick = interval(FLUSH_EVERY);
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let flush = |buf: &mut Vec<u8>| {
        if !buf.is_empty() {
            unacked.fetch_add(buf.len(), Ordering::Relaxed);
            (io.data)(std::mem::take(buf));
        }
    };
    let mut closed_by_user = false;
    loop {
        tokio::select! {
            cmd = cmds.recv() => match cmd {
                None | Some(Cmd::Close) => { closed_by_user = true; break; }
                // Keystrokes are tiny; a blocking write is fine.
                Some(Cmd::Write(b)) => { if port.write_all(&b).and_then(|()| port.flush()).is_err() { break; } }
                Some(Cmd::Resize(..)) => {} // a serial line has no window size
                Some(Cmd::Ack(n)) => {
                    crate::release_unacked(&unacked, n);
                }
            },
            chunk = out.recv(), if unacked.load(Ordering::Relaxed) < MAX_UNACKED => match chunk {
                None => break, // the device went away
                Some(c) => {
                    buf.extend_from_slice(&c);
                    if buf.len() >= FLUSH_AT { flush(&mut buf); }
                }
            },
            _ = tick.tick() => flush(&mut buf),
        }
    }
    flush(&mut buf);
    stop.store(true, Ordering::Relaxed);
    if closed_by_user {
        state(&io, SessionState::Closed);
    } else {
        fail(
            &io,
            ErrorCode::HostUnreachable,
            format!("{path} was disconnected."),
        );
        (io.event)(SessionEvent::Exit { status: None });
        state(&io, SessionState::Disconnected);
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::vault::{self, kdf::KdfParams};
    use std::sync::Mutex;

    #[test]
    fn settings_default_roundtrip_and_validation() {
        let c = open_in_memory().unwrap();
        let s = vault::create(&c, "correct-horse-staple", KdfParams::cheap()).unwrap();
        c.execute(
            "INSERT INTO connection_profiles (id, vault_id, created_at, updated_at) VALUES ('p', ?1, 0, 0)",
            [&s.vault_id],
        )
        .unwrap();
        c.execute(
            "INSERT INTO hosts (id, vault_id, profile_id, label, address, protocol, created_at, updated_at) VALUES ('h', ?1, 'p', 'router', '/dev/ttyUSB0', 'serial', 0, 0)",
            [&s.vault_id],
        )
        .unwrap();
        assert_eq!(get(&c, &s.vault_id, "h").unwrap(), SerialConfig::default()); // 9600 8N1
        let cfg = SerialConfig {
            baud: 115_200,
            data_bits: 7,
            stop_bits: 2,
            parity: "even".into(),
            flow: "hardware".into(),
        };
        set(&c, &s.vault_id, "h", &cfg).unwrap();
        assert_eq!(get(&c, &s.vault_id, "h").unwrap(), cfg);
        assert!(set(
            &c,
            &s.vault_id,
            "h",
            &SerialConfig {
                baud: 1,
                ..cfg.clone()
            }
        )
        .is_err());
        assert!(set(
            &c,
            &s.vault_id,
            "h",
            &SerialConfig {
                parity: "mark".into(),
                ..cfg.clone()
            }
        )
        .is_err());
        assert!(set(&c, &s.vault_id, "nope", &cfg).is_err());
        // Telnet hosts (migration 0002's value) survived the rebuild of the column.
        c.execute("UPDATE hosts SET protocol = 'telnet' WHERE id = 'h'", [])
            .unwrap();
    }

    /// A socket pair stands in for the adapter: what the "device" end writes shows up in the tab,
    /// what the tab sends arrives at the device, and closing the device ends the session.
    // Multi-threaded: the test itself blocks reading from the "device" while the session task runs.
    #[tokio::test(flavor = "multi_thread")]
    async fn bytes_flow_both_ways_and_unplugging_disconnects() {
        use std::os::unix::net::UnixStream;
        let (ours, mut device) = UnixStream::pair().unwrap();
        ours.set_read_timeout(Some(POLL)).unwrap();
        device
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let (reader, writer) = (ours.try_clone().unwrap(), ours);

        let shown: Arc<Mutex<Vec<u8>>> = Arc::default();
        let states: Arc<Mutex<Vec<String>>> = Arc::default();
        let (d, st) = (Arc::clone(&shown), Arc::clone(&states));
        let io = Arc::new(SessionIo {
            event: Box::new(move |e| match e {
                SessionEvent::State { state } => st.lock().unwrap().push(format!("{state:?}")),
                SessionEvent::Error { message, .. } => {
                    st.lock().unwrap().push(format!("Error: {message}"))
                }
                _ => {}
            }),
            data: Box::new(move |b| d.lock().unwrap().extend(b)),
        });
        let (tx, rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(run_io(io, "/dev/ttyUSB0".into(), reader, writer, rx));

        let wait = |cond: Box<dyn Fn() -> bool>| async move {
            tokio::time::timeout(Duration::from_secs(5), async {
                while !cond() {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("timed out");
        };
        device.write_all(b"router> ").unwrap();
        let sh = Arc::clone(&shown);
        wait(Box::new(move || {
            sh.lock().unwrap().as_slice() == b"router> "
        }))
        .await;

        tx.send(Cmd::Write(b"show\r".to_vec())).unwrap();
        let mut got = [0u8; 5];
        device.read_exact(&mut got).unwrap();
        assert_eq!(&got, b"show\r"); // untouched: no newline conversion

        // Unplugging: the device end closes while the tab is open.
        drop(device);
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("did not notice the unplug")
            .unwrap();
        let s = states.lock().unwrap().clone();
        assert_eq!(s.last().map(String::as_str), Some("Disconnected"), "{s:?}");
        assert!(s.iter().any(|e| e.contains("was disconnected")), "{s:?}");
    }

    #[tokio::test]
    async fn a_missing_device_reports_a_clear_error() {
        let events: Arc<Mutex<Vec<String>>> = Arc::default();
        let ev = Arc::clone(&events);
        let io = Arc::new(SessionIo {
            event: Box::new(move |e| {
                if let SessionEvent::Error { message, .. } = e {
                    ev.lock().unwrap().push(message);
                }
            }),
            data: Box::new(|_| {}),
        });
        let (_tx, rx) = mpsc::unbounded_channel();
        run(
            io,
            "/dev/does-not-exist".into(),
            SerialConfig::default(),
            rx,
        )
        .await;
        let msgs = events.lock().unwrap().clone();
        assert!(
            msgs.len() == 1 && msgs[0].contains("Can't open /dev/does-not-exist"),
            "{msgs:?}"
        );
    }
}
