//! A session's title: its first prompt, read once from the harness's own
//! session log. Pure parsing (`first_prompt`) plus a small, fail-soft I/O
//! wrapper (`first_prompt_for`) that never errors and never panics -- a
//! title that cannot be found just stays `None`, and the caller shows the
//! short id instead. Deliberately does not import the usage plugin's own
//! JSONL reader: a title must never depend on a plugin being enabled.

use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use serde_json::{Map, Value};

/// How many characters a title keeps before it is cut short with `…`.
const TITLE_LIMIT: usize = 80;

/// Bytes read from the front of a session log: enough to hold the user's
/// first prompt without loading a transcript that can grow unbounded.
const HEAD_BYTES: u64 = 64 * 1024;

/// The first line that reads as a user record, trimmed to a title.
///
/// A line is a candidate when it parses as a JSON object whose `type` is
/// `"user"`, or whose `message.role` is `"user"`; its text is
/// `message.content` as a string, else the first `message.content` array
/// item whose `type` is `"text"` (that item's `text`), else top-level
/// `content` as a string. A candidate with no text, or any line that
/// fails to parse or is not an object, is skipped rather than stopping
/// the search. Whitespace is collapsed and the result trimmed; past
/// [`TITLE_LIMIT`] characters it is cut with a trailing `…`. `None` when
/// no line qualifies.
#[must_use]
pub fn first_prompt(jsonl_head: &str) -> Option<String> {
    for line in jsonl_head.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(record) = value.as_object() else {
            continue;
        };
        if !is_user_record(record) {
            continue;
        }
        let Some(text) = extract_text(record) else {
            continue;
        };
        if let Some(title) = normalize(&text) {
            return Some(title);
        }
    }
    None
}

/// Whether a parsed record is a user turn: `type == "user"`, or nested
/// under `message.role == "user"` (assistant records carry the latter
/// under `"assistant"`, so either check alone would miss a shape).
fn is_user_record(record: &Map<String, Value>) -> bool {
    record.get("type").and_then(Value::as_str) == Some("user")
        || record
            .get("message")
            .and_then(|m| m.get("role"))
            .and_then(Value::as_str)
            == Some("user")
}

/// A user record's text, tried in order: `message.content` as a string;
/// else the first `message.content` array item whose `type` is `"text"`
/// (its `text` field); else top-level `content` as a string. `None` when
/// none of the three shapes produced text.
fn extract_text(record: &Map<String, Value>) -> Option<String> {
    let content = record.get("message").and_then(|m| m.get("content"));
    if let Some(s) = content.and_then(Value::as_str) {
        return Some(s.to_owned());
    }
    if let Some(items) = content.and_then(Value::as_array) {
        let text = items
            .iter()
            .find(|item| {
                item.get("type").and_then(Value::as_str) == Some("text")
            })
            .and_then(|item| item.get("text"))
            .and_then(Value::as_str);
        if let Some(t) = text {
            return Some(t.to_owned());
        }
    }
    record
        .get("content")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// Collapses whitespace and trims; `None` when nothing is left. Past
/// [`TITLE_LIMIT`] characters the result is cut and a trailing `…`
/// appended, the same convention `willie-sess` uses for a helper's words.
fn normalize(text: &str) -> Option<String> {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.chars().count() > TITLE_LIMIT {
        let cut: String = trimmed.chars().take(TITLE_LIMIT).collect();
        Some(format!("{cut}…"))
    } else {
        Some(trimmed.to_owned())
    }
}

/// The `*.jsonl` directly under `dir` with the newest modified time among
/// those modified at or after `started_at_secs` -- a log modified before
/// the session started belongs to an earlier one. A missing or
/// unreadable directory, or one with no qualifying file, is `None`.
#[must_use]
pub fn pick_log_for(dir: &Path, started_at_secs: u64) -> Option<PathBuf> {
    let entries = fs::read_dir(dir).ok()?;
    entries
        .flatten()
        .filter(|entry| {
            entry.path().extension().and_then(|e| e.to_str()) == Some("jsonl")
        })
        .filter_map(|entry| {
            let secs = entry
                .metadata()
                .and_then(|m| m.modified())
                .ok()?
                .duration_since(UNIX_EPOCH)
                .ok()?
                .as_secs();
            (secs >= started_at_secs).then(|| (entry.path(), secs))
        })
        .max_by_key(|(_, secs)| *secs)
        .map(|(path, _)| path)
}

/// A session's title, resolved through the harness registry: for the
/// first harness (in registry order) whose `session_logs_dir(home)` and
/// `pick_log_for` both resolve, reads the first [`HEAD_BYTES`] of that
/// file and hands it to [`first_prompt`]. Any missing directory, empty
/// listing, unreadable file or unparsed content falls through to the
/// next harness; `None` when none of them yields a title. Never an
/// error, never a panic -- the read is on untrusted, possibly-partial
/// log content written by another process.
#[must_use]
pub fn first_prompt_for(
    home: &Path,
    workspace: &str,
    started_at_secs: u64,
) -> Option<String> {
    willie_harness::registry().into_iter().find_map(|h| {
        let dir = h.session_logs_dir(home)?;
        let logdir = dir.join(h.escape_workspace(workspace));
        let path = pick_log_for(&logdir, started_at_secs)?;
        let head = read_head(&path)?;
        first_prompt(&head)
    })
}

/// The first [`HEAD_BYTES`] of `path`, decoded as UTF-8 with lossy
/// replacement. Any I/O failure (the file vanished mid-read, a
/// permission fault) yields `None` rather than a fault.
fn read_head(path: &Path) -> Option<String> {
    let file = fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(HEAD_BYTES).read_to_end(&mut bytes).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    /// A leading system line and a corrupt line are skipped; the user
    /// record's 120-character message is collapsed (it has no whitespace
    /// to collapse) and cut to 80 characters plus a trailing `…`.
    #[test]
    fn first_prompt_takes_the_first_user_record_and_trims_it() {
        let message = "x".repeat(120);
        let user_line = format!(
            r#"{{"type":"user","message":{{"role":"user","content":"{message}"}}}}"#
        );
        let tail = format!(
            "{}\n{}\n{}\n",
            r#"{"type":"system","subtype":"init"}"#,
            "{not valid json",
            user_line,
        );

        let title = first_prompt(&tail).unwrap();

        assert_eq!(title, format!("{}…", "x".repeat(80)));
    }

    /// The content array's first non-text item is skipped in favour of
    /// the first item whose `type` is `"text"`.
    #[test]
    fn first_prompt_reads_a_content_array() {
        let tail = r#"{"type":"user","message":{"role":"user","content":[{"type":"image","source":"s"},{"type":"text","text":"fix the login bug"}]}}"#;

        assert_eq!(first_prompt(tail).as_deref(), Some("fix the login bug"));
    }

    /// A log with no user record at all (only a system line and an
    /// assistant reply) resolves to `None`, not a guess.
    #[test]
    fn first_prompt_is_none_without_a_user_record() {
        let tail = concat!(
            r#"{"type":"system","subtype":"init"}"#,
            "\n",
            r#"{"type":"assistant","message":{"role":"assistant","content":"hi"}}"#,
        );

        assert_eq!(first_prompt(tail), None);
    }

    /// Three logs: one modified before the session started (excluded),
    /// and two modified after it, of which the newest is picked.
    #[test]
    fn pick_log_for_prefers_the_newest_log_started_after_the_session() {
        let dir = std::env::temp_dir()
            .join(format!("willie-session-title-pick-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let started_at_secs = 1_700_000_000u64;
        let plant = |name: &str, secs: u64| -> PathBuf {
            let path = dir.join(name);
            fs::write(&path, "").unwrap();
            let file = fs::OpenOptions::new().write(true).open(&path).unwrap();
            file.set_modified(UNIX_EPOCH + Duration::from_secs(secs))
                .unwrap();
            path
        };
        plant("before.jsonl", started_at_secs - 100);
        plant("older.jsonl", started_at_secs + 10);
        let newest = plant("newest.jsonl", started_at_secs + 20);

        let picked = pick_log_for(&dir, started_at_secs);

        assert_eq!(picked, Some(newest));
        let _ = fs::remove_dir_all(&dir);
    }
}
