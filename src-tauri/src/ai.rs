//! "Ask AI" (could-have M4): turn a plain-language request into one shell command, using the user's
//! own Anthropic API key. Opt-in by nature: nothing is sent until a key is saved and a request is
//! made, and then only the request text and the server's OS name go out (no terminal output, host
//! names or files). The key is stored encrypted in `secrets` and never reaches the webview; the
//! HTTP call is made here in Rust. A suggestion is only ever typed into the terminal, never run.

use std::time::Duration;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::error::{AppError, AppResult, ErrorCode};
use crate::vault::secrets::{self, SecretKind};
use crate::vault::VaultSession;

pub const DEFAULT_BASE: &str = "https://api.anthropic.com";
pub const DEFAULT_MODEL: &str = "claude-sonnet-5-5";
const MAX_REQUEST_CHARS: usize = 1000;
const MAX_COMMAND_CHARS: usize = 2000;

#[derive(Debug, Serialize, Type)]
pub struct AiConfig {
    pub has_key: bool,
    pub model: String,
    /// Where requests go. Only change it for a gateway you run yourself.
    pub base_url: String,
}

#[derive(Debug, Deserialize, Type)]
pub struct AiSettings {
    pub model: String,
    pub base_url: String,
    /// A new key. `None` leaves the saved one alone.
    pub api_key: Option<String>,
    /// Removes the saved key.
    pub clear_key: bool,
}

#[derive(Debug, Serialize, Type)]
pub struct Suggestion {
    pub command: String,
    /// The model asked a question instead of giving a command (the text after `# `).
    pub is_question: bool,
}

/// The stored `ai` settings document: model, base URL and the id of the key's secret.
#[derive(Default, Serialize, Deserialize)]
struct Stored {
    model: Option<String>,
    base_url: Option<String>,
    key_secret_id: Option<String>,
}

fn load(conn: &Connection) -> Stored {
    crate::settings::get_all(conn)
        .ok()
        .and_then(|m| m.get("ai").and_then(|v| serde_json::from_str(v).ok()))
        .unwrap_or_default()
}

fn store(conn: &Connection, s: &Stored) -> AppResult<()> {
    crate::settings::set(
        conn,
        "ai",
        &serde_json::to_string(s).map_err(|e| AppError::internal(e.to_string()))?,
    )
}

pub fn get(conn: &Connection) -> AiConfig {
    let s = load(conn);
    AiConfig {
        has_key: s.key_secret_id.is_some(),
        model: s.model.unwrap_or_else(|| DEFAULT_MODEL.into()),
        base_url: s.base_url.unwrap_or_else(|| DEFAULT_BASE.into()),
    }
}

pub fn save(
    conn: &mut Connection,
    vault: &VaultSession,
    input: &AiSettings,
) -> AppResult<AiConfig> {
    let model = input.model.trim();
    if model.is_empty() || model.len() > 100 || model.chars().any(char::is_whitespace) {
        return Err(AppError::validation(
            "Enter a model name, like the ones in your provider's docs.",
        ));
    }
    let base = input.base_url.trim().trim_end_matches('/');
    // A key travels to this address, so it must be https (plain http only for this computer).
    let local = base.starts_with("http://localhost") || base.starts_with("http://127.0.0.1");
    if !(base.starts_with("https://") || local)
        || base.len() > 200
        || base.chars().any(char::is_whitespace)
    {
        return Err(AppError::validation(
            "The address must start with https://.",
        ));
    }
    let mut stored = load(conn);
    let tx = conn.transaction()?;
    if input.clear_key {
        if let Some(id) = stored.key_secret_id.take() {
            secrets::delete(&tx, vault, &id)?;
        }
    }
    if let Some(key) = input
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|k| !k.is_empty())
    {
        if key.len() > 400 || key.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(AppError::validation("That doesn't look like an API key."));
        }
        match &stored.key_secret_id {
            Some(id) => secrets::replace(&tx, vault, id, key.as_bytes())?,
            None => {
                stored.key_secret_id = Some(secrets::put(
                    &tx,
                    vault,
                    SecretKind::Password,
                    key.as_bytes(),
                )?)
            }
        }
    }
    stored.model = Some(model.to_string());
    stored.base_url = Some(base.to_string());
    store(&tx, &stored)?;
    tx.commit()?;
    Ok(get(conn))
}

/// What a request needs, read under the lock so the network call can happen after it.
pub struct Ready {
    key: zeroize::Zeroizing<String>,
    model: String,
    base: String,
}

pub fn ready(conn: &Connection, vault: &VaultSession) -> AppResult<Ready> {
    let s = load(conn);
    let id = s
        .key_secret_id
        .ok_or_else(|| AppError::validation("Add your API key in Settings, then AI."))?;
    let key = zeroize::Zeroizing::new(
        String::from_utf8_lossy(&secrets::get(conn, vault, &id)?).into_owned(),
    );
    Ok(Ready {
        key,
        model: s.model.unwrap_or_else(|| DEFAULT_MODEL.into()),
        base: s.base_url.unwrap_or_else(|| DEFAULT_BASE.into()),
    })
}

/// Strips code fences and chatter, keeping just the command. `# question` becomes a question.
pub fn clean(reply: &str) -> Option<Suggestion> {
    let text = reply.trim();
    let text = text.strip_prefix("```").map_or(text, |t| {
        t.trim_start_matches(|c: char| c.is_alphanumeric())
            .trim_start_matches('\n')
    });
    let text = text.split("```").next().unwrap_or("").trim();
    if text.is_empty()
        || text.len() > MAX_COMMAND_CHARS
        || text
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return None;
    }
    match text.strip_prefix("# ") {
        Some(q) => Some(Suggestion {
            command: q.trim().to_string(),
            is_question: true,
        }),
        None => Some(Suggestion {
            command: text.to_string(),
            is_question: false,
        }),
    }
}

/// Asks the model for one command. `os` is the server's OS name from OS detection, if known.
pub async fn suggest(r: &Ready, request: &str, os: Option<&str>) -> AppResult<Suggestion> {
    let request = request.trim();
    if request.is_empty() {
        return Err(AppError::validation("Describe what you want to do."));
    }
    if request.chars().count() > MAX_REQUEST_CHARS {
        return Err(AppError::validation(format!(
            "Keep it under {MAX_REQUEST_CHARS} characters."
        )));
    }
    let system = format!(
        "You write shell commands for a {} terminal. Reply with exactly one command (a pipeline or one-liner is fine) and nothing else: no explanation, no code fences. \
         If the request is unclear or would be destructive without more detail, reply with `# ` followed by one short question instead.",
        os.filter(|o| *o != "unknown").unwrap_or("Linux")
    );
    let body = serde_json::json!({
        "model": r.model,
        "max_tokens": 400,
        "system": system,
        "messages": [{ "role": "user", "content": request }],
    });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| AppError::internal(e.to_string()))?;
    let resp = client
        .post(format!("{}/v1/messages", r.base))
        .header("x-api-key", r.key.as_str())
        .header("anthropic-version", "2023-06-01")
        .json(&body)
        .send()
        .await
        .map_err(|e| {
            AppError::new(
                ErrorCode::HostUnreachable,
                format!("Couldn't reach the AI service: {}", e.without_url()),
            )
        })?;
    let status = resp.status();
    let json: serde_json::Value = resp.json().await.unwrap_or_default();
    if status.as_u16() == 401 || status.as_u16() == 403 {
        return Err(AppError::new(
            ErrorCode::AuthFailed,
            "The AI service refused your API key.",
        ));
    }
    if !status.is_success() {
        let msg = json["error"]["message"]
            .as_str()
            .unwrap_or("the request failed");
        return Err(AppError::internal(format!(
            "The AI service said: {}",
            msg.chars().take(200).collect::<String>()
        )));
    }
    let text = json["content"]
        .as_array()
        .and_then(|c| c.iter().find_map(|b| b["text"].as_str()))
        .unwrap_or("");
    clean(text).ok_or_else(|| {
        AppError::internal("The AI service didn't return a usable command. Try rephrasing.")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;
    use crate::vault::{self, kdf::KdfParams};
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn replies_are_cleaned_to_a_command_or_a_question() {
        assert_eq!(clean("df -h").unwrap().command, "df -h");
        assert_eq!(
            clean("```bash\nfind . -name '*.log' -mtime +7\n```")
                .unwrap()
                .command,
            "find . -name '*.log' -mtime +7"
        );
        assert_eq!(
            clean("```\nls\n```\nThis lists files.").unwrap().command,
            "ls"
        );
        let q = clean("# Which folder?").unwrap();
        assert!(q.is_question && q.command == "Which folder?");
        assert!(clean("   ").is_none());
        assert!(clean("rm -rf /\u{7}").is_none()); // control characters are refused
    }

    #[test]
    fn key_is_stored_encrypted_replaced_and_cleared() {
        let mut c = open_in_memory().unwrap();
        let s = vault::create(&c, "correct-horse-staple", KdfParams::cheap()).unwrap();
        let input = |key: Option<&str>, clear: bool| AiSettings {
            model: "m1".into(),
            base_url: "https://example.test/".into(),
            api_key: key.map(String::from),
            clear_key: clear,
        };
        assert!(!get(&c).has_key && get(&c).model == DEFAULT_MODEL);
        let cfg = save(&mut c, &s, &input(Some("sk-test-123"), false)).unwrap();
        assert!(cfg.has_key && cfg.base_url == "https://example.test"); // slash trimmed
        let raw: Vec<u8> = c
            .query_row("SELECT ciphertext FROM secrets", [], |r| r.get(0))
            .unwrap();
        assert!(!raw.windows(7).any(|w| w == b"sk-test")); // not plaintext
        assert_eq!(&*ready(&c, &s).unwrap().key, "sk-test-123");
        save(&mut c, &s, &input(Some("sk-new"), false)).unwrap(); // replaced in place
        assert_eq!(
            c.query_row::<i64, _, _>("SELECT count(*) FROM secrets", [], |r| r.get(0))
                .unwrap(),
            1
        );
        assert_eq!(&*ready(&c, &s).unwrap().key, "sk-new");
        save(&mut c, &s, &input(None, false)).unwrap(); // keeping the key when none is given
        assert!(get(&c).has_key);
        assert!(!save(&mut c, &s, &input(None, true)).unwrap().has_key);
        assert!(ready(&c, &s).is_err());
        assert!(save(
            &mut c,
            &s,
            &AiSettings {
                base_url: "http://evil.test".into(),
                ..input(None, false)
            }
        )
        .is_err()); // key over plain http
        assert!(save(
            &mut c,
            &s,
            &AiSettings {
                model: "two words".into(),
                ..input(None, false)
            }
        )
        .is_err());
    }

    #[tokio::test]
    async fn request_carries_the_key_and_only_the_request_text() {
        // A tiny HTTP server standing in for the API.
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        let seen: Arc<Mutex<String>> = Arc::default();
        let s2 = Arc::clone(&seen);
        tokio::spawn(async move {
            for reply in [
                "{\"content\":[{\"type\":\"text\",\"text\":\"```\\ndu -sh *\\n```\"}]}",
                "{\"error\":{\"message\":\"bad key\"}}",
            ] {
                let (mut c, _) = l.accept().await.unwrap();
                let mut buf = vec![0u8; 16384];
                let mut n = 0;
                loop {
                    n += c.read(&mut buf[n..]).await.unwrap();
                    let text = String::from_utf8_lossy(&buf[..n]).into_owned();
                    if let Some((head, body)) = text.split_once("\r\n\r\n") {
                        let len: usize = head
                            .lines()
                            .find_map(|h| {
                                h.to_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse().unwrap())
                            })
                            .unwrap_or(0);
                        if body.len() >= len {
                            s2.lock().unwrap().push_str(&text);
                            break;
                        }
                    }
                }
                let status = if reply.contains("error") {
                    "401 Unauthorized"
                } else {
                    "200 OK"
                };
                c.write_all(format!("HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reply}", reply.len()).as_bytes()).await.unwrap();
            }
        });
        let r = Ready {
            key: zeroize::Zeroizing::new("sk-test-123".into()),
            model: "m1".into(),
            base,
        };
        let s = suggest(&r, "how big is each folder here", Some("ubuntu"))
            .await
            .unwrap();
        assert_eq!((s.command.as_str(), s.is_question), ("du -sh *", false));
        let req = seen.lock().unwrap().clone().to_lowercase();
        assert!(
            req.contains("post /v1/messages")
                && req.contains("x-api-key: sk-test-123")
                && req.contains("anthropic-version")
        );
        assert!(
            req.contains("how big is each folder here")
                && req.contains("ubuntu")
                && req.contains("\"model\":\"m1\"")
        );
        // A refused key is a clear error.
        let err = suggest(&r, "anything", None).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::AuthFailed);
        assert!(suggest(&r, "   ", None).await.is_err());
    }
}
