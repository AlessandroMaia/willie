//! `doctor-tools`: is this machine able to build and run Willie?
//!
//! Prints one line per tool, `[ok ]` or `[FAIL]`, with the detected version
//! or an install hint. Never installs anything: some prerequisites need an
//! elevated prompt and that decision belongs to the user.

use std::{path::Path, process::Command};

use crate::TaskResult;

/// One way of detecting a tool. Probes run in order until one is accepted.
struct Probe {
    argv: &'static [&'static str],
    /// Text the output must contain to count. Guards against a different
    /// program answering to the same name (GNU `link` vs MSVC `link.exe`).
    must_contain: Option<&'static str>,
    /// Some tools print their banner and exit non-zero when run without a
    /// real job; accept them anyway.
    ignore_exit_status: bool,
}

struct Check {
    name: &'static str,
    required: bool,
    probes: &'static [Probe],
    hint: &'static str,
}

const fn probe(argv: &'static [&'static str]) -> Probe {
    Probe {
        argv,
        must_contain: None,
        ignore_exit_status: false,
    }
}

const VSWHERE: &str =
    r"C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe";

const CHECKS: &[Check] = &[
    Check {
        name: "rustup",
        required: true,
        probes: &[probe(&["rustup", "--version"])],
        hint: "install from https://rustup.rs (per-user, no elevation)",
    },
    Check {
        name: "cargo (pinned toolchain)",
        required: true,
        probes: &[probe(&["cargo", "--version"])],
        hint: "rustup installs the toolchain from rust-toolchain.toml on first use",
    },
    Check {
        name: "MSVC linker (link.exe)",
        required: cfg!(windows),
        probes: &[
            Probe {
                argv: &[
                    VSWHERE,
                    "-latest",
                    "-products",
                    "*",
                    "-requires",
                    "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
                    "-property",
                    "displayName",
                ],
                must_contain: Some("Visual Studio"),
                ignore_exit_status: false,
            },
            Probe {
                argv: &["link.exe"],
                must_contain: Some("Microsoft (R) Incremental Linker"),
                ignore_exit_status: true,
            },
        ],
        hint: "install the \"Desktop development with C++\" workload \
(MSVC x64/x86 build tools + Windows SDK) with the Visual Studio installer",
    },
    Check {
        name: "just",
        required: true,
        probes: &[probe(&["just", "--version"])],
        hint: "download the release zip to %LOCALAPPDATA%\\Programs\\just and add it to PATH",
    },
    Check {
        name: "node",
        required: true,
        probes: &[probe(&["node", "--version"])],
        hint: "install Node.js 22 (needed by the frontend and the repo hooks)",
    },
    Check {
        name: "pnpm",
        required: true,
        probes: &[
            probe(&["pnpm", "--version"]),
            probe(&["pnpm.cmd", "--version"]),
        ],
        hint: "corepack enable pnpm, or the standalone installer",
    },
    Check {
        name: "cargo-zigbuild",
        required: true,
        probes: &[probe(&["cargo-zigbuild", "--version"])],
        hint: "python -m pip install --user cargo-zigbuild ziglang",
    },
    Check {
        name: "zig",
        required: true,
        probes: &[
            probe(&["zig", "version"]),
            probe(&["python", "-m", "ziglang", "version"]),
        ],
        hint: "python -m pip install --user ziglang (cargo-zigbuild finds it)",
    },
    Check {
        name: "wsl.exe",
        required: cfg!(windows),
        probes: &[probe(&["wsl.exe", "--version"])],
        hint: "WSL 2.4.4+ must be enabled by an administrator before Willie can run",
    },
    Check {
        name: "Windows Terminal (wt.exe)",
        required: false,
        probes: &[probe(&["wt.exe", "--version"])],
        hint: "recommended for session tabs; install from the Microsoft Store",
    },
];

/// A prerequisite that is a file in the working tree rather than a
/// program on PATH. Absence is always a failure: both entries below
/// stop `just check` from completing, so reporting them as optional
/// would only move the confusion later.
struct FileCheck {
    name: &'static str,
    /// Relative to the workspace root.
    path: &'static str,
    hint: &'static str,
}

const FILES: &[FileCheck] = &[
    FileCheck {
        name: "frontend dependencies",
        path: "node_modules",
        hint: "run `just setup` (installs from the lockfile at the root)",
    },
    FileCheck {
        name: "reference denylist",
        path: "docs/blueprint/refs-denylist.txt",
        hint: "`just check-refs` cannot run without it: place the list \
there, or point WILLIE_REFS_DENYLIST at it",
    },
];

/// The major version in `engines.node` of the root `package.json`.
/// Hand-parsed on purpose: one field of one file does not justify a
/// JSON dependency in a dev-only task.
fn node_floor(root: &Path) -> Option<u32> {
    let text = std::fs::read_to_string(root.join("package.json")).ok()?;
    let line = text.lines().find(|line| line.contains("\"node\""))?;
    first_number(line)
}

/// True when a detected `node --version` (`v24.18.0`) is at or above
/// `floor`. An unparsable version fails: an unknown version is
/// reported, never assumed good.
fn meets_node_floor(detected: &str, floor: u32) -> bool {
    first_number(detected).is_some_and(|major| major >= floor)
}

/// The first run of digits in `text`, as a number.
fn first_number(text: &str) -> Option<u32> {
    let digits: String = text
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

pub fn run(root: &Path) -> TaskResult {
    let mut missing = Vec::new();
    let floor = node_floor(root);
    for check in CHECKS {
        match detect(check) {
            Some(version) => {
                if check.name == "node"
                    && let Some(floor) = floor
                    && !meets_node_floor(&version, floor)
                {
                    println!(
                        "[FAIL] {:<28} {version} is below the required \
Node {floor} (engines.node)",
                        check.name
                    );
                    missing.push(check.name);
                    continue;
                }
                println!("[ok ] {:<28} {}", check.name, version);
            }
            None if check.required => {
                println!("[FAIL] {:<28} {}", check.name, check.hint);
                missing.push(check.name);
            }
            None => println!("[skip] {:<28} {}", check.name, check.hint),
        }
    }
    for check in FILES {
        if root.join(check.path).exists() {
            println!("[ok ] {:<28} {}", check.name, check.path);
        } else {
            println!("[FAIL] {:<28} {}", check.name, check.hint);
            missing.push(check.name);
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!("missing prerequisites: {}", missing.join(", ")))
    }
}

/// Runs the probes in order and returns the first accepted output's first
/// non-empty line.
fn detect(check: &Check) -> Option<String> {
    for probe in check.probes {
        let Some((program, args)) = probe.argv.split_first() else {
            continue;
        };
        let Ok(output) = Command::new(program).args(args).output() else {
            continue;
        };
        let text = willie_engine::text::decode_wsl_output(&output.stdout)
            + &willie_engine::text::decode_wsl_output(&output.stderr);
        if let Some(line) = accept(probe, output.status.success(), &text) {
            return Some(line);
        }
    }
    None
}

/// Decides whether a probe's output counts and extracts its headline.
fn accept(probe: &Probe, success: bool, text: &str) -> Option<String> {
    if !(success || probe.ignore_exit_status) {
        return None;
    }
    if let Some(needle) = probe.must_contain
        && !text.contains(needle)
    {
        return None;
    }
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_check_has_at_least_one_probe_and_a_hint() {
        for check in CHECKS {
            assert!(!check.probes.is_empty(), "{} has no probe", check.name);
            assert!(!check.hint.is_empty(), "{} has no hint", check.name);
        }
    }

    #[test]
    fn gnu_link_does_not_pass_for_the_msvc_linker() {
        let msvc = &CHECKS
            .iter()
            .find(|c| c.name.contains("link.exe"))
            .unwrap()
            .probes[1];
        assert!(
            accept(msvc, false, "link: missing operand after '/?'").is_none()
        );
        let banner =
            "Microsoft (R) Incremental Linker Version 14.50.1\n\nusage: LINK";
        assert_eq!(
            accept(msvc, false, banner).unwrap(),
            "Microsoft (R) Incremental Linker Version 14.50.1"
        );
    }

    #[test]
    fn failing_probe_is_rejected_unless_told_otherwise() {
        let strict = probe(&["x"]);
        assert!(accept(&strict, false, "1.0").is_none());
        assert_eq!(accept(&strict, true, "\n 1.0 \n").unwrap(), "1.0");
    }

    #[test]
    fn node_at_or_above_the_floor_passes_and_below_it_fails() {
        assert!(meets_node_floor("v24.18.0", 22));
        assert!(meets_node_floor("v22.0.0", 22));
        assert!(!meets_node_floor("v20.19.0", 22));
    }

    #[test]
    fn an_unreadable_node_version_fails_closed() {
        assert!(!meets_node_floor("not a version", 22));
        assert!(!meets_node_floor("", 22));
    }

    #[test]
    fn the_node_floor_is_read_from_the_engines_field() {
        let dir = std::env::temp_dir().join("willie-doctor-floor");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("package.json"),
            "{\n  \"engines\": {\n    \"node\": \">=22.0.0\"\n  }\n}\n",
        )
        .unwrap();
        assert_eq!(node_floor(&dir), Some(22));
    }

    #[test]
    fn every_file_check_has_a_relative_path_and_a_hint() {
        for check in FILES {
            assert!(!check.hint.is_empty(), "{} has no hint", check.name);
            assert!(
                !Path::new(check.path).is_absolute(),
                "{} must be relative to the workspace root",
                check.name
            );
        }
    }
}
