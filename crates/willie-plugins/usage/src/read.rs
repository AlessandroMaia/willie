//! Pure, host-testable reading of a harness session's JSONL tail.
//!
//! Every function here takes plain strings or in-memory listings and
//! returns values, never errors: a malformed line is a skip, not a
//! failure, because a session log is written by another process while
//! we read it and a torn last line is expected, not exceptional. No
//! filesystem I/O and no dependency on `willied` or `willie-proto` — the
//! usage plugin (its own crate slice) does the actual reads and maps
//! [`SessionReading`] onto the wire `SessionUsage` type.

use serde_json::Value;

/// A session's token usage and context-window fill, computed from the
/// newest usage-carrying record in a JSONL tail. `context_pct` is `None`
/// when the caller could not resolve a context window for the model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionReading {
    pub tokens: u64,
    pub context_pct: Option<u8>,
}

/// The four token fields a usage block may carry, each defaulting to 0,
/// paired with that same record's model — kept together so a later
/// record's model can never combine with an earlier record's usage.
struct TokenCounts {
    input: u64,
    cache_creation: u64,
    cache_read: u64,
    output: u64,
    model: Option<String>,
}

/// Reads the newest usage block out of a session log tail.
///
/// Lines are parsed independently as JSON values; a non-object, a line
/// that fails to parse, or a `isSidechain: true` record is skipped
/// (best-effort, never a panic). The usage block may sit at top-level
/// `usage` or nested under `message.usage` — Claude Code writes the
/// latter for assistant turns; both are accepted. The *last* matching
/// line in the tail wins, since JSONL is append-order and the tail is
/// assumed to end at (or near) "now". That same record's model —
/// `message.model`, else a top-level `model` — resolves the context
/// window through `context_window_for`, falling back to the `context_window`
/// param when the model is missing or unrecognised. `context_pct` covers
/// only the input-side fields (a session's context window is what it must
/// hold on the next turn, not what it just produced), clamped to 0..=100.
#[must_use]
pub fn project_session(
    jsonl_tail: &str,
    context_window: Option<u64>,
) -> SessionReading {
    let mut newest: Option<TokenCounts> = None;

    for line in jsonl_tail.lines() {
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
        if record.get("isSidechain").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let usage = record
            .get("usage")
            .or_else(|| record.get("message").and_then(|m| m.get("usage")))
            .and_then(Value::as_object);
        let Some(usage) = usage else {
            continue;
        };

        let field =
            |key: &str| usage.get(key).and_then(Value::as_u64).unwrap_or(0);
        let model = record
            .get("message")
            .and_then(|m| m.get("model"))
            .or_else(|| record.get("model"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        newest = Some(TokenCounts {
            input: field("input_tokens"),
            cache_creation: field("cache_creation_input_tokens"),
            cache_read: field("cache_read_input_tokens"),
            output: field("output_tokens"),
            model,
        });
    }

    let Some(counts) = newest else {
        return SessionReading {
            tokens: 0,
            context_pct: None,
        };
    };

    // `Value::as_u64` accepts any field up to u64::MAX, so a corrupt-but-
    // parseable record (a bogus huge token count) must not panic here:
    // best-effort means saturating on the sum and widening to u128 before
    // the percentage's `* 100`, which would otherwise overflow well below
    // u64::MAX.
    let tokens = counts
        .input
        .saturating_add(counts.cache_creation)
        .saturating_add(counts.cache_read)
        .saturating_add(counts.output);
    let context_side = counts
        .input
        .saturating_add(counts.cache_creation)
        .saturating_add(counts.cache_read);
    let window = context_window_for(counts.model.as_deref()).or(context_window);
    let context_pct = window.filter(|&w| w > 0).map(|window| {
        (u128::from(context_side) * 100 / u128::from(window)).min(100) as u8
    });

    SessionReading {
        tokens,
        context_pct,
    }
}

/// Picks the log file whose recorded time falls inside the session
/// window `(start, end)`, inclusive on both ends. When a project keeps
/// one JSONL file per calendar day (or per size-based rotation), the
/// file covering "now" is the one whose timestamp lands in the window
/// the caller is asking about; ties resolve to the first match in
/// `dir_listing`, which the caller controls the order of.
#[must_use]
pub fn pick_log(
    dir_listing: &[(String, u64)],
    window: (u64, u64),
) -> Option<String> {
    let (start, end) = window;
    dir_listing
        .iter()
        .find(|(_, time)| *time >= start && *time <= end)
        .map(|(name, _)| name.clone())
}

/// A small table of known context windows by model-name prefix. Only
/// covers the `claude-` family Willie launches today; an unrecognised
/// or absent model name resolves to `None` rather than guessing.
#[must_use]
pub fn context_window_for(model: Option<&str>) -> Option<u64> {
    match model {
        Some(name) if name.starts_with("claude-") => Some(200_000),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Claude Code's assistant records nest usage under `message.usage`;
    /// a preceding user record, a sidechain record with implausibly
    /// large counts, and a trailing corrupt line must not affect the
    /// result — only the real assistant record's four fields are summed,
    /// and the context percentage counts only the input-side fields.
    #[test]
    fn newest_assistant_usage_sums_four_fields_and_skips_sidechain_and_corrupt_lines()
     {
        let tail = concat!(
            r#"{"type":"user","message":{"role":"user","content":"hi"}}"#,
            "\n",
            r#"{"type":"assistant","message":{"role":"assistant","usage":{"input_tokens":50000,"cache_creation_input_tokens":10000,"cache_read_input_tokens":5000,"output_tokens":20000}}}"#,
            "\n",
            r#"{"type":"assistant","isSidechain":true,"message":{"usage":{"input_tokens":999999,"cache_creation_input_tokens":999999,"cache_read_input_tokens":999999,"output_tokens":999999}}}"#,
            "\n",
            "{not valid json",
        );

        let reading = project_session(tail, Some(100_000));

        assert_eq!(reading.tokens, 85_000);
        assert_eq!(reading.context_pct, Some(65));
    }

    /// A corrupt-but-parseable line can carry a bogus huge token count
    /// (`Value::as_u64` accepts up to `u64::MAX`) — the sum and the
    /// percentage math must saturate/clamp rather than overflow-panic.
    #[test]
    fn a_huge_token_count_saturates_instead_of_overflowing() {
        let tail = r#"{"message":{"usage":{"input_tokens":18446744073709551615,"cache_creation_input_tokens":18446744073709551615,"cache_read_input_tokens":18446744073709551615,"output_tokens":18446744073709551615}}}"#;

        let reading = project_session(tail, Some(100_000));

        assert_eq!(reading.tokens, u64::MAX);
        assert_eq!(reading.context_pct, Some(100));
    }

    /// Two usage-carrying lines, neither sidechain nor corrupt: the last
    /// one in the tail wins, not the first (JSONL is append-order).
    #[test]
    fn newest_of_two_valid_usage_records_wins_over_the_first() {
        let tail = concat!(
            r#"{"message":{"usage":{"input_tokens":1,"cache_creation_input_tokens":1,"cache_read_input_tokens":1,"output_tokens":1}}}"#,
            "\n",
            r#"{"message":{"usage":{"input_tokens":100,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":0}}}"#,
        );

        let reading = project_session(tail, None);

        assert_eq!(reading.tokens, 100);
    }

    /// A top-level `usage` block (no `message` nesting) is also a valid
    /// shape and must be picked up the same way.
    #[test]
    fn top_level_usage_block_is_also_accepted() {
        let tail = r#"{"type":"summary","usage":{"input_tokens":10,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":5}}"#;

        let reading = project_session(tail, None);

        assert_eq!(reading.tokens, 15);
        assert_eq!(reading.context_pct, None);
    }

    /// No line in the tail carries a usable usage block (only a user
    /// record and a corrupt line) — a best-effort zero, not a panic.
    #[test]
    fn tail_with_no_valid_usage_record_yields_zero_tokens_and_no_context_pct() {
        let tail = concat!(
            r#"{"type":"user","message":{"role":"user","content":"hi"}}"#,
            "\n",
            "{also not valid",
        );

        let reading = project_session(tail, Some(200_000));

        assert_eq!(reading.tokens, 0);
        assert_eq!(reading.context_pct, None);
    }

    /// A missing context window (unresolved model) never yields a
    /// percentage even when a usage record is present.
    #[test]
    fn missing_context_window_yields_no_context_pct() {
        let tail = r#"{"message":{"usage":{"input_tokens":10,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":1}}}"#;

        let reading = project_session(tail, None);

        assert_eq!(reading.tokens, 11);
        assert_eq!(reading.context_pct, None);
    }

    /// The newest record's own `message.model` resolves the context
    /// window through `context_window_for` — no caller-supplied window
    /// needed at all — proving `context_pct` is derived from the record,
    /// not merely from whatever the caller happened to pass in.
    #[test]
    fn a_recognized_model_on_the_newest_record_resolves_the_window() {
        let tail = r#"{"message":{"model":"claude-sonnet-4-20250514","usage":{"input_tokens":50000,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":1000}}}"#;

        let reading = project_session(tail, None);

        assert_eq!(reading.tokens, 51_000);
        assert_eq!(reading.context_pct, Some(25));
    }

    /// An unrecognized model on the newest record resolves to no window,
    /// same as no model at all — best-effort, never a guess.
    #[test]
    fn an_unrecognized_model_on_the_newest_record_yields_no_context_pct() {
        let tail = r#"{"message":{"model":"gpt-x","usage":{"input_tokens":10,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":1}}}"#;

        let reading = project_session(tail, None);

        assert_eq!(reading.tokens, 11);
        assert_eq!(reading.context_pct, None);
    }

    /// Among two candidate files, the one whose time falls inside the
    /// session window is picked; the other is ignored.
    #[test]
    fn pick_log_chooses_the_file_whose_time_overlaps_the_window() {
        let listing = vec![
            ("2026-09-01.jsonl".to_string(), 100),
            ("2026-09-06.jsonl".to_string(), 500),
        ];

        let picked = pick_log(&listing, (400, 600));

        assert_eq!(picked, Some("2026-09-06.jsonl".to_string()));
    }

    /// No file's recorded time falls inside the window: no pick, not a
    /// panic or a fallback guess.
    #[test]
    fn pick_log_returns_none_when_no_file_overlaps() {
        let listing = vec![
            ("2026-09-01.jsonl".to_string(), 100),
            ("2026-09-02.jsonl".to_string(), 200),
        ];

        assert_eq!(pick_log(&listing, (400, 600)), None);
    }

    /// The `claude-` family resolves to a known window; anything else,
    /// including no model at all, resolves to `None`.
    #[test]
    fn context_window_for_claude_models_is_some_unknown_is_none() {
        assert_eq!(
            context_window_for(Some("claude-3-5-sonnet-20241022")),
            Some(200_000)
        );
        assert_eq!(context_window_for(Some("gpt-4")), None);
        assert_eq!(context_window_for(None), None);
    }
}
