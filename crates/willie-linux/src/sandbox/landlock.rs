//! What Landlock adds on top of the mounts, as data.
//!
//! The mounts decide what a session sees; Landlock decides what it may
//! do there once it runs: read and execute anywhere, write only where
//! the plan mounted read-write. The writable set is derived from the
//! plan's own ops, so it has one source and cannot drift from the
//! mounts. `applicability` turns the ABI the kernel answers into the
//! rights to handle, or into not applying at all. The two kernel structs
//! and the access-right bits are declared here — stable ABI the pinned
//! libc does not carry — so the stage that applies them and the doctor
//! that reasons about them share one declaration, with no I/O and no
//! libc.

use serde::{Deserialize, Serialize};

use willie_core::sandbox::PathMode;

use super::{Op, Plan};

/// The `flags` value under which `landlock_create_ruleset` answers the
/// ABI version instead of creating a ruleset.
pub const LANDLOCK_CREATE_RULESET_VERSION: u32 = 1;
/// The rule type of `landlock_add_rule` that grants rights to everything
/// beneath a directory.
pub const LANDLOCK_RULE_PATH_BENEATH: u32 = 1;

// The filesystem access rights, by the kernel's names and bits. A right
// a ruleset handles and no rule grants is denied beneath every path.
pub const ACCESS_FS_EXECUTE: u64 = 1 << 0;
pub const ACCESS_FS_WRITE_FILE: u64 = 1 << 1;
pub const ACCESS_FS_READ_FILE: u64 = 1 << 2;
pub const ACCESS_FS_READ_DIR: u64 = 1 << 3;
pub const ACCESS_FS_REMOVE_DIR: u64 = 1 << 4;
pub const ACCESS_FS_REMOVE_FILE: u64 = 1 << 5;
pub const ACCESS_FS_MAKE_CHAR: u64 = 1 << 6;
pub const ACCESS_FS_MAKE_DIR: u64 = 1 << 7;
pub const ACCESS_FS_MAKE_REG: u64 = 1 << 8;
pub const ACCESS_FS_MAKE_SOCK: u64 = 1 << 9;
pub const ACCESS_FS_MAKE_FIFO: u64 = 1 << 10;
pub const ACCESS_FS_MAKE_BLOCK: u64 = 1 << 11;
pub const ACCESS_FS_MAKE_SYM: u64 = 1 << 12;
/// Linking or renaming across directories. First handled by ABI 2.
pub const ACCESS_FS_REFER: u64 = 1 << 13;
/// Truncating a file. First handled by ABI 3.
pub const ACCESS_FS_TRUNCATE: u64 = 1 << 14;

/// The `/` rule's rights: what a session may do anywhere. No write bit
/// is among them, so a path the plan did not mount read-write stays
/// read-only to the session even where a mount got it wrong.
pub const READ_EXEC: u64 =
    ACCESS_FS_EXECUTE | ACCESS_FS_READ_FILE | ACCESS_FS_READ_DIR;

/// Every filesystem right ABI 2 handles: bits 0 through `REFER`.
pub const ALL_ABI2: u64 = (1 << 14) - 1;

/// `landlock_ruleset_attr`: the rights a ruleset handles.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RulesetAttr {
    pub handled_access_fs: u64,
}

/// `landlock_path_beneath_attr`: the rights granted beneath the
/// directory `parent_fd` is open on. Packed, as the kernel declares it:
/// twelve bytes, the descriptor right after the rights with no padding,
/// so `landlock_add_rule` reads exactly what was written.
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PathBeneathAttr {
    pub allowed_access: u64,
    pub parent_fd: i32,
}

/// What Landlock adds to the mounts: read and execute under `/`, write
/// only at these paths. Derived from the plan, so the writable set has
/// one source; the read set is implicit — a single rule on `/`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rules {
    #[serde(default)]
    pub write: Vec<String>,
}

/// The trees the base sets up writable whatever the policy says: the
/// fresh temporary directory and the device nodes. Not in the plan's
/// ops, because the base is not policy, so they are granted here.
const BASE_WRITABLE: [&str; 2] = ["/tmp", "/dev"];

/// The writable set of `plan`: the destination of every private
/// filesystem and every read-write bind, in the order the ops mount
/// them, plus the trees the base sets up writable whatever the policy
/// says. Each path once.
pub fn rules(plan: &Plan) -> Rules {
    let mut write = Vec::new();
    for op in &plan.ops {
        match op {
            Op::Tmpfs { dest, .. }
            | Op::Bind {
                dest,
                mode: PathMode::Rw,
                ..
            } => grant(&mut write, dest),
            Op::Bind { .. } | Op::Symlink { .. } => {}
        }
    }
    for tree in BASE_WRITABLE {
        grant(&mut write, tree);
    }
    Rules { write }
}

/// A `Vec` with a linear scan: the set is a handful of paths, and its
/// order is the plan's, which a set would lose.
fn grant(write: &mut Vec<String>, path: &str) {
    if !write.iter().any(|p| p == path) {
        write.push(path.to_owned());
    }
}

/// What the ABI the kernel answered means for the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Apply {
    /// Apply a ruleset that handles these rights.
    Ruleset { handled: u64 },
    /// Do not apply: the session runs on the mounts alone and the report
    /// names the mechanism unavailable.
    Unavailable,
}

/// The decision for an ABI: below 2 nothing is applied, because ABI 1
/// lacks `REFER` and every cross-directory rename would be denied, which
/// breaks git; 2 handles every right through `REFER`; 3 and above add
/// `TRUNCATE`. Zero and a negative errno are "no Landlock".
pub fn applicability(abi: i32) -> Apply {
    match abi {
        i32::MIN..=1 => Apply::Unavailable,
        2 => Apply::Ruleset { handled: ALL_ABI2 },
        _ => Apply::Ruleset {
            handled: ALL_ABI2 | ACCESS_FS_TRUNCATE,
        },
    }
}

#[cfg(test)]
mod tests {
    use std::mem::{align_of, size_of};

    use willie_core::sandbox::{CapabilitySet, ExtraPath, PathMode};

    use super::*;
    use crate::sandbox::fixtures::*;

    /// One rule per read-write mount and per private filesystem, plus the
    /// base's two writable trees; the read-only mounts and the binaries
    /// are reached through the `/` rule alone.
    #[test]
    fn rules_write_set_is_the_rw_mounts_plus_tmp_and_dev() {
        let (spec, plan) = planned(CapabilitySet {
            extra_paths: vec![
                ExtraPath {
                    path: "/srv/shared".into(),
                    mode: PathMode::Ro,
                },
                ExtraPath {
                    path: "/mnt/c/out".into(),
                    mode: PathMode::Rw,
                },
            ],
            ..all_on()
        });

        let write = rules(&plan).write;

        for granted in [
            HOME,
            WS,
            "/home/willie/.willie/agent-state/claude",
            "/home/willie/.npm",
            "/home/willie/.nuget",
            "/home/willie/.cache",
            "/mnt/c/out",
            "/tmp",
            "/dev",
        ] {
            assert!(write.iter().any(|p| p == granted), "{granted}: {write:?}");
        }
        for read_only in [
            "/home/willie/.local",
            "/home/willie/.dotnet",
            "/home/willie/.gitconfig",
            INNER,
            "/srv/shared",
        ] {
            assert!(!write.iter().any(|p| p == read_only), "{read_only}");
        }
        assert!(
            !write
                .iter()
                .any(|p| p.contains(&spec.project_id.to_string())),
            "the cache source, not its mount, carries the project id: {write:?}"
        );
        let mut unique = write.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), write.len(), "{write:?}");
    }

    /// The workspace is the one read-write mount every policy has, so it
    /// is writable under the tightest policy too; the private home and
    /// the base's trees come with it.
    #[test]
    fn the_workspace_is_writable_under_every_policy() {
        let (_, plan) = planned(CapabilitySet::default());

        assert_eq!(rules(&plan).write, vec![HOME, WS, "/tmp", "/dev"]);
    }

    /// A path the policy names that the base already makes writable, or
    /// names twice, is one rule: the kernel would accept the repeat, but
    /// the set is shown to people.
    #[test]
    fn a_path_mounted_twice_or_already_writable_is_listed_once() {
        let (_, plan) = planned(CapabilitySet {
            extra_paths: vec![
                ExtraPath {
                    path: "/tmp".into(),
                    mode: PathMode::Rw,
                },
                ExtraPath {
                    path: WS.into(),
                    mode: PathMode::Rw,
                },
            ],
            ..CapabilitySet::default()
        });

        assert_eq!(rules(&plan).write, vec![HOME, WS, "/tmp", "/dev"]);
    }

    /// ABI 1 is treated as no Landlock: without `REFER` a rename across
    /// directories is denied and git stops working, so a ruleset would
    /// break the workload it is meant to contain.
    #[test]
    fn applicability_follows_the_abi() {
        for abi in [-1, 0, 1] {
            assert_eq!(applicability(abi), Apply::Unavailable, "{abi}");
        }
        assert_eq!(applicability(2), Apply::Ruleset { handled: ALL_ABI2 });
        for abi in [3, 7] {
            assert_eq!(
                applicability(abi),
                Apply::Ruleset {
                    handled: ALL_ABI2 | ACCESS_FS_TRUNCATE
                },
                "{abi}"
            );
        }
        assert_eq!(ALL_ABI2, 0x3FFF);
        assert_ne!(ALL_ABI2 & ACCESS_FS_REFER, 0);
        assert_eq!(ALL_ABI2 & ACCESS_FS_TRUNCATE, 0);
    }

    /// The stage hands these to the kernel by address and size, so they
    /// must be laid out exactly as the kernel declares them: the
    /// path-beneath attribute is packed, a 64-bit word followed directly
    /// by a 32-bit descriptor.
    #[test]
    fn the_path_beneath_attr_is_twelve_bytes_as_the_kernel_declares_it() {
        assert_eq!(size_of::<PathBeneathAttr>(), 12);
        assert_eq!(align_of::<PathBeneathAttr>(), 1);
        assert_eq!(size_of::<RulesetAttr>(), 8);
    }

    /// The `/` rule grants reading and executing, and nothing that
    /// changes the filesystem.
    #[test]
    fn read_exec_is_the_three_read_rights() {
        assert_eq!(READ_EXEC, 0b1101);
        assert_eq!(
            READ_EXEC
                & !(ACCESS_FS_EXECUTE
                    | ACCESS_FS_READ_FILE
                    | ACCESS_FS_READ_DIR),
            0
        );
        assert_eq!(READ_EXEC & ACCESS_FS_WRITE_FILE, 0);
    }

    /// The rules travel in the request line the outer stage writes to the
    /// inner one; a line without them is an empty set, not a refusal.
    #[test]
    fn rules_round_trip_and_default_to_empty() {
        let rules = Rules {
            write: vec![WS.into(), "/tmp".into()],
        };

        let json = serde_json::to_string(&rules).expect("serialises");
        let back: Rules = serde_json::from_str(&json).expect("parses");
        assert_eq!(back, rules);

        let empty: Rules = serde_json::from_str("{}").expect("parses");
        assert_eq!(empty, Rules::default());
    }
}
