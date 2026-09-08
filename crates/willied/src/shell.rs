//! The interactive zsh launch: a `Shell` session runs a login shell in
//! the project's workspace under the same sandbox as an agent, with
//! Willie's own prompt (`ZDOTDIR`).

use std::{collections::BTreeMap, path::Path};

use willie_harness::{Launch, session_path};

/// The zsh binary the image installs. `session.create` refuses a shell
/// fail-closed when it is absent (a distro built before this feature).
pub const ZSH_PATH: &str = "/usr/bin/zsh";

/// Whether the zsh binary is present. Pure so a test can point it at a
/// path that never exists, independently of what the real image carries.
#[must_use]
pub fn zsh_present(path: &Path) -> bool {
    path.is_file()
}

/// The launch for an interactive zsh in `workspace`: the same allowlisted
/// env a harness gets (mirrors `willie_harness::Harness::launch`), plus
/// `ZDOTDIR` for Willie's own prompt and `WILLIE_WORKSPACE` for the
/// login shell's `cd` target.
#[must_use]
pub fn shell_launch(workspace: &Path, home: &Path) -> Launch {
    let mut env = BTreeMap::new();
    let path = session_path(home)
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(":");
    env.insert("PATH".to_owned(), path);
    env.insert("HOME".to_owned(), home.to_string_lossy().into_owned());
    env.insert(
        "USER".to_owned(),
        home.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "willie".to_owned()),
    );
    env.insert("TERM".to_owned(), "xterm-256color".to_owned());
    env.insert("COLORTERM".to_owned(), "truecolor".to_owned());
    env.insert("LANG".to_owned(), "C.UTF-8".to_owned());
    if let Ok(tz) = std::env::var("TZ")
        && !tz.is_empty()
    {
        env.insert("TZ".to_owned(), tz);
    }
    env.insert("ZDOTDIR".to_owned(), "/etc/willie/zsh".to_owned());
    env.insert(
        "WILLIE_WORKSPACE".to_owned(),
        workspace.to_string_lossy().into_owned(),
    );
    Launch {
        argv: vec!["/usr/bin/zsh".to_owned(), "-l".to_owned()],
        env,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_launch_runs_zsh_as_a_login_shell_in_the_workspace_with_the_allowlisted_env()
     {
        // TZ is read from this process; make the test deterministic.
        // SAFETY: tests run single-threaded here; nothing reads the env
        // concurrently.
        unsafe { std::env::set_var("TZ", "UTC") };
        let l = shell_launch(
            Path::new("/home/willie/projects/x"),
            Path::new("/home/willie"),
        );
        assert_eq!(l.argv, vec!["/usr/bin/zsh".to_owned(), "-l".to_owned()]);
        let keys: Vec<&str> = l.env.keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            vec![
                "COLORTERM",
                "HOME",
                "LANG",
                "PATH",
                "TERM",
                "TZ",
                "USER",
                "WILLIE_WORKSPACE",
                "ZDOTDIR",
            ]
        );
        assert_eq!(
            l.env["PATH"],
            "/home/willie/.local/bin:/usr/local/bin:/usr/bin:/bin"
        );
        assert_eq!(l.env["HOME"], "/home/willie");
        assert_eq!(l.env["USER"], "willie");
        assert_eq!(l.env["TERM"], "xterm-256color");
        assert_eq!(l.env["COLORTERM"], "truecolor");
        assert_eq!(l.env["LANG"], "C.UTF-8");
        assert_eq!(l.env["TZ"], "UTC");
        assert_eq!(l.env["ZDOTDIR"], "/etc/willie/zsh");
        assert_eq!(l.env["WILLIE_WORKSPACE"], "/home/willie/projects/x");
    }

    #[test]
    fn zsh_present_checks_a_real_file_and_nothing_else() {
        let dir = std::env::temp_dir()
            .join(format!("willie-shell-present-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let missing = dir.join("zsh");
        assert!(!zsh_present(&missing));
        std::fs::write(&missing, b"").unwrap();
        assert!(zsh_present(&missing));
        let a_dir = dir.join("a-directory");
        std::fs::create_dir_all(&a_dir).unwrap();
        assert!(!zsh_present(&a_dir));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
