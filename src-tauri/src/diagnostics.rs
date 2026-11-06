//! "Copy diagnostics" (Settings -> About): version, OS and the last log lines, with anything that
//! names your servers or you removed, so it can be pasted into an issue. Nothing is sent anywhere.

use rusqlite::Connection;

use crate::error::AppResult;

pub const LOG_LINES: usize = 200;

/// Removes the home folder, every term in `terms` (host labels, addresses, usernames from the
/// vault), IPv4 addresses and `name@host` pairs. Matching of `terms` ignores case.
pub fn redact(text: &str, home: Option<&str>, terms: &[String]) -> String {
    let mut out = text.to_string();
    if let Some(h) = home.filter(|h| h.len() > 1) {
        out = out.replace(h, "~");
    }
    // Longest first, so "web-1.example.com" goes before "web-1".
    let mut terms: Vec<&String> = terms
        .iter()
        .filter(|t| t.trim().chars().count() >= 3)
        .collect();
    terms.sort_by_key(|t| std::cmp::Reverse(t.len()));
    for t in terms {
        out = replace_ignore_case(&out, t.trim(), "[redacted]");
    }
    mask_emails(&mask_ipv6(&mask_ipv4(&out)))
}

fn replace_ignore_case(hay: &str, needle: &str, with: &str) -> String {
    let (lh, ln) = (hay.to_lowercase(), needle.to_lowercase());
    // Lowercasing can change byte lengths for odd characters; then skip rather than slice wrongly.
    if lh.len() != hay.len() || ln.is_empty() {
        return hay.replace(needle, with);
    }
    let mut out = String::new();
    let mut last = 0;
    while let Some(i) = lh[last..].find(&ln) {
        out.push_str(&hay[last..last + i]);
        out.push_str(with);
        last += i + ln.len();
    }
    out.push_str(&hay[last..]);
    out
}

fn is_name(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')
}

/// `deploy@10.0.0.5` or `me@host.example` -> `[user]@[host]`.
fn mask_emails(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '@' {
            // Take back what we already pushed for the name before '@'.
            let mut left = 0;
            while out.chars().rev().nth(left).is_some_and(is_name) {
                left += 1;
            }
            let mut j = i + 1;
            // `[redacted]` / `[ip]` from the earlier passes count as a host name too.
            while j < chars.len() && (is_name(chars[j]) || matches!(chars[j], '[' | ']')) {
                j += 1;
            }
            if left > 0 && j > i + 1 {
                for _ in 0..left {
                    out.pop();
                }
                out.push_str("[user]@[host]");
                i = j;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// `a.b.c.d` with 1-3 digit parts -> `[ip]`.
fn mask_ipv4(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit() && (i == 0 || !is_name(chars[i - 1])) {
            let mut j = i;
            let mut parts = 0;
            loop {
                let start = j;
                while j < chars.len() && chars[j].is_ascii_digit() && j - start < 3 {
                    j += 1;
                }
                if j == start {
                    break;
                }
                parts += 1;
                if parts < 4 && j < chars.len() && chars[j] == '.' {
                    j += 1;
                } else {
                    break;
                }
            }
            let ends_clean = j >= chars.len() || !chars[j].is_ascii_alphanumeric();
            if parts == 4 && ends_clean {
                out.push_str("[ip]");
                i = j;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// IPv6 addresses (`fe80::1`, `2001:db8::7334`, eight full groups) -> `[ip]`. A run of hex digits and colons counts only
/// when it has `::` or at least five colons, so a time like `12:30:45` and a path like `std::vector` stay as they are.
fn mask_ipv6(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if (chars[i].is_ascii_hexdigit() || chars[i] == ':') && (i == 0 || !is_name(chars[i - 1])) {
            let mut j = i;
            while j < chars.len() && (chars[j].is_ascii_hexdigit() || chars[j] == ':') {
                j += 1;
            }
            let run: String = chars[i..j].iter().collect();
            let colons = run.matches(':').count();
            let clean_end = j >= chars.len() || !chars[j].is_ascii_alphanumeric();
            if clean_end && run.len() >= 3 && (run.contains("::") || colons >= 5) {
                out.push_str("[ip]");
                i = j;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Names from the vault that must not appear in a pasted report.
pub fn private_terms(conn: &Connection) -> AppResult<Vec<String>> {
    let mut terms = Vec::new();
    for sql in [
        "SELECT label FROM hosts",
        "SELECT address FROM hosts",
        "SELECT label FROM groups",
        "SELECT username FROM connection_profiles WHERE username IS NOT NULL",
        "SELECT label FROM identities",
        "SELECT username FROM identities",
    ] {
        let mut st = conn.prepare(sql)?;
        let rows = st.query_map([], |r| r.get::<_, String>(0))?;
        for r in rows {
            terms.push(r?);
        }
    }
    Ok(terms)
}

/// The last `n` lines of the newest `.log` file in `dir`.
pub fn tail_log(dir: &std::path::Path, n: usize) -> String {
    let newest = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "log"))
        .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok());
    let Some(file) = newest else {
        return "(no log file yet)".into();
    };
    let text = std::fs::read_to_string(file.path()).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipv6_addresses_are_removed_but_times_and_paths_stay() {
        let r = redact(
            "from fe80::1 and 2001:db8::7334 and [::1]:22 and 2001:0db8:0000:0000:0000:0000:0000:0001 at 12:30:45 in std::vector",
            None,
            &[],
        );
        assert_eq!(r.matches("[ip]").count(), 4, "{r}");
        assert!(r.contains("12:30:45") && r.contains("std::vector"), "{r}");
        assert!(!r.contains("fe80") && !r.contains("db8"), "{r}");
    }

    #[test]
    fn hostnames_users_ips_and_home_are_removed() {
        let line = "2026-11-05 connect ops@Web-1.Example.com (10.0.12.7:22) from /Users/me/Library/x as deploy; kex ok v1.2.3.4x";
        let terms = vec![
            "web-1.example.com".to_string(),
            "deploy".to_string(),
            "ab".to_string(),
        ];
        let r = redact(line, Some("/Users/me"), &terms);
        assert!(!r.contains("Web-1") && !r.contains("deploy") && !r.contains("10.0.12.7"));
        assert!(!r.contains("/Users/me") && r.contains("~/Library/x"));
        assert!(r.contains("[user]@[host]") && !r.contains("ops@"), "{r}");
        assert!(r.contains("[ip]:22"));
        assert!(r.contains("v1.2.3.4x")); // a version string isn't an address
        assert!(r.contains("kex ok")); // ordinary words stay
    }
}
