//! Importing from other SSH clients: PuTTY (a `.reg` export of its sessions, or a session file from
//! `~/.putty/sessions`) and MobaXterm (a `.mobaconf` / `.ini` export). Only the basics come across:
//! name, host, port, user and, for MobaXterm, the folder. Passwords and key files are never read.

use super::{ImportPreview, ImportRow};

/// `.reg` exports are UTF-16 with a byte order mark; most other files are UTF-8.
pub fn decode_text(bytes: &[u8]) -> Option<String> {
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        let units: Vec<u16> = rest
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        return String::from_utf16(&units).ok();
    }
    let text = String::from_utf8_lossy(bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes))
        .into_owned();
    // A file full of replacement characters isn't text we understand.
    (text.matches('\u{FFFD}').count() * 20 < text.len().max(1)).then_some(text)
}

/// `My%20Server` -> `My Server` (PuTTY escapes session names like this).
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let (mut out, mut i) = (Vec::with_capacity(b.len()), 0);
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            // Hex digits from the bytes, not a string slice: a slice could cut a multi-byte character.
            if let (Some(h), Some(l)) = (
                (b[i + 1] as char).to_digit(16),
                (b[i + 2] as char).to_digit(16),
            ) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[derive(Default)]
struct PuttySession {
    name: String,
    host: String,
    port: Option<u16>,
    user: String,
    protocol: String,
    key_file: bool,
    proxy: bool,
}

impl PuttySession {
    fn set(&mut self, key: &str, value: &str) {
        match key {
            "HostName" => self.host = value.to_string(),
            "PortNumber" => self.port = value.parse().ok().filter(|p| *p > 0),
            "UserName" => self.user = value.to_string(),
            "Protocol" => self.protocol = value.to_lowercase(),
            "PublicKeyFile" => self.key_file = !value.is_empty(),
            "ProxyMethod" => self.proxy = value != "0" && !value.is_empty(),
            _ => {}
        }
    }
}

/// A `.reg` value: `"text"` (with `\\` and `\"` escapes) or `dword:0000001a`.
fn reg_value(v: &str) -> Option<String> {
    let v = v.trim();
    if let Some(hex) = v.strip_prefix("dword:") {
        return u32::from_str_radix(hex, 16).ok().map(|n| n.to_string());
    }
    let inner = v.strip_prefix('"')?.strip_suffix('"')?;
    Some(
        inner
            .replace("\\\\", "\u{0}")
            .replace("\\\"", "\"")
            .replace('\u{0}', "\\"),
    )
}

pub fn parse_putty(text: &str) -> ImportPreview {
    let mut sessions: Vec<PuttySession> = Vec::new();
    let is_reg = text.lines().any(|l| l.trim_start().starts_with("[HKEY_"));
    if is_reg {
        let mut current: Option<PuttySession> = None;
        for line in text.lines().map(str::trim) {
            if let Some(path) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                sessions.extend(current.take());
                // Only keys under ...\PuTTY\Sessions\<name> are sessions.
                let name = path
                    .rsplit_once("\\Sessions\\")
                    .map(|(_, n)| percent_decode(n))
                    .filter(|n| !n.contains('\\'));
                current = name.map(|name| PuttySession {
                    name,
                    ..Default::default()
                });
            } else if let (Some(s), Some((k, v))) = (current.as_mut(), line.split_once('=')) {
                if let Some(v) = reg_value(v) {
                    s.set(k.trim().trim_matches('"'), &v);
                }
            }
        }
        sessions.extend(current);
    } else {
        // A session file from ~/.putty/sessions: `Key=Value` lines, one session, no name inside.
        let mut s = PuttySession::default();
        for line in text.lines() {
            if let Some((k, v)) = line.split_once('=') {
                s.set(k.trim(), v.trim());
            }
        }
        if !s.host.is_empty() {
            s.name = s.host.clone();
            sessions.push(s);
        }
    }

    let (mut rows, mut warnings, mut skipped) = (Vec::new(), Vec::new(), 0);
    for s in sessions {
        if s.name.eq_ignore_ascii_case("Default Settings") || s.host.is_empty() {
            continue; // PuTTY's template and sessions with no host aren't servers
        }
        if !s.protocol.is_empty() && s.protocol != "ssh" {
            skipped += 1;
            continue;
        }
        let mut row = ImportRow {
            label: s.name,
            address: s.host,
            port: s.port,
            username: Some(s.user).filter(|u| !u.is_empty()),
            ..Default::default()
        };
        if s.key_file {
            row.warnings
                .push("The key file isn't imported. Add it in Keychain.".into());
        }
        if s.proxy {
            row.warnings
                .push("The proxy settings aren't imported. Set a proxy on the host.".into());
        }
        rows.push(row);
    }
    if skipped > 0 {
        warnings.push(format!(
            "{skipped} session(s) that aren't SSH (serial, Telnet, raw) were skipped."
        ));
    }
    if rows.is_empty() {
        warnings.push("No SSH sessions found.".into());
    }
    ImportPreview { rows, warnings }
}

/// MobaXterm bookmarks: `[Bookmarks]` / `[Bookmarks_1]` sections, each with `SubRep=<folder>` and
/// lines like `web-1=#109#0%10.0.0.5%22%deploy%%-1%-1%...` (`#109#` is the SSH session type).
pub fn parse_mobaxterm(text: &str) -> ImportPreview {
    let (mut rows, mut other) = (Vec::new(), 0);
    let mut folder: Option<String> = None;
    let mut in_bookmarks = false;
    for line in text.lines().map(str::trim) {
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            in_bookmarks = name.starts_with("Bookmarks");
            folder = None;
            continue;
        }
        if !in_bookmarks || line.is_empty() || line.starts_with(';') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key {
            // The folder, `Work\Prod` for nested ones: the last name is used as the group.
            "SubRep" => {
                folder = Some(
                    value
                        .rsplit(['\\', '/'])
                        .next()
                        .unwrap_or("")
                        .trim()
                        .to_string(),
                )
                .filter(|f| !f.is_empty())
            }
            "ImgNum" => {}
            _ => {
                let Some(ssh) = value.strip_prefix("#109#") else {
                    other += 1;
                    continue;
                };
                // After the type digit: host % port % user % ...
                let mut parts = ssh.splitn(5, '%');
                let _kind = parts.next();
                let (host, port, user) = (
                    parts.next().unwrap_or("").trim(),
                    parts.next().unwrap_or("").trim(),
                    parts.next().unwrap_or("").trim(),
                );
                if host.is_empty() {
                    continue;
                }
                rows.push(ImportRow {
                    label: key.trim().to_string(),
                    address: host.to_string(),
                    port: port.parse().ok().filter(|p| *p > 0),
                    username: Some(user.to_string()).filter(|u| !u.is_empty() && u != "<default>"),
                    group: folder.clone(),
                    ..Default::default()
                });
            }
        }
    }
    let mut warnings = Vec::new();
    if other > 0 {
        warnings.push(format!(
            "{other} bookmark(s) that aren't SSH (RDP, VNC, Telnet, ...) were skipped."
        ));
    }
    if rows.is_empty() {
        warnings.push("No SSH bookmarks found.".into());
    }
    ImportPreview { rows, warnings }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn putty_reg_export_utf16_and_session_files() {
        let reg = "Windows Registry Editor Version 5.00\r\n\r\n[HKEY_CURRENT_USER\\Software\\SimonTatham\\PuTTY\\Sessions\\Default%20Settings]\r\n\"HostName\"=\"\"\r\n\r\n[HKEY_CURRENT_USER\\Software\\SimonTatham\\PuTTY\\Sessions\\Web%20Server%201]\r\n\"HostName\"=\"web1.example.com\"\r\n\"Protocol\"=\"ssh\"\r\n\"PortNumber\"=dword:000008ae\r\n\"UserName\"=\"deploy\"\r\n\"PublicKeyFile\"=\"C:\\\\keys\\\\id.ppk\"\r\n\"ProxyMethod\"=dword:00000002\r\n\r\n[HKEY_CURRENT_USER\\Software\\SimonTatham\\PuTTY\\Sessions\\router]\r\n\"HostName\"=\"10.0.0.1\"\r\n\"Protocol\"=\"telnet\"\r\n\r\n[HKEY_CURRENT_USER\\Software\\SimonTatham\\PuTTY\\SshHostKeys]\r\n\"rsa2@22:x\"=\"0x1\"\r\n";
        // The file as regedit writes it: UTF-16LE with a BOM.
        let mut bytes = vec![0xFF, 0xFE];
        bytes.extend(reg.encode_utf16().flat_map(u16::to_le_bytes));
        let text = decode_text(&bytes).unwrap();
        let p = parse_putty(&text);
        assert_eq!(p.rows.len(), 1);
        let r = &p.rows[0];
        assert_eq!(
            (
                r.label.as_str(),
                r.address.as_str(),
                r.port,
                r.username.as_deref()
            ),
            (
                "Web Server 1",
                "web1.example.com",
                Some(2222),
                Some("deploy")
            )
        );
        assert_eq!(r.warnings.len(), 2); // key file, proxy
        assert!(
            p.warnings.iter().any(|w| w.contains("1 session")),
            "{:?}",
            p.warnings
        ); // the Telnet one
           // A plain session file.
        let s =
            parse_putty("HostName=db.example.com\nPortNumber=22\nUserName=root\nProtocol=ssh\n");
        assert_eq!(
            (s.rows[0].address.as_str(), s.rows[0].username.as_deref()),
            ("db.example.com", Some("root"))
        );
        assert!(parse_putty("nothing useful").rows.is_empty());
    }

    #[test]
    fn mobaxterm_bookmarks_folders_and_other_types() {
        let ini = "[Misc]\nfoo=bar\n\n[Bookmarks]\nSubRep=\nImgNum=42\nlocal-nas=#109#0%192.168.1.9%22%admin%%-1%-1%%%%%0%0%0%%%-1%0%0%0%%1080%%0%0%1%%0%%%%0%-1%-1\n\n[Bookmarks_1]\nSubRep=Work\\Prod\nImgNum=41\nweb-1=#109#0%10.0.0.5%2200%<default>%%-1\ndesktop=#91#4%10.0.0.7%3389%\nempty=#109#0%%22%x\n";
        let p = parse_mobaxterm(ini);
        assert_eq!(p.rows.len(), 2);
        assert_eq!(
            (
                p.rows[0].label.as_str(),
                p.rows[0].address.as_str(),
                p.rows[0].port,
                p.rows[0].username.as_deref(),
                p.rows[0].group.as_deref()
            ),
            ("local-nas", "192.168.1.9", Some(22), Some("admin"), None)
        );
        assert_eq!(
            (
                p.rows[1].label.as_str(),
                p.rows[1].port,
                p.rows[1].username.as_deref(),
                p.rows[1].group.as_deref()
            ),
            ("web-1", Some(2200), None, Some("Prod"))
        );
        assert!(
            p.warnings.iter().any(|w| w.contains("1 bookmark")),
            "{:?}",
            p.warnings
        ); // the RDP one
    }

    #[test]
    fn percent_escapes_and_bad_bytes() {
        assert_eq!(percent_decode("My%20Server%2F1"), "My Server/1");
        assert_eq!(percent_decode("50%"), "50%");
        assert_eq!(percent_decode("a%é"), "a%é"); // not hex, and no panic on a multi-byte character
        assert!(decode_text(&[0xC3, 0x28, 0xC3, 0x28, 0xC3, 0x28]).is_none());
    }
}
