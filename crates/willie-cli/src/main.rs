//! `willie` — the stateless CLI used inside the distribution.
//!
//! Windows Terminal tabs will run `willie attach <session>` through
//! `wsl.exe` (later slice). Holds no state of its own. Linux only.

#![deny(clippy::unwrap_used, clippy::expect_used)]

use std::process::ExitCode;

use willie_proto::daemon::{CheckStatus, DoctorReport};

const EXIT_FAILURE: u8 = 1;
const EXIT_USAGE: u8 = 2;
const EXIT_PRECONDITION: u8 = 3;

fn version_line() -> String {
    format!("willie {}", willie_core::VERSION)
}

/// The one thing the command line asked for.
#[derive(Debug, PartialEq, Eq)]
enum Command {
    Version,
    Doctor { json: bool },
    Usage,
}

fn parse(args: &[&str]) -> Command {
    match args {
        ["--version"] => Command::Version,
        ["doctor"] => Command::Doctor { json: false },
        ["doctor", "--json"] => Command::Doctor { json: true },
        _ => Command::Usage,
    }
}

/// One line per check: `[ok ]`, `[FAIL]` or `[skip]`, then the detail,
/// then the remediation on the same line for failures (see
/// docs/CLI_CONTRACT.md).
fn render(report: &DoctorReport) -> String {
    let mut out = String::new();
    for check in &report.checks {
        let tag = match check.status {
            CheckStatus::Ok => "[ok ]",
            CheckStatus::Fail => "[FAIL]",
            CheckStatus::Skip => "[skip]",
        };
        let mut line = format!("{tag:<6} {:<28} {}", check.name, check.detail);
        if check.status == CheckStatus::Fail
            && let Some(hint) = &check.remediation
        {
            line.push_str(&format!(" → {hint}"));
        }
        line.push('\n');
        out.push_str(&line);
    }
    out
}

/// Pretty JSON for a report, as printed by `willie doctor --json`.
fn render_json(report: &DoctorReport) -> serde_json::Result<String> {
    serde_json::to_string_pretty(report)
}

fn exit_code(report: &DoctorReport) -> u8 {
    if report.healthy() {
        0
    } else {
        EXIT_PRECONDITION
    }
}

fn doctor(json: bool) -> ExitCode {
    let report = willie_linux::doctor::run_all();
    if json {
        match render_json(&report) {
            Ok(text) => println!("{text}"),
            Err(e) => {
                eprintln!("willie doctor: cannot encode report: {e}");
                return ExitCode::from(EXIT_FAILURE);
            }
        }
    } else {
        print!("{}", render(&report));
    }
    ExitCode::from(exit_code(&report))
}

fn usage() -> ExitCode {
    eprintln!("usage: willie doctor [--json] | willie --version");
    ExitCode::from(EXIT_USAGE)
}

#[cfg(target_os = "linux")]
fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match parse(&args) {
        Command::Version => {
            println!("{}", version_line());
            ExitCode::SUCCESS
        }
        Command::Doctor { json } => doctor(json),
        Command::Usage => usage(),
    }
}

#[cfg(not(target_os = "linux"))]
fn main() -> ExitCode {
    let _ = (doctor, render, render_json, exit_code, parse);
    eprintln!(
        "{} runs only inside the Willie Linux distribution",
        version_line()
    );
    usage()
}

#[cfg(test)]
mod tests {
    use super::*;
    use willie_proto::daemon::{CheckStatus, DoctorCheck, DoctorReport};

    fn c(name: &str, status: CheckStatus, required: bool) -> DoctorCheck {
        DoctorCheck {
            name: name.into(),
            status,
            detail: "d".into(),
            remediation: Some("fix it".into()),
            required,
        }
    }

    #[test]
    fn version_line_names_the_binary_and_version() {
        assert_eq!(version_line(), format!("willie {}", willie_core::VERSION));
    }

    #[test]
    fn version_flag_parses() {
        assert_eq!(parse(&["--version"]), Command::Version);
    }

    #[test]
    fn doctor_parses_with_and_without_json() {
        assert_eq!(parse(&["doctor"]), Command::Doctor { json: false });
        assert_eq!(
            parse(&["doctor", "--json"]),
            Command::Doctor { json: true }
        );
    }

    #[test]
    fn unknown_or_reordered_arguments_are_usage() {
        assert_eq!(parse(&["--json", "doctor"]), Command::Usage);
        assert_eq!(parse(&["doctor", "--json", "extra"]), Command::Usage);
        assert_eq!(parse(&[]), Command::Usage);
    }

    #[test]
    fn render_uses_the_contract_prefixes_and_shows_remediation_on_failure() {
        let report = DoctorReport {
            checks: vec![
                c("a", CheckStatus::Ok, true),
                c("b", CheckStatus::Fail, true),
                c("c", CheckStatus::Skip, false),
            ],
        };
        let text = render(&report);
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines[0].starts_with("[ok ]  a"));
        assert!(!lines[0].contains("fix it"));
        assert!(lines[1].starts_with("[FAIL] b"));
        assert!(lines[1].contains("→ fix it"));
        assert!(lines[2].starts_with("[skip] c"));
    }

    #[test]
    fn render_json_round_trips_through_a_doctor_report() {
        let report = DoctorReport {
            checks: vec![c("a", CheckStatus::Ok, true)],
        };
        let text = render_json(&report).unwrap();
        let back: DoctorReport = serde_json::from_str(&text).unwrap();
        assert_eq!(back, report);
    }

    #[test]
    fn exit_code_is_3_only_when_a_required_check_fails() {
        assert_eq!(
            exit_code(&DoctorReport {
                checks: vec![c("a", CheckStatus::Fail, false)]
            }),
            0
        );
        assert_eq!(
            exit_code(&DoctorReport {
                checks: vec![c("a", CheckStatus::Fail, true)]
            }),
            3
        );
    }
}
