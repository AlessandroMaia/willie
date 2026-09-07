//! `willie` — the stateless CLI used inside the distribution.
//!
//! Windows Terminal tabs will run `willie attach <session>` through
//! `wsl.exe` (later slice). Holds no state of its own. Linux only.

#![deny(clippy::unwrap_used, clippy::expect_used)]

use std::process::ExitCode;

#[cfg(target_os = "linux")]
mod attach;

/// A client for the daemon's own local socket, named apart from the
/// `daemon` proto module alias below so `fetch_explain` can use both in
/// the same scope.
#[cfg(target_os = "linux")]
mod daemon_client;

#[cfg(target_os = "linux")]
use willie_core::id::ProjectId;
use willie_core::sandbox::{Explained, Source};
use willie_proto::{
    daemon::{CheckStatus, DoctorReport},
    rpc::RpcError,
    sandbox::ExplainResult,
};
#[cfg(target_os = "linux")]
use willie_proto::{
    daemon::{Hello, HelloReply, method as daemon},
    project::{ProjectList, method as project},
    sandbox::{ExplainParams, method as sandbox},
};

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
    Doctor {
        json: bool,
    },
    /// Serve one terminal from the session at `target` (a session id or a
    /// socket path).
    Attach {
        target: String,
        /// Fixed window size, for a client with no terminal of its own.
        size: Option<(u16, u16)>,
        /// Whether to put the local terminal in raw mode.
        raw: bool,
        /// Read `hostterm` frames from stdin instead of a raw terminal;
        /// stdout carries only session bytes. For the desktop app.
        host: bool,
    },
    /// What a session for `project` would run under, without starting
    /// one.
    SandboxExplain {
        project: String,
        json: bool,
    },
    Usage,
}

fn parse(args: &[&str]) -> Command {
    match args {
        ["--version"] => Command::Version,
        ["doctor"] => Command::Doctor { json: false },
        ["doctor", "--json"] => Command::Doctor { json: true },
        ["sandbox", "explain", project] => Command::SandboxExplain {
            project: (*project).to_owned(),
            json: false,
        },
        ["sandbox", "explain", project, "--json"] => Command::SandboxExplain {
            project: (*project).to_owned(),
            json: true,
        },
        ["attach", rest @ ..] => parse_attach(rest),
        _ => Command::Usage,
    }
}

fn parse_attach(args: &[&str]) -> Command {
    let mut target = None;
    let mut size = None;
    let mut raw = true;
    let mut host = false;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match *arg {
            "--no-raw" => raw = false,
            "--host" => host = true,
            "--size" => match rest.next().and_then(|v| parse_size(v)) {
                Some(parsed) => size = Some(parsed),
                None => return Command::Usage,
            },
            other if other.starts_with('-') => return Command::Usage,
            other if target.is_none() => target = Some(other.to_owned()),
            _ => return Command::Usage,
        }
    }
    match target {
        Some(target) => Command::Attach {
            target,
            size,
            raw,
            host,
        },
        None => Command::Usage,
    }
}

/// `ROWSxCOLS`, as `stty size` reports it: rows first.
fn parse_size(text: &str) -> Option<(u16, u16)> {
    let (rows, cols) = text.split_once('x')?;
    Some((rows.parse().ok()?, cols.parse().ok()?))
}

/// A bare session id is the well-known socket; anything with a `/` is a
/// socket path (tests, hand-made sessions).
fn socket_for(target: &str) -> std::path::PathBuf {
    if target.contains('/') {
        std::path::PathBuf::from(target)
    } else {
        willie_linux::paths::session_socket(
            std::path::Path::new(willie_linux::paths::RUN_DIR),
            target,
        )
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

/// One line per capability: name, state, source. Data on stdout, so no
/// heading and no consequence text here. Named apart from `render`
/// (doctor's report renderer already owns that name in this file).
fn render_capabilities(entries: &[Explained]) -> String {
    entries
        .iter()
        .map(|e| {
            format!(
                "{}\t{}\t{}",
                e.capability.display_name(),
                if e.enabled { "on" } else { "off" },
                match e.source {
                    Source::Default => "default",
                    Source::Profile => "profile",
                    Source::Unavailable => "unavailable",
                }
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The stderr line a failed command ends with: the command's own
/// prefix, the stable code, the message, then the remediation when the
/// error carries one. The code belongs on this line because
/// docs/CLI_CONTRACT.md makes it part of every error, not only of the
/// `--json` object: without it the only way to tell one refusal from
/// another is to match the prose.
fn error_line(command: &str, err: &RpcError) -> String {
    let mut line = format!("{command}: {}: {}", err.code, err.message);
    if let Some(hint) = &err.remediation {
        line.push_str(&format!(" → {hint}"));
    }
    line
}

/// Where the daemon's socket lives: `WILLIE_RUN_DIR` when set (tests, a
/// hermetic run), else `willie_linux::paths::RUN_DIR`, matching how
/// `willied` itself resolves the run dir.
#[cfg(target_os = "linux")]
fn daemon_socket_path() -> std::path::PathBuf {
    let run_dir = std::env::var("WILLIE_RUN_DIR")
        .unwrap_or_else(|_| willie_linux::paths::RUN_DIR.to_owned());
    willie_linux::paths::daemon_socket(std::path::Path::new(&run_dir))
}

/// `project` as a `proj_…` id is used as-is (a malformed one is
/// `project_not_found`, not a parse error the caller has to interpret);
/// otherwise it's a slug, resolved through `project.list`.
#[cfg(target_os = "linux")]
fn resolve_project(
    client: &mut daemon_client::Client,
    project: &str,
) -> Result<ProjectId, RpcError> {
    if project.starts_with("proj_") {
        return project.parse::<ProjectId>().map_err(|_| {
            RpcError::new(
                "project_not_found",
                "the id is not a valid project id",
            )
            .with_remediation(
                "check the slug against the Projects screen, or pass the \
                 proj_ id",
            )
        });
    }
    let list: ProjectList =
        client.call(project::LIST, serde_json::json!({}))?;
    list.projects
        .into_iter()
        .find(|p| p.slug == project)
        .map(|p| p.id)
        .ok_or_else(|| {
            RpcError::new(
                "project_not_found",
                "no project has that slug; the slug is the workspace \
                 directory name under ~/projects",
            )
            .with_remediation(
                "check the slug against the Projects screen, or pass the \
                 proj_ id",
            )
        })
}

/// Connects to the daemon's local socket, says hello, resolves `project`
/// to a project id, and asks for its sandbox explanation. No socket (or
/// nothing listening) is `daemon_unreachable`; a reply that doesn't
/// arrive in ten seconds is `daemon_timeout`; anything else on the wire
/// is `daemon_transport`.
#[cfg(target_os = "linux")]
fn fetch_explain(project: &str) -> Result<ExplainResult, RpcError> {
    let socket = daemon_socket_path();
    let mut client = daemon_client::Client::connect(&socket)?;
    let _hello: HelloReply =
        client.call(daemon::HELLO, Hello::for_client("willie"))?;
    let project_id = resolve_project(&mut client, project)?;
    client.call(sandbox::EXPLAIN, ExplainParams { project_id })
}

/// Never reached: the non-Linux build's `main` never dispatches to
/// `sandbox_explain`. Kept, with the pre-socket error code, so `cargo
/// check --workspace` still succeeds on the Windows host — mirroring
/// `real_clock_or_zero` in `willied`.
#[cfg(not(target_os = "linux"))]
fn fetch_explain(project: &str) -> Result<ExplainResult, RpcError> {
    let _ = project;
    Err(RpcError::new(
        "daemon_unreachable",
        "willie only runs inside the Willie Linux distribution",
    ))
}

/// `willie sandbox explain <project>`: data on stdout, prose on stderr,
/// `--json` prints the reply verbatim.
fn sandbox_explain(project: &str, json: bool) -> ExitCode {
    match fetch_explain(project) {
        Ok(reply) => {
            if json {
                match serde_json::to_string_pretty(&reply) {
                    Ok(text) => println!("{text}"),
                    Err(e) => {
                        eprintln!(
                            "willie sandbox explain: cannot encode reply: {e}"
                        );
                        return ExitCode::from(EXIT_FAILURE);
                    }
                }
            } else {
                eprintln!("capability\tstate\tsource");
                println!("{}", render_capabilities(&reply.entries));
                for entry in &reply.entries {
                    eprintln!(
                        "  {}: {}",
                        entry.capability.display_name(),
                        entry.capability.consequence()
                    );
                }
            }
            ExitCode::SUCCESS
        }
        Err(err) => {
            if json {
                match serde_json::to_string_pretty(&err) {
                    Ok(text) => eprintln!("{text}"),
                    Err(e) => eprintln!(
                        "willie sandbox explain: cannot encode error: {e}"
                    ),
                }
            } else {
                eprintln!("{}", error_line("willie sandbox explain", &err));
            }
            ExitCode::from(EXIT_FAILURE)
        }
    }
}

fn usage() -> ExitCode {
    eprintln!(
        "usage: willie attach [--no-raw] [--size ROWSxCOLS] [--host] \
         <session-id|socket> | willie doctor [--json] | willie sandbox \
         explain <project> [--json] | willie --version"
    );
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
        Command::Attach {
            target,
            size,
            raw,
            host,
        } => attach::run(&socket_for(&target), size, raw, host),
        Command::SandboxExplain { project, json } => {
            sandbox_explain(&project, json)
        }
        Command::Usage => usage(),
    }
}

#[cfg(not(target_os = "linux"))]
fn main() -> ExitCode {
    let _ = (
        doctor,
        render,
        render_json,
        exit_code,
        parse,
        parse_attach,
        parse_size,
        socket_for,
        render_capabilities,
        fetch_explain,
        sandbox_explain,
    );
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
    fn attach_takes_an_id_or_a_path_and_two_optional_flags() {
        assert_eq!(
            parse(&["attach", "sess_01J"]),
            Command::Attach {
                target: "sess_01J".into(),
                size: None,
                raw: true,
                host: false,
            }
        );
        assert_eq!(
            parse(&["attach", "--no-raw", "--size", "24x80", "/tmp/s.sock"]),
            Command::Attach {
                target: "/tmp/s.sock".into(),
                size: Some((24, 80)),
                raw: false,
                host: false,
            }
        );
        assert_eq!(parse(&["attach"]), Command::Usage);
        assert_eq!(parse(&["attach", "--size", "x", "a"]), Command::Usage);
        assert_eq!(parse(&["attach", "a", "b"]), Command::Usage);
    }

    #[test]
    fn host_flag_parses() {
        assert_eq!(
            parse(&["attach", "sess_01J", "--host"]),
            Command::Attach {
                target: "sess_01J".into(),
                size: None,
                raw: true,
                host: true,
            }
        );
    }

    #[test]
    fn a_bare_id_resolves_to_the_run_dir_socket_and_a_path_is_kept() {
        assert_eq!(
            socket_for("sess_01J"),
            std::path::PathBuf::from("/run/willie/sessions/sess_01J.sock")
        );
        assert_eq!(
            socket_for("/tmp/x.sock"),
            std::path::PathBuf::from("/tmp/x.sock")
        );
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

    #[test]
    fn sandbox_explain_parses_with_and_without_json() {
        assert_eq!(
            parse(&["sandbox", "explain", "proj_1"]),
            Command::SandboxExplain {
                project: "proj_1".into(),
                json: false,
            }
        );
        assert_eq!(
            parse(&["sandbox", "explain", "proj_1", "--json"]),
            Command::SandboxExplain {
                project: "proj_1".into(),
                json: true,
            }
        );
        assert_eq!(parse(&["sandbox", "explain"]), Command::Usage);
        assert_eq!(parse(&["sandbox"]), Command::Usage);
    }

    #[test]
    fn render_capabilities_prints_name_state_and_source_tab_separated() {
        use willie_core::sandbox::Capability;

        let entries = vec![
            Explained {
                capability: Capability::ProjectRw,
                enabled: true,
                source: Source::Default,
            },
            Explained {
                capability: Capability::AgentState,
                enabled: false,
                source: Source::Profile,
            },
            Explained {
                capability: Capability::Ssh,
                enabled: false,
                source: Source::Unavailable,
            },
        ];

        let text = render_capabilities(&entries);

        assert_eq!(
            text.lines().collect::<Vec<_>>(),
            [
                "project.rw\ton\tdefault",
                "agent.state\toff\tprofile",
                "ssh\toff\tunavailable"
            ]
        );
    }

    #[test]
    fn a_text_mode_error_names_the_code_then_the_message_then_the_hint() {
        let err = RpcError::new("daemon_unreachable", "nothing to ask")
            .with_remediation("read the record");

        assert_eq!(
            error_line("willie sandbox explain", &err),
            "willie sandbox explain: daemon_unreachable: nothing to ask \
             → read the record"
        );
        assert_eq!(
            error_line("willie sandbox explain", &RpcError::new("x", "no")),
            "willie sandbox explain: x: no"
        );
    }
}
