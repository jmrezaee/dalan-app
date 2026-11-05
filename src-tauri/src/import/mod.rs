//! Import (S25, F10): hosts from an OpenSSH `config` file or a CSV file.
//!
//! Two steps, so nothing is written until the user has seen the list:
//! 1. `preview_*` parses text into `ImportRow`s (pure functions, easy to test) and flags duplicates.
//! 2. `apply` saves the rows the user ticked, creating groups and jump chains.
//!
//! The webview only picks a file path; Rust reads the file (hard rule 2).

mod clients;

use std::collections::{HashMap, HashSet};

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::error::{AppError, AppResult};
use crate::groups::{self, GroupInput};
use crate::hosts::{self, HostInput, DEFAULT_PORT};
use crate::vault::VaultSession;

/// A file bigger than this is not a config or host list.
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_ROWS: usize = 5000;

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Default)]
pub struct ImportRow {
    pub label: String,
    pub address: String,
    pub port: Option<u16>,
    pub username: Option<String>,
    /// CSV only: a group name, created at the top level when it doesn't exist yet.
    pub group: Option<String>,
    pub tags: Vec<String>,
    /// ssh_config only: `ProxyJump` names, matched to host labels or addresses.
    pub jump: Vec<String>,
    /// The vault already has a host with this address, port and user. Off by default in the UI.
    pub duplicate: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize, Type)]
pub struct ImportPreview {
    pub rows: Vec<ImportRow>,
    /// File-level notes (ignored `Include`, unreadable lines).
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize, Type)]
pub struct ImportResult {
    pub created: u32,
    pub skipped: u32,
}

// ------------------------------------------------------------------ ssh_config

/// `*` and `?` wildcards, as in `ssh_config` patterns.
fn glob(pat: &[char], s: &[char]) -> bool {
    match (pat.first(), s.first()) {
        (None, None) => true,
        (Some('*'), _) => glob(&pat[1..], s) || (!s.is_empty() && glob(pat, &s[1..])),
        (Some('?'), Some(_)) => glob(&pat[1..], &s[1..]),
        (Some(p), Some(c)) if p.eq_ignore_ascii_case(c) => glob(&pat[1..], &s[1..]),
        _ => false,
    }
}

/// A `Host` line matches when one pattern matches and no `!negated` pattern does.
fn host_matches(patterns: &[String], alias: &str) -> bool {
    let a: Vec<char> = alias.chars().collect();
    let hit = |p: &str| glob(&p.chars().collect::<Vec<_>>(), &a);
    let mut any = false;
    for p in patterns {
        if let Some(neg) = p.strip_prefix('!') {
            if hit(neg) {
                return false;
            }
        } else if hit(p) {
            any = true;
        }
    }
    any
}

struct Block {
    patterns: Vec<String>,
    /// (lowercase keyword, value) in file order. The first value for a keyword wins, like OpenSSH.
    opts: Vec<(String, String)>,
}

/// Splits `Keyword value` or `Keyword=value`. Values may be quoted.
fn split_line(line: &str) -> Option<(String, String)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let end = line.find(|c: char| c.is_whitespace() || c == '=')?;
    let key = line[..end].to_lowercase();
    let rest = line[end..].trim_start_matches(|c: char| c.is_whitespace() || c == '=');
    let value = rest.trim().trim_matches('"').to_string();
    (!value.is_empty()).then_some((key, value))
}

pub fn parse_ssh_config(text: &str) -> ImportPreview {
    let mut warnings = Vec::new();
    // Options before the first `Host` apply to everything.
    let mut blocks = vec![Block {
        patterns: vec!["*".into()],
        opts: vec![],
    }];
    let (mut saw_include, mut saw_match) = (false, false);
    for line in text.lines() {
        let Some((key, value)) = split_line(line) else {
            continue;
        };
        match key.as_str() {
            "host" => blocks.push(Block {
                patterns: value.split_whitespace().map(String::from).collect(),
                opts: vec![],
            }),
            "match" => {
                // Conditions can't be evaluated here, so the block never applies.
                saw_match = true;
                blocks.push(Block {
                    patterns: vec![],
                    opts: vec![],
                });
            }
            "include" => saw_include = true,
            _ => blocks
                .last_mut()
                .expect("blocks is never empty")
                .opts
                .push((key, value)),
        }
    }
    if saw_include {
        warnings.push("Include lines were not followed. Import those files separately.".into());
    }
    if saw_match {
        warnings.push("Match blocks were skipped.".into());
    }

    // Hosts are the concrete names on `Host` lines; wildcard lines only supply defaults.
    let mut seen = HashSet::new();
    let mut rows = Vec::new();
    for alias in blocks
        .iter()
        .flat_map(|b| b.patterns.iter())
        .filter(|p| !p.contains(['*', '?', '!']))
    {
        if !seen.insert(alias.to_lowercase()) {
            continue;
        }
        let get = |key: &str| -> Option<&str> {
            blocks
                .iter()
                .filter(|b| host_matches(&b.patterns, alias))
                .flat_map(|b| b.opts.iter())
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.as_str())
        };
        let mut row = ImportRow {
            label: alias.clone(),
            address: get("hostname").map_or_else(|| alias.clone(), |h| h.replace("%h", alias)),
            username: get("user").map(String::from),
            ..Default::default()
        };
        if let Some(p) = get("port") {
            match p.parse::<u16>().ok().filter(|p| *p > 0) {
                Some(p) => row.port = Some(p),
                None => row
                    .warnings
                    .push(format!("Port “{p}” isn't valid, so 22 is used.")),
            }
        }
        if get("proxycommand").is_some() {
            row.warnings
                .push("ProxyCommand isn't supported and was ignored.".into());
        }
        if let Some(j) = get("proxyjump") {
            if !j.eq_ignore_ascii_case("none") {
                // `user@host:port,other` -> host names only; sign-in details come from each host.
                row.jump = j
                    .split(',')
                    .map(|h| {
                        let h = h.rsplit('@').next().unwrap_or(h);
                        h.split(':').next().unwrap_or(h).trim().to_string()
                    })
                    .filter(|h| !h.is_empty())
                    .collect();
            }
        }
        if get("identityfile").is_some() {
            row.warnings
                .push("The key file isn't imported. Add it in Keychain.".into());
        }
        rows.push(row);
    }
    if rows.is_empty() {
        warnings
            .push("No hosts found. Only named Host entries are imported, not wildcards.".into());
    }
    ImportPreview { rows, warnings }
}

// ------------------------------------------------------------------ CSV

/// Minimal RFC 4180 reader: quoted fields, doubled quotes, commas and newlines inside quotes.
fn csv_records(text: &str) -> Vec<Vec<String>> {
    let (mut out, mut rec, mut field) = (Vec::new(), Vec::new(), String::new());
    let (mut quoted, mut chars) = (
        false,
        text.trim_start_matches('\u{feff}').chars().peekable(),
    );
    while let Some(c) = chars.next() {
        match (c, quoted) {
            ('"', true) if chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            ('"', _) => quoted = !quoted,
            (',', false) => rec.push(std::mem::take(&mut field)),
            ('\r', false) => {}
            ('\n', false) => {
                rec.push(std::mem::take(&mut field));
                out.push(std::mem::take(&mut rec));
            }
            _ => field.push(c),
        }
    }
    if !field.is_empty() || !rec.is_empty() {
        rec.push(field);
        out.push(rec);
    }
    out.retain(|r| r.iter().any(|f| !f.trim().is_empty()));
    out
}

pub fn parse_csv(text: &str) -> AppResult<ImportPreview> {
    let mut records = csv_records(text).into_iter();
    let header: Vec<String> = records
        .next()
        .ok_or_else(|| AppError::validation("The file is empty."))?
        .iter()
        .map(|h| h.trim().to_lowercase())
        .collect();
    let col = |names: &[&str]| header.iter().position(|h| names.contains(&h.as_str()));
    let address_col = col(&["address", "host", "hostname", "ip"]).ok_or_else(|| {
        AppError::validation("The first row needs an “address” column (label, port, username, group and tags are optional).")
    })?;
    let (label_col, port_col, user_col, group_col, tags_col) = (
        col(&["label", "name"]),
        col(&["port"]),
        col(&["username", "user"]),
        col(&["group", "folder"]),
        col(&["tags", "tag"]),
    );
    let mut warnings = Vec::new();
    let mut rows = Vec::new();
    for (i, rec) in records.enumerate() {
        let cell = |c: Option<usize>| {
            c.and_then(|c| rec.get(c))
                .map(|v| v.trim())
                .filter(|v| !v.is_empty())
        };
        let Some(address) = cell(Some(address_col)) else {
            warnings.push(format!("Row {} has no address and was skipped.", i + 2));
            continue;
        };
        let mut row = ImportRow {
            label: cell(label_col).unwrap_or(address).to_string(),
            address: address.to_string(),
            username: cell(user_col).map(String::from),
            group: cell(group_col).map(String::from),
            tags: cell(tags_col)
                .map(|t| {
                    t.split([';', '|'])
                        .map(|t| t.trim().to_string())
                        .filter(|t| !t.is_empty())
                        .collect()
                })
                .unwrap_or_default(),
            ..Default::default()
        };
        if let Some(p) = cell(port_col) {
            match p.parse::<u16>().ok().filter(|p| *p > 0) {
                Some(p) => row.port = Some(p),
                None => row
                    .warnings
                    .push(format!("Port “{p}” isn't valid, so 22 is used.")),
            }
        }
        rows.push(row);
    }
    Ok(ImportPreview { rows, warnings })
}

// ------------------------------------------------------------------ preview and apply

/// Reads a chosen file. `None` means the default `~/.ssh/config`.
pub fn read_file(kind: &str, path: Option<&str>) -> AppResult<String> {
    let path = match path {
        Some(p) => std::path::PathBuf::from(p),
        None if kind == "ssh_config" => {
            let home = std::env::var("HOME")
                .or_else(|_| std::env::var("USERPROFILE"))
                .map_err(|_| AppError::not_found("Couldn't find your home folder."))?;
            std::path::Path::new(&home).join(".ssh").join("config")
        }
        None => return Err(AppError::validation("Choose a file first.")),
    };
    let meta = std::fs::metadata(&path)
        .map_err(|_| AppError::not_found(format!("Couldn't open {}.", path.display())))?;
    if meta.len() > MAX_FILE_BYTES {
        return Err(AppError::validation(
            "That file is too big to be a host list.",
        ));
    }
    let bytes = std::fs::read(&path)
        .map_err(|_| AppError::not_found(format!("Couldn't open {}.", path.display())))?;
    // PuTTY's `.reg` exports are UTF-16, so every kind goes through the same decoder.
    clients::decode_text(&bytes)
        .ok_or_else(|| AppError::validation("That file isn't text we can read."))
}

/// Parses `text` and marks rows that already exist in the vault.
pub fn preview(
    conn: &Connection,
    vault_id: &str,
    kind: &str,
    text: &str,
) -> AppResult<ImportPreview> {
    let mut p = match kind {
        "ssh_config" => parse_ssh_config(text),
        "csv" => parse_csv(text)?,
        "putty" => clients::parse_putty(text),
        "mobaxterm" => clients::parse_mobaxterm(text),
        _ => return Err(AppError::validation("Unknown import type.")),
    };
    if p.rows.len() > MAX_ROWS {
        return Err(AppError::validation(format!(
            "Import up to {MAX_ROWS} hosts at a time."
        )));
    }
    let tree = hosts::tree(conn, vault_id)?;
    let key = |a: &str, port: Option<u16>, u: Option<&str>| {
        (
            a.to_lowercase(),
            port.unwrap_or(DEFAULT_PORT),
            u.map(str::to_string),
        )
    };
    let existing: HashSet<_> = tree
        .hosts
        .iter()
        .map(|h| key(&h.address, h.port, h.username.as_deref()))
        .collect();
    let known_names: HashSet<String> = tree
        .hosts
        .iter()
        .flat_map(|h| [h.label.to_lowercase(), h.address.to_lowercase()])
        .chain(p.rows.iter().map(|r| r.label.to_lowercase()))
        .collect();
    for r in &mut p.rows {
        r.duplicate = existing.contains(&key(&r.address, r.port, r.username.as_deref()));
        for j in &r.jump {
            if !known_names.contains(&j.to_lowercase()) {
                r.warnings.push(format!(
                    "Jump host “{j}” isn't in your list, so this host connects directly."
                ));
            }
        }
    }
    Ok(p)
}

/// Saves the rows the user ticked. A row that fails validation is skipped, the rest still go in.
/// `group_id` is where rows without their own group land.
pub fn apply(
    conn: &mut Connection,
    s: &VaultSession,
    rows: &[ImportRow],
    group_id: Option<&str>,
) -> AppResult<ImportResult> {
    if rows.len() > MAX_ROWS {
        return Err(AppError::validation(format!(
            "Import up to {MAX_ROWS} hosts at a time."
        )));
    }
    // Top-level groups by lowercase name, so two rows naming "Prod" share one group.
    let mut top: HashMap<String, String> = groups::list(conn, &s.vault_id)?
        .into_iter()
        .filter(|g| g.parent_id.is_none())
        .map(|g| (g.label.to_lowercase(), g.id))
        .collect();
    let (mut created, mut skipped) = (0, 0);
    // label/address -> host id, for ProxyJump. Existing hosts first, then each new one.
    let mut by_name: HashMap<String, String> = HashMap::new();
    for h in hosts::tree(conn, &s.vault_id)?.hosts {
        by_name
            .entry(h.label.to_lowercase())
            .or_insert_with(|| h.id.clone());
        by_name.entry(h.address.to_lowercase()).or_insert(h.id);
    }
    let mut new_ids: Vec<Option<String>> = Vec::with_capacity(rows.len());
    for r in rows {
        let gid = match r.group.as_deref().map(str::trim).filter(|g| !g.is_empty()) {
            Some(name) => match top.get(&name.to_lowercase()) {
                Some(id) => Some(id.clone()),
                None => match groups::save(conn, s, &new_group(name)) {
                    Ok(g) => {
                        top.insert(name.to_lowercase(), g.id.clone());
                        Some(g.id)
                    }
                    Err(_) => None,
                },
            },
            None => group_id.map(String::from),
        };
        let input = HostInput {
            id: None,
            label: r.label.clone(),
            address: r.address.clone(),
            port: r.port,
            username: r.username.clone(),
            password: None,
            clear_password: false,
            group_id: gid,
            key_id: None,
            identity_id: None,
            startup_snippet_id: None,
            jump_mode: "inherit".into(),
            jump_host_ids: vec![],
            tags: r.tags.clone(),
        };
        match hosts::save(conn, s, &input) {
            Ok(h) => {
                created += 1;
                by_name.insert(r.label.to_lowercase(), h.id.clone());
                by_name
                    .entry(r.address.to_lowercase())
                    .or_insert_with(|| h.id.clone());
                new_ids.push(Some(h.id));
            }
            Err(_) => {
                skipped += 1;
                new_ids.push(None);
            }
        }
    }
    // Second pass: every host exists now, so chains can point at hosts from the same file.
    for (r, id) in rows.iter().zip(&new_ids) {
        let Some(id) = id else { continue };
        let hops: Vec<String> = r
            .jump
            .iter()
            .filter_map(|j| by_name.get(&j.to_lowercase()))
            .filter(|h| *h != id)
            .cloned()
            .collect();
        if hops.is_empty() {
            continue;
        }
        let tx = conn.transaction()?;
        let profile_id: String = tx.query_row(
            "SELECT profile_id FROM hosts WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )?;
        tx.execute(
            "UPDATE connection_profiles SET chain_mode = 'custom' WHERE id = ?1",
            params![profile_id],
        )?;
        groups::write_jumps(&tx, &profile_id, &hops)?;
        tx.commit()?;
    }
    Ok(ImportResult { created, skipped })
}

fn new_group(label: &str) -> GroupInput {
    GroupInput {
        id: None,
        parent_id: None,
        label: label.to_string(),
        port: None,
        username: None,
        password: None,
        clear_password: false,
        key_id: None,
        identity_id: None,
        startup_snippet_id: None,
        jump_mode: "inherit".into(),
        jump_host_ids: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_config_defaults_wildcards_and_jumps() {
        let p = parse_ssh_config(
            "Host web-1 web-2\n  HostName 10.0.0.%h\n  Port 2200\nHost db\n  HostName db.example.com\n  User admin\n  ProxyJump ops@web-1:22\n  ProxyCommand nc %h %p\n  IdentityFile ~/.ssh/id\nHost *\n  User deploy\n  Port 22\nHost *.internal !skip.internal\n  User internal\nInclude other\nMatch host x\n  User nope\n",
        );
        assert_eq!(p.rows.len(), 3);
        let web = &p.rows[0];
        assert_eq!(web.label, "web-1");
        assert_eq!(web.address, "10.0.0.web-1");
        assert_eq!(web.port, Some(2200)); // first value wins over `Host *`
        assert_eq!(web.username.as_deref(), Some("deploy")); // default from `Host *`
        let db = &p.rows[2];
        assert_eq!(db.username.as_deref(), Some("admin"));
        assert_eq!(db.jump, vec!["web-1"]);
        assert_eq!(db.warnings.len(), 2); // ProxyCommand, IdentityFile
        assert_eq!(p.warnings.len(), 2); // Include, Match
    }

    #[test]
    fn csv_quotes_columns_and_bad_rows() {
        let p = parse_csv(
            "Name,Host,Port,User,Group,Tags\n\"Web, one\",1.2.3.4,2222,root,Prod,a;b\n,5.6.7.8,x,,,\nbad,,,,,\n",
        )
        .unwrap();
        assert_eq!(p.rows.len(), 2);
        assert_eq!(p.rows[0].label, "Web, one");
        assert_eq!(p.rows[0].port, Some(2222));
        assert_eq!(p.rows[0].group.as_deref(), Some("Prod"));
        assert_eq!(p.rows[0].tags, vec!["a", "b"]);
        assert_eq!(p.rows[1].label, "5.6.7.8"); // label falls back to the address
        assert_eq!(p.rows[1].warnings.len(), 1); // bad port
        assert_eq!(p.warnings.len(), 1); // row without address
        assert!(parse_csv("name,port\nx,1\n").is_err());
    }

    #[test]
    fn apply_creates_groups_tags_jumps_and_flags_duplicates() {
        use crate::db::open_in_memory;
        use crate::vault::{self, kdf::KdfParams};
        let mut c = open_in_memory().unwrap();
        let s = vault::create(&c, "correct-horse-staple", KdfParams::cheap()).unwrap();
        let text = "label,address,port,user,group,tags\nbastion,1.1.1.1,,ops,Prod,x\nweb,2.2.2.2,2200,,Prod,\n";
        let mut rows = preview(&c, &s.vault_id, "csv", text).unwrap().rows;
        rows[1].jump = vec!["bastion".into(), "ghost".into()];
        let r = apply(&mut c, &s, &rows, None).unwrap();
        assert_eq!((r.created, r.skipped), (2, 0));
        let tree = hosts::tree(&c, &s.vault_id).unwrap();
        assert_eq!(tree.groups.len(), 1); // "Prod" shared by both rows
        let web = tree.hosts.iter().find(|h| h.label == "web").unwrap();
        assert_eq!(web.jump_mode, "custom");
        assert_eq!(web.jump_host_ids.len(), 1); // ghost dropped
        assert_eq!(web.group_id, Some(tree.groups[0].id.clone()));
        // Importing the same file again marks everything as a duplicate.
        assert!(preview(&c, &s.vault_id, "csv", text)
            .unwrap()
            .rows
            .iter()
            .all(|r| r.duplicate));
    }
}
