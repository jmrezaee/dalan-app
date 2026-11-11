//! `ssh://` links (replica/windows-installer/spec.md section 9). On Windows the installer can make Dalan the handler of
//! `ssh://` links; Windows then starts `dalan-app.exe --open-url "ssh://user@host:22"`, or, when Dalan is already
//! running, starts a second copy that the single-instance plugin hands the same arguments to.
//!
//! A link is untrusted input from another app. So:
//!   - this module only *parses* it, strictly, into a user, a host and a port;
//!   - the UI shows what was asked and waits for Connect: **a link never connects by itself**;
//!   - nothing from the link is ever passed on as an option (a user or host can't start with `-`).

use std::collections::VecDeque;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::Mutex;

use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Manager};
use tauri_specta::Event;

use crate::events::LinkReceived;

/// What a valid link asked for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct SshTarget {
    pub user: Option<String>,
    pub host: String,
    pub port: u16,
}

const MAX_LINK_LEN: usize = 255;
const MAX_USER_LEN: usize = 64;

/// Reads `ssh://[user@]host[:port]` and nothing else. `None` for anything that doesn't fit, which the UI reports as
/// "This link isn't a valid ssh:// address."
pub fn parse_ssh_url(raw: &str) -> Option<SshTarget> {
    if raw.len() > MAX_LINK_LEN || raw.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return None;
    }
    let rest = strip_scheme(raw)?;
    // One optional "/" at the end is fine (`ssh://host/`). A path, query or fragment is not.
    let authority = rest.strip_suffix('/').unwrap_or(rest);
    if authority.is_empty() || authority.contains(['/', '?', '#', '\\']) {
        return None;
    }
    let (userinfo, hostport) = match authority.rsplit_once('@') {
        Some((u, h)) => (Some(u), h),
        None => (None, authority),
    };

    let user = match userinfo {
        None => None,
        Some(u) => {
            // `user;fingerprint=...` : the fingerprint is never trusted, so it is dropped. A password
            // (`user:secret@`) is refused: a link must not carry one.
            let name = u.split(';').next().unwrap_or("");
            let name = percent_decode_once(name)?;
            if !valid_user(&name) {
                return None;
            }
            Some(name)
        }
    };

    let (host_text, port) = split_host_port(hostport)?;
    let host = percent_decode_once(host_text)?;
    if !valid_host(&host) {
        return None;
    }
    Some(SshTarget {
        user,
        host: host.trim_matches(['[', ']']).to_string(),
        port,
    })
}

/// `[::1]:2222`, `host:2222`, `host`. No port means 22.
fn split_host_port(s: &str) -> Option<(&str, u16)> {
    let (host, port) = if let Some(rest) = s.strip_prefix('[') {
        let (inside, after) = rest.split_once(']')?;
        let host = &s[..inside.len() + 2];
        match after {
            "" => (host, None),
            a => (host, Some(a.strip_prefix(':')?)),
        }
    } else {
        match s.rsplit_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (s, None),
        }
    };
    let port = match port {
        None => 22,
        Some(p) if !p.is_empty() && p.len() <= 5 && p.bytes().all(|b| b.is_ascii_digit()) => {
            p.parse::<u16>().ok().filter(|p| *p >= 1)?
        }
        Some(_) => return None,
    };
    Some((host, port))
}

fn strip_scheme(raw: &str) -> Option<&str> {
    let (scheme, rest) = raw.split_once("://")?;
    scheme.eq_ignore_ascii_case("ssh").then_some(rest)
}

/// `%41` -> `A`, once. A stray `%` or a result that isn't UTF-8 is refused. The decoded text is checked by the caller,
/// so `%2D` can't smuggle in a leading `-`.
fn percent_decode_once(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// 1 to 64 of `A-Z a-z 0-9 . _ -`, not starting with `-`.
fn valid_user(u: &str) -> bool {
    !u.is_empty()
        && u.len() <= MAX_USER_LEN
        && !u.starts_with('-')
        && u.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// A hostname, an IPv4 address or a bracketed IPv6 address; never starting with `-`.
fn valid_host(h: &str) -> bool {
    if let Some(inner) = h.strip_prefix('[').and_then(|x| x.strip_suffix(']')) {
        return inner.parse::<Ipv6Addr>().is_ok();
    }
    if h.is_empty() || h.len() > 253 || h.starts_with('-') {
        return false;
    }
    if h.parse::<Ipv4Addr>().is_ok() {
        return true;
    }
    h.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    })
}

/// The value after `--open-url`: `--open-url <link>` or `--open-url=<link>`.
pub fn link_arg(args: &[String]) -> Option<&str> {
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--open-url" {
            return it.next().map(String::as_str);
        }
        if let Some(v) = a.strip_prefix("--open-url=") {
            return Some(v);
        }
    }
    None
}

/// `--minimized`: start in the taskbar (used by "Start Dalan when I sign in to Windows").
pub fn wants_minimized(args: &[String]) -> bool {
    args.iter().any(|a| a == "--minimized")
}

/// Links that arrived and that the UI hasn't picked up yet. A link can arrive before the UI is listening (a cold
/// start), so the event only says "look"; the UI then takes the queue. A handful at most: a flood is dropped.
#[derive(Default)]
pub struct LinkQueue(Mutex<VecDeque<Option<SshTarget>>>);

const MAX_QUEUED: usize = 5;

impl LinkQueue {
    fn push(&self, t: Option<SshTarget>) {
        let mut q = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if q.len() >= MAX_QUEUED {
            q.pop_front();
        }
        q.push_back(t);
    }

    /// Everything waiting, oldest first. `None` is a link that didn't parse.
    pub fn take(&self) -> Vec<Option<SshTarget>> {
        let mut q = self.0.lock().unwrap_or_else(|e| e.into_inner());
        q.drain(..).collect()
    }
}

/// The program's arguments, from a cold start or from a second launch (single-instance): a link is queued and the UI
/// told; the window is brought forward. A start with no link does nothing here.
pub fn handle_args(app: &AppHandle, args: &[String]) {
    let Some(raw) = link_arg(args) else { return };
    if let Some(q) = app.try_state::<LinkQueue>() {
        q.push(parse_ssh_url(raw));
    }
    let _ = LinkReceived {}.emit(app);
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(s: &str) -> SshTarget {
        parse_ssh_url(s).unwrap_or_else(|| panic!("should parse: {s}"))
    }
    fn target(user: Option<&str>, host: &str, port: u16) -> SshTarget {
        SshTarget {
            user: user.map(str::to_string),
            host: host.to_string(),
            port,
        }
    }

    #[test]
    fn plain_links() {
        assert_eq!(ok("ssh://host"), target(None, "host", 22));
        assert_eq!(
            ok("ssh://deploy@10.0.0.5"),
            target(Some("deploy"), "10.0.0.5", 22)
        );
        assert_eq!(
            ok("ssh://deploy@web-1.example.com:2222"),
            target(Some("deploy"), "web-1.example.com", 2222)
        );
        assert_eq!(ok("SSH://Host/"), target(None, "Host", 22));
        assert_eq!(ok("ssh://a_b.c@h"), target(Some("a_b.c"), "h", 22));
    }

    #[test]
    fn ipv6_in_brackets() {
        assert_eq!(ok("ssh://[::1]"), target(None, "::1", 22));
        assert_eq!(
            ok("ssh://root@[2001:db8::7]:2200"),
            target(Some("root"), "2001:db8::7", 2200)
        );
        assert!(parse_ssh_url("ssh://[::1").is_none());
        assert!(parse_ssh_url("ssh://[not-an-address]").is_none());
        assert!(parse_ssh_url("ssh://[::1]x").is_none());
        // Without brackets a colon is a port separator, and "::1" is not a port.
        assert!(parse_ssh_url("ssh://::1").is_none());
    }

    #[test]
    fn the_fingerprint_parameter_is_ignored() {
        assert_eq!(
            ok("ssh://deploy;fingerprint=ab:cd@host"),
            target(Some("deploy"), "host", 22)
        );
    }

    #[test]
    fn percent_encoding_is_decoded_once() {
        assert_eq!(
            ok("ssh://de%70loy@host"),
            target(Some("deploy"), "host", 22)
        );
        // "%2541" decodes once to "%41", which is not a valid user: it is refused, not decoded a second time.
        assert!(parse_ssh_url("ssh://%2541@host").is_none());
        assert!(parse_ssh_url("ssh://user@ho%zzst").is_none());
        assert!(parse_ssh_url("ssh://user@host%").is_none());
    }

    #[test]
    fn injection_attempts_are_refused() {
        for bad in [
            "ssh://-oProxyCommand=calc@h",
            "ssh://-o@h",
            "ssh://%2Doops@h",
            "ssh://user@-host",
            "ssh://user@%2Dhost",
            "ssh://h -o x",
            "ssh://h\t-oProxyCommand=x",
            "ssh://user@host\nsecond",
            "ssh://user@host\0",
            "ssh://user@host;rm -rf /",
            "ssh://user@host`id`",
            "ssh://user@$(id)",
            "ssh://user:secret@host",
            "ssh://us er@host",
            "ssh://user@host/path",
            "ssh://user@host/?x=1",
            "ssh://user@host#frag",
            "ssh://user@host\\share",
        ] {
            assert!(parse_ssh_url(bad).is_none(), "should refuse: {bad:?}");
        }
    }

    #[test]
    fn other_schemes_and_shapes_are_refused() {
        for bad in [
            "",
            "ssh://",
            "ssh:///",
            "ssh:host",
            "http://host",
            "ssh+git://host",
            "host",
            "ssh://@host",
            "ssh://user@",
            "ssh://user@@host",
            "ssh://host.",
            "ssh://ho..st",
            "ssh://-",
            "ssh://host-:22",
        ] {
            assert!(parse_ssh_url(bad).is_none(), "should refuse: {bad:?}");
        }
    }

    #[test]
    fn ports() {
        assert_eq!(ok("ssh://h:1").port, 1);
        assert_eq!(ok("ssh://h:65535").port, 65535);
        for bad in [
            "ssh://h:0",
            "ssh://h:65536",
            "ssh://h:99999",
            "ssh://h:",
            "ssh://h:22x",
            "ssh://h:-1",
            "ssh://h:+22",
            "ssh://h:123456",
        ] {
            assert!(parse_ssh_url(bad).is_none(), "should refuse: {bad:?}");
        }
    }

    #[test]
    fn length_limits() {
        let long_user = "a".repeat(65);
        assert!(parse_ssh_url(&format!("ssh://{long_user}@h")).is_none());
        assert!(parse_ssh_url(&format!("ssh://{}@h", "a".repeat(64))).is_some());
        let long_label = "a".repeat(64);
        assert!(parse_ssh_url(&format!("ssh://{long_label}.com")).is_none());
        // 256 characters in all
        let overlong = format!("ssh://{}", "a.".repeat(125));
        assert!(overlong.len() > 255);
        assert!(parse_ssh_url(&overlong).is_none());
        assert!(parse_ssh_url(&format!("ssh://{}", "x".repeat(10_000))).is_none());
    }

    #[test]
    fn non_ascii_is_refused() {
        assert!(parse_ssh_url("ssh://usér@host").is_none());
        assert!(parse_ssh_url("ssh://user@höst").is_none());
        assert!(parse_ssh_url("ssh://user@host\u{202e}").is_none());
    }

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn link_and_minimized_arguments() {
        assert_eq!(
            link_arg(&args(&["dalan-app.exe", "--open-url", "ssh://h"])),
            Some("ssh://h")
        );
        assert_eq!(
            link_arg(&args(&["dalan-app.exe", "--open-url=ssh://h"])),
            Some("ssh://h")
        );
        assert_eq!(link_arg(&args(&["dalan-app.exe"])), None);
        assert_eq!(link_arg(&args(&["dalan-app.exe", "--open-url"])), None);
        assert!(wants_minimized(&args(&["dalan-app.exe", "--minimized"])));
        assert!(!wants_minimized(&args(&["dalan-app.exe"])));
    }

    #[test]
    fn the_queue_keeps_the_newest_few_and_empties_when_taken() {
        let q = LinkQueue::default();
        for i in 0..8u16 {
            q.push(Some(target(None, "h", i + 1)));
        }
        q.push(None);
        let got = q.take();
        assert_eq!(got.len(), MAX_QUEUED);
        assert_eq!(got.last(), Some(&None));
        assert!(q.take().is_empty());
    }
}
