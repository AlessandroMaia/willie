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

pub fn run(_root: &Path) -> TaskResult {
    let mut missing_required = Vec::new();
    for check in CHECKS {
        match detect(check) {
            Some(version) => println!("[ok ] {:<28} {}", check.name, version),
            None if check.required => {
                println!("[FAIL] {:<28} {}", check.name, check.hint);
                missing_required.push(check.name);
            }
            None => println!("[skip] {:<28} {}", check.name, check.hint),
        }
    }
    if missing_required.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "missing required tools: {}",
            missing_required.join(", ")
        ))
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
        let text = decode(&output.stdout) + &decode(&output.stderr);
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

/// Console tools on Windows mostly write UTF-8, but `wsl.exe` writes
/// UTF-16LE. Detect the latter by its BOM or by the NUL high bytes that
/// Latin text produces, and decode accordingly.
fn decode(bytes: &[u8]) -> String {
    let has_bom = bytes.starts_with(&[0xFF, 0xFE]);
    let high_zeroes =
        bytes.iter().skip(1).step_by(2).filter(|b| **b == 0).count();
    let looks_utf16 =
        has_bom || (bytes.len() >= 4 && high_zeroes > bytes.len() / 4);
    if looks_utf16 {
        let units: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes(*pair))
            .collect();
        String::from_utf16_lossy(&units)
            .trim_start_matches('\u{feff}')
            .to_owned()
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    }
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
    fn utf16le_output_is_decoded() {
        let text = "Versão do WSL: 2.6.1.0";
        let mut bytes = vec![0xFF, 0xFE];
        for unit in text.encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        assert_eq!(decode(&bytes), text);
    }

    #[test]
    fn utf8_output_is_left_alone() {
        assert_eq!(decode("cargo 1.98.0\n".as_bytes()), "cargo 1.98.0\n");
    }
}
