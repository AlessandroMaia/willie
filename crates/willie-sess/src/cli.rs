//! Command line of the supervisor. Portable on purpose: parsing has no
//! Linux in it, so its tests run wherever `cargo test` runs.

/// What the command line asked for.
#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Version,
    /// Run the session described by the spec file.
    Run {
        spec: String,
    },
    /// Usage error, with the reason to print on stderr.
    Usage(String),
}

pub const USAGE: &str =
    "usage: willie-sess run --spec <path> | willie-sess --version";

/// Parse the arguments after the program name.
pub fn parse(args: &[String]) -> Command {
    let strs: Vec<&str> = args.iter().map(String::as_str).collect();
    match strs.as_slice() {
        ["--version"] => Command::Version,
        ["run", "--spec", spec] => Command::Run {
            spec: (*spec).to_owned(),
        },
        ["run", ..] => Command::Usage("run takes exactly --spec <path>".into()),
        [other, ..] => Command::Usage(format!("unknown command `{other}`")),
        [] => Command::Usage("no command".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn version_parses_alone() {
        assert_eq!(parse(&args(&["--version"])), Command::Version);
    }

    #[test]
    fn run_needs_exactly_a_spec_path() {
        assert_eq!(
            parse(&args(&[
                "run",
                "--spec",
                "/var/lib/willie/sessions/s/spec.json"
            ])),
            Command::Run {
                spec: "/var/lib/willie/sessions/s/spec.json".into()
            }
        );
        assert!(matches!(parse(&args(&["run"])), Command::Usage(_)));
        assert!(matches!(
            parse(&args(&["run", "--spec"])),
            Command::Usage(_)
        ));
        assert!(matches!(
            parse(&args(&["run", "--spec", "/a", "extra"])),
            Command::Usage(_)
        ));
        assert!(matches!(parse(&args(&[])), Command::Usage(_)));
        assert!(matches!(parse(&args(&["serve"])), Command::Usage(_)));
    }
}
