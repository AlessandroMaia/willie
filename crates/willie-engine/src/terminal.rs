//! Opening a session in a terminal. A session's real client is a Windows
//! Terminal tab running `willie attach <id>` through `wsl.exe`; when
//! Windows Terminal is absent, the same line runs in a new console. The
//! argv builders are pure so the exact command is testable on any host;
//! only the spawn is Windows-specific.

use willie_core::id::SessionId;

use crate::wsl;

/// The Linux path of the CLI inside the distribution.
const WILLIE_BIN: &str = "/opt/willie/bin/willie";

/// Why a session could not be opened in any terminal.
#[derive(Debug, thiserror::Error)]
#[error("could not open a terminal for the session: {0}")]
pub struct TerminalError(String);

/// `wsl.exe` arguments that attach a terminal to session `id`.
#[must_use]
pub fn attach_argv(id: &str, _title: &str) -> Vec<String> {
    vec![
        "-d".into(),
        wsl::DISTRO_NAME.to_owned(),
        "--user".into(),
        "willie".into(),
        "--exec".into(),
        WILLIE_BIN.into(),
        "attach".into(),
        id.to_owned(),
    ]
}

/// `wt.exe` arguments: a titled new tab running the attach line.
#[must_use]
pub fn wt_argv(id: &str, title: &str) -> Vec<String> {
    let mut v = vec![
        "-w".into(),
        "0".into(),
        "new-tab".into(),
        "--title".into(),
        title.to_owned(),
        "--".into(),
        "wsl.exe".into(),
    ];
    v.extend(attach_argv(id, title));
    v
}

/// Open a Windows Terminal tab for the session, falling back to a new
/// console if `wt.exe` cannot be launched. Returns once the launcher has
/// been spawned (the tab lives on its own), never blocking on the harness.
pub fn open_tab(id: SessionId, title: &str) -> Result<(), TerminalError> {
    let id = id.to_string();
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
        // The wt.exe launcher itself needs no window; the tab it opens is
        // Windows Terminal's own window.
        if let Some(wt) = locate_wt()
            && std::process::Command::new(wt)
                .args(wt_argv(&id, title))
                .creation_flags(CREATE_NO_WINDOW)
                .spawn()
                .is_ok()
        {
            return Ok(());
        }
        // Fallback: the same attach line in a fresh console window.
        std::process::Command::new("wsl.exe")
            .args(attach_argv(&id, title))
            .creation_flags(CREATE_NEW_CONSOLE)
            .spawn()
            .map(|_| ())
            .map_err(|e| TerminalError(e.to_string()))
    }
    #[cfg(not(windows))]
    {
        let _ = (id, title);
        Err(TerminalError("terminals are Windows-only".into()))
    }
}

/// `wt.exe` on `PATH` or under `%LOCALAPPDATA%\Microsoft\WindowsApps`.
#[cfg(windows)]
fn locate_wt() -> Option<std::path::PathBuf> {
    use std::os::windows::process::CommandExt;
    if std::process::Command::new("wt.exe")
        .arg("--version")
        .creation_flags(0x0800_0000)
        .output()
        .is_ok()
    {
        return Some(std::path::PathBuf::from("wt.exe"));
    }
    let local = std::env::var_os("LOCALAPPDATA")?;
    let p = std::path::Path::new(&local)
        .join("Microsoft")
        .join("WindowsApps")
        .join("wt.exe");
    p.is_file().then_some(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_attach_argv_targets_the_distro_user_and_the_willie_binary() {
        let a = attach_argv("sess_01J", "my project");
        // `wsl.exe -d <distro> --user willie --exec /opt/willie/bin/willie attach sess_01J`
        assert_eq!(a[0], "-d");
        assert_eq!(a[1], wsl::DISTRO_NAME);
        assert!(a.windows(2).any(|w| w == ["--user", "willie"]));
        assert!(
            a.windows(2)
                .any(|w| w == ["--exec", "/opt/willie/bin/willie"])
        );
        assert_eq!(a[a.len() - 2], "attach");
        assert_eq!(a[a.len() - 1], "sess_01J");
    }

    #[test]
    fn the_wt_argv_wraps_the_attach_line_with_a_title() {
        let w = wt_argv("sess_01J", "my project");
        assert_eq!(&w[0..4], &["-w", "0", "new-tab", "--title"]);
        assert_eq!(w[4], "my project");
        assert!(w.contains(&"--".to_owned()));
        assert!(w.contains(&"wsl.exe".to_owned()));
        assert!(w.ends_with(&["attach".to_owned(), "sess_01J".to_owned()]));
    }
}
