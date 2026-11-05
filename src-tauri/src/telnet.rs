//! Telnet sessions (could-have M4): a plain TCP connection with the Telnet option handshake, shown
//! in a normal terminal tab. It speaks the same `Cmd` / `SessionIo` language as SSH and the local
//! terminal. **Nothing is encrypted**: everything typed, including a password, crosses the network
//! in the clear, so the host editor warns about it and Dalan never saves a Telnet password.
//!
//! Negotiation, kept small on purpose: we accept `ECHO` and `SUPPRESS-GO-AHEAD` from the server,
//! offer `TERMINAL-TYPE` (xterm-256color) and `NAWS` (window size), and refuse everything else.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::time::{interval, timeout, MissedTickBehavior};

use crate::error::ErrorCode;
use crate::ssh::{Cmd, SessionEvent, SessionIo, SessionState};

const IAC: u8 = 255;
const DONT: u8 = 254;
const DO: u8 = 253;
const WONT: u8 = 252;
const WILL: u8 = 251;
const SB: u8 = 250;
const SE: u8 = 240;
const OPT_ECHO: u8 = 1;
const OPT_SGA: u8 = 3;
const OPT_TTYPE: u8 = 24;
const OPT_NAWS: u8 = 31;
const TTYPE_IS: u8 = 0;
const TTYPE_SEND: u8 = 1;

const FLUSH_EVERY: Duration = Duration::from_millis(8);
const FLUSH_AT: usize = 64 * 1024;
const MAX_UNACKED: usize = 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Splits the byte stream into terminal text and Telnet commands, and answers the commands.
/// State carries over between reads, because a command can arrive split across packets.
pub struct Parser {
    state: State,
    sub: Vec<u8>,
    /// Options we already agreed to (so a repeated request isn't answered again, which could loop).
    agreed: [bool; 256],
    pub cols: u16,
    pub rows: u16,
}

#[derive(Default, PartialEq)]
enum State {
    #[default]
    Data,
    Iac,
    Verb(u8),
    Sub,
    SubIac,
}

impl Parser {
    pub fn new(cols: u16, rows: u16) -> Self {
        Parser {
            state: State::Data,
            sub: Vec::new(),
            agreed: [false; 256],
            cols,
            rows,
        }
    }

    /// Returns (text for the terminal, bytes to send back to the server).
    pub fn feed(&mut self, input: &[u8]) -> (Vec<u8>, Vec<u8>) {
        let (mut text, mut reply) = (Vec::with_capacity(input.len()), Vec::new());
        for &b in input {
            match std::mem::take(&mut self.state) {
                State::Data => {
                    if b == IAC {
                        self.state = State::Iac;
                    } else {
                        text.push(b);
                    }
                }
                State::Iac => match b {
                    IAC => text.push(IAC), // an escaped 0xFF is data
                    DO | DONT | WILL | WONT => self.state = State::Verb(b),
                    SB => {
                        self.sub.clear();
                        self.state = State::Sub;
                    }
                    _ => {} // NOP, GA and friends: nothing to do
                },
                State::Verb(verb) => self.negotiate(verb, b, &mut reply),
                State::Sub => {
                    if b == IAC {
                        self.state = State::SubIac;
                    } else {
                        self.sub.push(b);
                        self.state = State::Sub;
                    }
                }
                State::SubIac => match b {
                    SE => self.subnegotiation(&mut reply),
                    IAC => {
                        self.sub.push(IAC);
                        self.state = State::Sub;
                    }
                    _ => self.state = State::Sub,
                },
            }
        }
        (text, reply)
    }

    fn negotiate(&mut self, verb: u8, opt: u8, reply: &mut Vec<u8>) {
        let slot = &mut self.agreed[usize::from(opt)];
        match verb {
            // The server offers to do something (echo, suppress go-ahead): fine for ECHO and SGA.
            WILL if matches!(opt, OPT_ECHO | OPT_SGA) => {
                if !*slot {
                    *slot = true;
                    reply.extend([IAC, DO, opt]);
                }
            }
            WILL => reply.extend([IAC, DONT, opt]),
            // The server asks us to do something.
            DO if matches!(opt, OPT_TTYPE | OPT_NAWS | OPT_SGA) => {
                if !*slot {
                    *slot = true;
                    reply.extend([IAC, WILL, opt]);
                    if opt == OPT_NAWS {
                        reply.extend(naws(self.cols, self.rows));
                    }
                }
            }
            DO => reply.extend([IAC, WONT, opt]),
            // DONT / WONT: the server agrees we don't, nothing to answer.
            _ => *slot = false,
        }
    }

    fn subnegotiation(&mut self, reply: &mut Vec<u8>) {
        if self.sub.first() == Some(&OPT_TTYPE) && self.sub.get(1) == Some(&TTYPE_SEND) {
            reply.extend([IAC, SB, OPT_TTYPE, TTYPE_IS]);
            reply.extend(b"xterm-256color");
            reply.extend([IAC, SE]);
        }
    }

    /// The window changed: tell the server, if it asked for sizes.
    pub fn resize(&mut self, cols: u16, rows: u16) -> Vec<u8> {
        self.cols = cols;
        self.rows = rows;
        if self.agreed[usize::from(OPT_NAWS)] {
            naws(cols, rows)
        } else {
            Vec::new()
        }
    }
}

/// `IAC SB NAWS cols rows IAC SE`, with any 0xFF in the numbers doubled.
fn naws(cols: u16, rows: u16) -> Vec<u8> {
    let mut v = vec![IAC, SB, OPT_NAWS];
    for b in cols.to_be_bytes().into_iter().chain(rows.to_be_bytes()) {
        v.push(b);
        if b == IAC {
            v.push(IAC);
        }
    }
    v.extend([IAC, SE]);
    v
}

/// What the user typed, as Telnet wants it: 0xFF doubled, Enter (CR) as CR LF.
pub fn encode_input(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    for &b in bytes {
        match b {
            IAC => out.extend([IAC, IAC]),
            b'\r' => out.extend(*b"\r\n"),
            _ => out.push(b),
        }
    }
    out
}

fn state(io: &SessionIo, s: SessionState) {
    (io.event)(SessionEvent::State { state: s });
}

fn fail(io: &SessionIo, code: ErrorCode, message: String) {
    (io.event)(SessionEvent::Error { code, message });
}

/// Connects and runs until the server closes the connection or the tab closes.
pub async fn run(
    io: Arc<SessionIo>,
    address: String,
    port: u16,
    cols: u32,
    rows: u32,
    mut cmds: mpsc::UnboundedReceiver<Cmd>,
) {
    state(&io, SessionState::Connecting);
    let stream = match timeout(
        CONNECT_TIMEOUT,
        TcpStream::connect((address.as_str(), port)),
    )
    .await
    {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => {
            return fail(
                &io,
                ErrorCode::HostUnreachable,
                format!("Can't reach {address}:{port}: {e}"),
            )
        }
        Err(_) => {
            return fail(
                &io,
                ErrorCode::HostUnreachable,
                format!("Can't reach {address}:{port}: timed out"),
            )
        }
    };
    let _ = stream.set_nodelay(true);
    let (mut rd, mut wr) = stream.into_split();
    let mut parser = Parser::new(cols.clamp(1, 1000) as u16, rows.clamp(1, 500) as u16);
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
    let mut chunk = vec![0u8; 16 * 1024];
    let mut closed_by_user = false;

    loop {
        tokio::select! {
            cmd = cmds.recv() => match cmd {
                None | Some(Cmd::Close) => { closed_by_user = true; break; }
                Some(Cmd::Write(b)) => { if wr.write_all(&encode_input(&b)).await.is_err() { break; } }
                Some(Cmd::Resize(c, r)) => {
                    let sub = parser.resize(c.clamp(1, 1000) as u16, r.clamp(1, 500) as u16);
                    if !sub.is_empty() && wr.write_all(&sub).await.is_err() { break; }
                }
                Some(Cmd::Ack(n)) => {
                    crate::release_unacked(&unacked, n);
                }
            },
            // Not read while the UI is behind: the socket fills and the server slows down.
            n = rd.read(&mut chunk), if unacked.load(Ordering::Relaxed) < MAX_UNACKED => match n {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let (text, reply) = parser.feed(&chunk[..n]);
                    if !reply.is_empty() && wr.write_all(&reply).await.is_err() { break; }
                    buf.extend_from_slice(&text);
                    if buf.len() >= FLUSH_AT { flush(&mut buf); }
                }
            },
            _ = tick.tick() => flush(&mut buf),
        }
    }
    flush(&mut buf);
    if closed_by_user {
        state(&io, SessionState::Closed);
    } else {
        (io.event)(SessionEvent::Exit { status: None });
        state(&io, SessionState::Disconnected);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tokio::net::TcpListener;

    #[test]
    fn parser_strips_commands_answers_them_and_survives_split_packets() {
        let mut p = Parser::new(100, 30);
        // WILL ECHO, DO TTYPE, DO NAWS, DO (unknown 99), then text with an escaped 0xFF.
        let (text, reply) = p.feed(&[
            IAC, WILL, OPT_ECHO, IAC, DO, OPT_TTYPE, IAC, DO, OPT_NAWS, IAC, DO, 99, b'h', b'i',
            IAC, IAC,
        ]);
        assert_eq!(text, vec![b'h', b'i', 255]);
        let mut want = vec![IAC, DO, OPT_ECHO, IAC, WILL, OPT_TTYPE, IAC, WILL, OPT_NAWS];
        want.extend(naws(100, 30));
        want.extend([IAC, WONT, 99]);
        assert_eq!(reply, want);
        // The same request again isn't answered again (no negotiation loops).
        assert!(p.feed(&[IAC, WILL, OPT_ECHO]).1.is_empty());
        // Terminal type request, split in the middle of the command.
        let (_, r1) = p.feed(&[IAC, SB, OPT_TTYPE]);
        let (_, r2) = p.feed(&[TTYPE_SEND, IAC, SE]);
        assert!(r1.is_empty());
        assert!(r2.starts_with(&[IAC, SB, OPT_TTYPE, TTYPE_IS]) && r2.ends_with(&[IAC, SE]));
        // Resizing sends a new size now that NAWS is on; a size with 255 in it is escaped.
        assert_eq!(
            p.resize(255, 40),
            vec![IAC, SB, OPT_NAWS, 0, 255, 255, 0, 40, IAC, SE]
        );
        // WILL for something we don't want is refused.
        assert_eq!(p.feed(&[IAC, WILL, 42]).1, vec![IAC, DONT, 42]);
    }

    #[test]
    fn typed_bytes_are_escaped() {
        assert_eq!(encode_input(b"ls\r"), b"ls\r\n");
        assert_eq!(encode_input(&[b'a', 255]), vec![b'a', 255, 255]);
    }

    #[tokio::test]
    async fn session_negotiates_shows_text_and_sends_input() {
        // A tiny server: asks to negotiate, prints a banner, then records what it receives.
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        let seen: Arc<Mutex<Vec<u8>>> = Arc::default();
        let s2 = Arc::clone(&seen);
        tokio::spawn(async move {
            let (mut c, _) = l.accept().await.unwrap();
            c.write_all(&[IAC, WILL, OPT_ECHO, IAC, DO, OPT_NAWS])
                .await
                .unwrap();
            c.write_all(b"login: ").await.unwrap();
            let mut b = [0u8; 256];
            while let Ok(n) = c.read(&mut b).await {
                if n == 0 {
                    break;
                }
                s2.lock().unwrap().extend_from_slice(&b[..n]);
            }
        });
        let shown: Arc<Mutex<Vec<u8>>> = Arc::default();
        let states: Arc<Mutex<Vec<String>>> = Arc::default();
        let (d, st) = (Arc::clone(&shown), Arc::clone(&states));
        let io = Arc::new(SessionIo {
            event: Box::new(move |e| {
                if let SessionEvent::State { state } = e {
                    st.lock().unwrap().push(format!("{state:?}"));
                }
            }),
            data: Box::new(move |b| d.lock().unwrap().extend(b)),
        });
        let (tx, rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(run(io, "127.0.0.1".into(), port, 80, 24, rx));
        let wait = |cond: Box<dyn Fn() -> bool>| async move {
            timeout(Duration::from_secs(5), async {
                while !cond() {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("timed out");
        };
        let sh = Arc::clone(&shown);
        wait(Box::new(move || {
            sh.lock().unwrap().as_slice() == b"login: "
        }))
        .await; // commands stripped
        tx.send(Cmd::Write(b"bob\r".to_vec())).unwrap();
        let sn = Arc::clone(&seen);
        wait(Box::new(move || sn.lock().unwrap().ends_with(b"bob\r\n"))).await;
        let got = seen.lock().unwrap().clone();
        assert!(got.starts_with(&[
            IAC, DO, OPT_ECHO, IAC, WILL, OPT_NAWS, IAC, SB, OPT_NAWS, 0, 80, 0, 24, IAC, SE
        ]));
        tx.send(Cmd::Close).unwrap();
        timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            states.lock().unwrap().last().map(String::as_str),
            Some("Closed")
        );
    }
}
