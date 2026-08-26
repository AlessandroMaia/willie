//! Prefixed identifiers such as `proj_01J…` and `sess_01J…`.
//!
//! The prefix makes an id self-describing wherever it shows up (logs,
//! socket names, directories); the ULID part sorts by creation time and is
//! safe in file names.

use std::{fmt, hash::Hash, marker::PhantomData, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use ulid::Ulid;

/// Marker trait naming the prefix of an id family.
pub trait Kind:
    fmt::Debug + Clone + Copy + PartialEq + Eq + Hash + PartialOrd + Ord
{
    /// Prefix without the trailing underscore, e.g. `proj`.
    const PREFIX: &'static str;
}

macro_rules! kind {
    ($(#[$meta:meta])* $name:ident => $prefix:literal) => {
        $(#[$meta])*
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord,
        )]
        pub struct $name;

        impl Kind for $name {
            const PREFIX: &'static str = $prefix;
        }
    };
}

kind!(
    /// A registered workspace/project.
    Project => "proj"
);
kind!(
    /// One execution of a harness in a project.
    Session => "sess"
);
kind!(
    /// A managed tool installed in the distro.
    Tool => "tool"
);
kind!(
    /// A configuration profile.
    Profile => "prof"
);

/// Identifier of a project.
pub type ProjectId = Id<Project>;
/// Identifier of a session.
pub type SessionId = Id<Session>;
/// Identifier of a managed tool.
pub type ToolId = Id<Tool>;
/// Identifier of a configuration profile.
pub type ProfileId = Id<Profile>;

/// A typed, prefixed ULID.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Id<K: Kind> {
    ulid: Ulid,
    _kind: PhantomData<K>,
}

impl<K: Kind> Id<K> {
    /// Generates a fresh id for the current instant.
    #[must_use]
    pub fn new() -> Self {
        Self::from_ulid(Ulid::new())
    }

    /// Wraps an existing ULID.
    #[must_use]
    pub const fn from_ulid(ulid: Ulid) -> Self {
        Self {
            ulid,
            _kind: PhantomData,
        }
    }

    /// The prefix used by this id family, without the underscore.
    #[must_use]
    pub const fn prefix() -> &'static str {
        K::PREFIX
    }

    /// The underlying ULID.
    #[must_use]
    pub const fn ulid(&self) -> Ulid {
        self.ulid
    }
}

impl<K: Kind> Default for Id<K> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: Kind> fmt::Display for Id<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}_{}", K::PREFIX, self.ulid)
    }
}

impl<K: Kind> fmt::Debug for Id<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// Failure to parse a prefixed id from text.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseIdError {
    /// The text did not start with the expected `prefix_`.
    #[error("expected an id starting with `{expected}_`, got `{input}`")]
    WrongPrefix {
        /// Prefix this id family requires.
        expected: &'static str,
        /// The offending input.
        input: String,
    },
    /// The part after the prefix is not a valid ULID.
    #[error("`{0}` does not contain a valid ULID")]
    InvalidUlid(String),
}

impl<K: Kind> FromStr for Id<K> {
    type Err = ParseIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let rest = s
            .strip_prefix(K::PREFIX)
            .and_then(|r| r.strip_prefix('_'))
            .ok_or_else(|| ParseIdError::WrongPrefix {
                expected: K::PREFIX,
                input: s.to_owned(),
            })?;
        let ulid = Ulid::from_string(rest)
            .map_err(|_| ParseIdError::InvalidUlid(s.to_owned()))?;
        Ok(Self::from_ulid(ulid))
    }
}

impl<K: Kind> Serialize for Id<K> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de, K: Kind> Deserialize<'de> for Id<K> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_uses_the_family_prefix() {
        let id = SessionId::new();
        assert!(id.to_string().starts_with("sess_"));
        assert!(ProjectId::new().to_string().starts_with("proj_"));
    }

    #[test]
    fn text_round_trips_through_parse() {
        let id = ProjectId::new();
        let parsed: ProjectId = id.to_string().parse().unwrap();
        assert_eq!(parsed, id);
    }

    #[test]
    fn parsing_rejects_another_family() {
        let session = SessionId::new().to_string();
        let err = session.parse::<ProjectId>().unwrap_err();
        assert!(matches!(
            err,
            ParseIdError::WrongPrefix {
                expected: "proj",
                ..
            }
        ));
    }

    #[test]
    fn parsing_rejects_garbage_after_the_prefix() {
        let err = "sess_not-a-ulid".parse::<SessionId>().unwrap_err();
        assert!(matches!(err, ParseIdError::InvalidUlid(_)));
    }

    #[test]
    fn serde_uses_the_prefixed_string_form() {
        let id = ToolId::new();
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, format!("\"{id}\""));
        let back: ToolId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, id);
    }
}
