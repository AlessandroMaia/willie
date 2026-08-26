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

/// One line per check: `[ok ]`, `[FAIL]` or `[skip]`, then the detail,
/// then the remediation for failures (see docs/CLI_CONTRACT.md).
fn render(report: &DoctorReport) -> String {
    let mut out = String::new();
    for check in &report.checks {
        let tag = match check.status {
            CheckStatus::Ok => "[ok ]",
            CheckStatus::Fail => "[FAIL]",
            CheckStatus::Skip => "[skip]",
        };
        out.push_str(&format!("{tag} {:<28} {}\n", check.name, check.detail));
        if check.status == CheckStatus::Fail
            && let Some(hint) = &check.remediation
        {
            out.push_str(&format!("       → {hint}\n"));
        }
    }
    out
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
        match serde_json::to_string_pretty(&report) {
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
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["--version"] => {
            println!("{}", version_line());
            ExitCode::SUCCESS
        }
        ["doctor"] => doctor(false),
        ["doctor", "--json"] => doctor(true),
        _ => usage(),
    }
}

#[cfg(not(target_os = "linux"))]
fn main() -> ExitCode {
    let _ = (doctor, render, exit_code);
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
    fn render_uses_the_contract_prefixes_and_shows_remediation_on_failure() {
        let report = DoctorReport {
            checks: vec![
                c("a", CheckStatus::Ok, true),
                c("b", CheckStatus::Fail, true),
                c("c", CheckStatus::Skip, false),
            ],
        };
        let text = render(&report);
        assert!(text.contains("[ok ] a"));
        assert!(text.contains("[FAIL] b"));
        assert!(text.contains("fix it"));
        assert!(text.contains("[skip] c"));
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
