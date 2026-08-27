//! `events.jsonl`: the append-only truth about one session's lifetime.
//! Only the supervisor writes it; the daemon and the tests read it.

use std::{
    fs::{File, OpenOptions},
    io::{self, BufRead, BufReader, Write},
    path::Path,
    sync::{Mutex, MutexGuard, PoisonError},
    time::{SystemTime, UNIX_EPOCH},
};

use willie_core::session::{SessionEvent, SessionEventKind};

/// Seconds since the Unix epoch as a decimal string — the same clock
/// shape the daemon stamps on jobs.
#[must_use]
pub fn epoch_secs() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
        .to_string()
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The open log. Cloneable through `Arc` by callers; every append is one
/// line, flushed, so a supervisor killed mid-run leaves a readable file.
#[derive(Debug)]
pub struct EventLog {
    file: Mutex<File>,
    clock: fn() -> String,
}

impl EventLog {
    /// Open (create) the log, owner-readable only, close-on-exec so the
    /// harness never inherits it.
    pub fn open(path: &Path, clock: fn() -> String) -> io::Result<Self> {
        let mut options = OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc_o_cloexec());
        }
        let file = options.open(path)?;
        Ok(Self {
            file: Mutex::new(file),
            clock,
        })
    }

    /// Append one event and return it, stamped, for live fan-out. A write
    /// failure is reported on stderr (the supervisor log) and nothing
    /// else: the session must not die because its history could not be
    /// written.
    pub fn append(&self, kind: SessionEventKind) -> SessionEvent {
        let event = SessionEvent {
            at: (self.clock)(),
            kind,
        };
        match serde_json::to_string(&event) {
            Ok(line) => {
                let mut file = lock(&self.file);
                if let Err(e) =
                    writeln!(file, "{line}").and_then(|()| file.flush())
                {
                    eprintln!("willie-sess: could not append an event: {e}");
                }
            }
            Err(e) => eprintln!("willie-sess: could not encode an event: {e}"),
        }
        event
    }
}

#[cfg(unix)]
fn libc_o_cloexec() -> i32 {
    // O_CLOEXEC is 0o2000000 on every Linux architecture.
    0o2_000_000
}

/// Every parseable line, in order. Lines that do not parse are skipped:
/// a torn last line from a killed supervisor must not hide the rest.
///
/// The read side of the log: the supervisor only appends, so today the
/// round-trip tests are the only readers. Kept as the log's reader for a
/// daemon that re-adopts a running session.
#[allow(dead_code)]
pub fn read_all(path: &Path) -> io::Result<Vec<SessionEvent>> {
    let reader = BufReader::new(File::open(path)?);
    let mut out = Vec::new();
    for line in reader.lines() {
        let line = line?;
        if let Ok(ev) = serde_json::from_str::<SessionEvent>(&line) {
            out.push(ev);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use willie_core::session::SessionEventKind;

    fn clock() -> String {
        "77".into()
    }

    #[test]
    fn appended_events_are_one_json_line_each_and_read_back_in_order() {
        let dir = std::env::temp_dir()
            .join(format!("willie-sess-events-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("events.jsonl");
        let log = EventLog::open(&path, clock).unwrap();
        let first = log.append(SessionEventKind::Created);
        log.append(SessionEventKind::Started { pid: 5 });
        assert_eq!(first.at, "77");
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), 2);
        assert_eq!(
            text.lines().next().unwrap(),
            r#"{"at":"77","kind":"created"}"#
        );
        let events = read_all(&path).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].kind, SessionEventKind::Started { pid: 5 });
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unreadable_lines_are_skipped_not_fatal() {
        let dir = std::env::temp_dir()
            .join(format!("willie-sess-badevents-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("events.jsonl");
        std::fs::write(
            &path,
            "{\"at\":\"1\",\"kind\":\"created\"}\ngarbage\n{\"at\":\"2\",\"kind\":\"started\",\"pid\":1}\n",
        )
        .unwrap();
        let events = read_all(&path).unwrap();
        assert_eq!(events.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_clock_is_a_decimal_epoch_second_string() {
        let s = epoch_secs();
        assert!(!s.is_empty() && s.chars().all(|c| c.is_ascii_digit()));
    }
}
