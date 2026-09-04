//! The namespace helper's argument vector, as data. The base is what
//! every session gets and no policy can change; the plan follows it.

use willie_core::sandbox::PathMode;

use super::{Op, Plan};

/// Where the image installs the helper.
pub const BWRAP: &str = "/usr/bin/bwrap";

/// Files under `/etc` a process needs to resolve names, users, time and
/// certificates. Never the directory itself: it holds the sudoers and
/// the shadow file.
const ETC: [&str; 7] = [
    "resolv.conf",
    "hosts",
    "passwd",
    "group",
    "nsswitch.conf",
    "ssl",
    "alternatives",
];

/// Under `/etc` as well, but a slim image may lack them.
const ETC_OPTIONAL: [&str; 5] = [
    "ld.so.cache",
    "localtime",
    "terminfo",
    "gitconfig",
    "os-release",
];

/// Privilege escalation helpers hidden behind the null device. A user
/// namespace already makes them inert; masking them makes that visible.
const MASKED: [&str; 1] = ["/usr/bin/sudo"];

/// The merged-usr layout: the top-level directories are links into
/// `/usr`, as they are in the image.
const USR_LINKS: [(&str, &str); 4] = [
    ("usr/bin", "/bin"),
    ("usr/lib", "/lib"),
    ("usr/lib64", "/lib64"),
    ("usr/sbin", "/sbin"),
];

fn push(v: &mut Vec<String>, args: &[&str]) {
    v.extend(args.iter().map(|a| (*a).to_owned()));
}

/// The complete command line: helper, base, plan, working directory,
/// separator, harness. The network stays shared, because the harness is
/// an API client; a new terminal session is not requested, because it
/// would detach the harness from the terminal that is the whole point.
#[must_use]
pub fn argv(plan: &Plan) -> Vec<String> {
    let mut v = vec![BWRAP.to_owned()];
    push(
        &mut v,
        &[
            "--unshare-user",
            "--unshare-pid",
            "--unshare-ipc",
            "--unshare-uts",
            "--die-with-parent",
        ],
    );
    // Mounts do not clear an environment. Clearing here rather than
    // relying on how the supervisor was started keeps the boundary
    // self-contained, and the interop variables are why that matters:
    // the kernel holds the interop interpreter open, so its address
    // plus one wrong bind is an escape (decision 0016).
    push(&mut v, &["--clearenv"]);
    for (key, value) in &plan.env {
        push(&mut v, &["--setenv", key, value]);
    }
    push(&mut v, &["--ro-bind", "/usr", "/usr"]);
    for (target, link) in USR_LINKS {
        push(&mut v, &["--symlink", target, link]);
    }
    for name in ETC {
        let path = format!("/etc/{name}");
        push(&mut v, &["--ro-bind", &path, &path]);
    }
    for name in ETC_OPTIONAL {
        let path = format!("/etc/{name}");
        push(&mut v, &["--ro-bind-try", &path, &path]);
    }
    push(
        &mut v,
        &["--proc", "/proc", "--dev", "/dev", "--tmpfs", "/tmp"],
    );
    for masked in MASKED {
        push(&mut v, &["--ro-bind", "/dev/null", masked]);
    }
    for op in &plan.ops {
        render(op, &mut v);
    }
    push(&mut v, &["--chdir", &plan.workspace, "--"]);
    v.extend(plan.argv.iter().cloned());
    v
}

fn render(op: &Op, v: &mut Vec<String>) {
    match op {
        Op::Tmpfs { dest, perms } => {
            push(v, &["--perms", perms, "--tmpfs", dest]);
        }
        Op::Bind {
            src,
            dest,
            mode,
            optional,
        } => {
            let flag = match (mode, optional) {
                (PathMode::Ro, false) => "--ro-bind",
                (PathMode::Ro, true) => "--ro-bind-try",
                (PathMode::Rw, false) => "--bind",
                (PathMode::Rw, true) => "--bind-try",
            };
            push(v, &[flag, src, dest]);
        }
        Op::Symlink { target, link } => {
            push(v, &["--symlink", target, link]);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use willie_core::sandbox::PathMode;

    use super::*;
    use crate::sandbox::{Op, Plan};

    const HOME: &str = "/home/willie";
    const WS: &str = "/home/willie/projects/x";
    const BIN: &str = "/home/willie/.local/bin/claude";

    fn plan() -> Plan {
        Plan {
            home: HOME.into(),
            workspace: WS.into(),
            argv: vec![BIN.into(), "--continue".into()],
            env: BTreeMap::new(),
            ensure_dirs: Vec::new(),
            ops: vec![
                Op::Tmpfs {
                    dest: HOME.into(),
                    perms: "0700",
                },
                Op::Bind {
                    src: WS.into(),
                    dest: WS.into(),
                    mode: PathMode::Rw,
                    optional: false,
                },
                Op::Bind {
                    src: "/home/willie/.local".into(),
                    dest: "/home/willie/.local".into(),
                    mode: PathMode::Ro,
                    optional: true,
                },
                Op::Symlink {
                    target: ".willie/agent-state/claude/dot-claude".into(),
                    link: "/home/willie/.claude".into(),
                },
            ],
        }
    }

    /// Where a run of arguments starts in `v`, if anywhere.
    fn index_of(v: &[String], run: &[&str]) -> Option<usize> {
        v.windows(run.len())
            .position(|w| w.iter().zip(run).all(|(a, b)| a == b))
    }

    fn has(v: &[String], run: &[&str]) -> bool {
        index_of(v, run).is_some()
    }

    /// How many values each option takes. The vector is only as good as
    /// its shape: an option with a missing value would silently turn the
    /// next flag into a path.
    fn arity(option: &str) -> Option<usize> {
        Some(match option {
            "--unshare-user" | "--unshare-pid" | "--unshare-ipc"
            | "--unshare-uts" | "--die-with-parent" => 0,
            "--clearenv" => 0,
            "--proc" | "--dev" | "--tmpfs" | "--chdir" | "--perms" => 1,
            "--ro-bind" | "--ro-bind-try" | "--bind" | "--bind-try"
            | "--symlink" => 2,
            "--setenv" => 2,
            _ => return None,
        })
    }

    /// Splits the vector into (options, command) after checking every
    /// option's arity.
    fn parse(v: &[String]) -> (Vec<&str>, Vec<&str>) {
        assert_eq!(v[0], BWRAP);
        let mut i = 1;
        let mut options = Vec::new();
        while v[i] != "--" {
            let n = arity(&v[i]).unwrap_or_else(|| panic!("unknown {}", v[i]));
            options.push(v[i].as_str());
            for k in 1..=n {
                assert!(!v[i + k].starts_with("--"), "{} lacks a value", v[i]);
            }
            i += 1 + n;
        }
        (options, v[i + 1..].iter().map(String::as_str).collect())
    }

    #[test]
    fn the_vector_starts_with_the_helper_and_unshares_everything_but_the_network()
     {
        let v = argv(&plan());

        assert_eq!(v[0], BWRAP);
        for flag in [
            "--unshare-user",
            "--unshare-pid",
            "--unshare-ipc",
            "--unshare-uts",
            "--die-with-parent",
        ] {
            assert!(v.contains(&flag.to_owned()), "{flag}");
        }
        for absent in [
            "--unshare-net",
            "--share-net",
            "--unshare-all",
            "--new-session",
        ] {
            assert!(!v.contains(&absent.to_owned()), "{absent}");
        }
    }

    #[test]
    fn the_system_tree_is_read_only_with_the_merged_usr_links() {
        let v = argv(&plan());

        assert!(has(&v, &["--ro-bind", "/usr", "/usr"]));
        assert!(!has(&v, &["--bind", "/usr", "/usr"]));
        for (target, link) in [
            ("usr/bin", "/bin"),
            ("usr/lib", "/lib"),
            ("usr/lib64", "/lib64"),
            ("usr/sbin", "/sbin"),
        ] {
            assert!(has(&v, &["--symlink", target, link]), "{link}");
        }
    }

    #[test]
    fn the_process_device_and_temporary_trees_are_fresh() {
        let v = argv(&plan());

        assert!(has(&v, &["--proc", "/proc"]));
        assert!(has(&v, &["--dev", "/dev"]));
        assert!(has(&v, &["--tmpfs", "/tmp"]));
    }

    /// `/etc` is never bound whole: it holds the sudoers and the shadow
    /// file. The files a process needs to resolve names, users, time and
    /// certificates come one by one, and the ones a slim image may lack
    /// are allowed to be absent.
    #[test]
    fn the_selected_etc_files_come_one_by_one_and_the_optional_ones_may_be_absent()
     {
        let v = argv(&plan());

        for name in [
            "resolv.conf",
            "hosts",
            "passwd",
            "group",
            "nsswitch.conf",
            "ssl",
            "alternatives",
        ] {
            let p = format!("/etc/{name}");
            assert!(has(&v, &["--ro-bind", &p, &p]), "{p}");
        }
        for name in [
            "ld.so.cache",
            "localtime",
            "terminfo",
            "gitconfig",
            "os-release",
        ] {
            let p = format!("/etc/{name}");
            assert!(has(&v, &["--ro-bind-try", &p, &p]), "{p}");
        }
        assert!(!v.contains(&"/etc".to_owned()));
        for secret in ["/etc/shadow", "/etc/sudoers", "/etc/sudoers.d"] {
            assert!(!v.contains(&secret.to_owned()), "{secret}");
        }
    }

    #[test]
    fn sudo_is_masked_behind_the_null_device() {
        let v = argv(&plan());

        assert!(has(&v, &["--ro-bind", "/dev/null", "/usr/bin/sudo"]));
    }

    /// The base comes first, then the plan in its own order, then the
    /// working directory, then the separator and the harness untouched.
    #[test]
    fn the_plan_follows_the_base_in_order_then_chdir_then_the_harness() {
        let v = argv(&plan());

        let usr = index_of(&v, &["--ro-bind", "/usr", "/usr"]).unwrap();
        let home = index_of(&v, &["--perms", "0700", "--tmpfs", HOME]).unwrap();
        let ws = index_of(&v, &["--bind", WS, WS]).unwrap();
        let local = index_of(
            &v,
            &[
                "--ro-bind-try",
                "/home/willie/.local",
                "/home/willie/.local",
            ],
        )
        .unwrap();
        let link = index_of(
            &v,
            &[
                "--symlink",
                ".willie/agent-state/claude/dot-claude",
                "/home/willie/.claude",
            ],
        )
        .unwrap();
        let chdir = index_of(&v, &["--chdir", WS]).unwrap();
        assert!(usr < home && home < ws && ws < local);
        assert!(local < link && link < chdir);
        let tail: Vec<&str> =
            v[chdir + 2..].iter().map(String::as_str).collect();
        assert_eq!(tail, ["--", BIN, "--continue"]);
    }

    #[test]
    fn every_option_has_its_arity_and_the_command_follows_the_separator() {
        let v = argv(&plan());

        let (options, command) = parse(&v);
        assert!(options.len() > 10);
        assert_eq!(command, [BIN, "--continue"]);
    }

    /// What the base never names, so the plan alone decides what of the
    /// machine a session sees.
    #[test]
    fn the_base_names_neither_the_windows_drives_nor_the_interop_nor_willie() {
        let mut bare = plan();
        bare.ops.clear();
        bare.env = BTreeMap::new();
        bare.workspace = "/w".into();
        bare.argv = vec!["/w/h".into()];
        let v = argv(&bare);

        for arg in &v[1..] {
            for forbidden in
                ["/mnt", "/init", "/run", "/var", "/home", "/opt", "/root"]
            {
                assert!(!arg.starts_with(forbidden), "{arg}");
            }
        }
    }

    /// `--clearenv` wipes what `--setenv` then sets, so the order is not
    /// cosmetic: a `--setenv` before it would be erased.
    #[test]
    fn the_environment_is_cleared_before_the_allowlist_is_set() {
        let mut p = plan();
        p.env = BTreeMap::from([
            ("HOME".to_owned(), HOME.to_owned()),
            ("TERM".to_owned(), "xterm-256color".to_owned()),
        ]);

        let v = argv(&p);

        let clear = v
            .iter()
            .position(|a| a == "--clearenv")
            .expect("the environment is cleared");
        let first_set = v
            .iter()
            .position(|a| a == "--setenv")
            .expect("the allowlist is set");
        assert!(clear < first_set, "cleared after being set");
        assert!(has(&v, &["--setenv", "HOME", HOME]));
        assert!(has(&v, &["--setenv", "TERM", "xterm-256color"]));
        assert_eq!(v.iter().filter(|a| *a == "--setenv").count(), 2);
    }

    /// The one thing an inherited environment buys an attacker: the
    /// interop variables. The builder reads the plan and nothing else,
    /// so what is asserted here is that the vector carries the plan's
    /// environment and names none of them; that an inherited one cannot
    /// survive is `--clearenv`, asserted above.
    #[test]
    fn the_vector_sets_the_plan_environment_and_names_no_interop_variable() {
        let mut p = plan();
        p.env = BTreeMap::from([("PATH".to_owned(), "/usr/bin".to_owned())]);

        let v = argv(&p);

        assert!(v.contains(&"--clearenv".to_owned()));
        assert_eq!(v.iter().filter(|a| *a == "--setenv").count(), 1);
        for leaked in ["WSL_INTEROP", "WSL_DISTRO_NAME", "WSLENV", "WT_SESSION"]
        {
            assert!(!v.iter().any(|a| a.contains(leaked)), "{leaked}");
        }
    }

    /// A spec with no environment still gets an empty one, never the
    /// launcher's.
    #[test]
    fn an_empty_environment_is_still_cleared() {
        let mut p = plan();
        p.env = BTreeMap::new();

        let v = argv(&p);

        assert!(v.contains(&"--clearenv".to_owned()));
        assert!(!v.contains(&"--setenv".to_owned()));
    }
}
