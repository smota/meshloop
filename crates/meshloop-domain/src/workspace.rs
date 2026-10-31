//! Portable, path-free workspace lifecycle records. No filesystem or process access.
use crate::evidence::AttemptId;
use serde::{Deserialize, Serialize};

macro_rules! text_type {
    ($name:ident, $check:expr) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);
        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, String> {
                Self::try_from(value.into())
            }
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
        impl TryFrom<String> for $name {
            type Error = String;
            fn try_from(value: String) -> Result<Self, Self::Error> {
                if ($check)(&value) {
                    Ok(Self(value))
                } else {
                    Err(concat!("invalid ", stringify!($name)).into())
                }
            }
        }
        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }
    };
}
fn identifier(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
fn digest(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
text_type!(LifecycleId, identifier);
text_type!(PlanHash, digest);
text_type!(NativeIdentity, digest);
text_type!(RootAlias, |s: &str| !s.is_empty()
    && s.len() <= 64
    && s.bytes().all(|b| b.is_ascii_lowercase()
        || b.is_ascii_digit()
        || b == b'-'));
fn relative(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 4096
        && !s.contains(['\\', ':'])
        && !s.chars().any(char::is_control)
        && s.split('/').all(|part| {
            let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
            !part.is_empty()
                && part != "."
                && part != ".."
                && !part.ends_with(['.', ' '])
                && !part.contains(['<', '>', '"', '|', '?', '*'])
                && !matches!(
                    stem.as_str(),
                    "CON"
                        | "PRN"
                        | "AUX"
                        | "NUL"
                        | "COM1"
                        | "COM2"
                        | "COM3"
                        | "COM4"
                        | "COM5"
                        | "COM6"
                        | "COM7"
                        | "COM8"
                        | "COM9"
                        | "LPT1"
                        | "LPT2"
                        | "LPT3"
                        | "LPT4"
                        | "LPT5"
                        | "LPT6"
                        | "LPT7"
                        | "LPT8"
                        | "LPT9"
                )
        })
}
text_type!(RelativePath, relative);
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryRecord {
    pub id: LifecycleId,
    pub common_dir_identity: NativeIdentity,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RootRecord {
    pub repository_id: LifecycleId,
    pub alias: RootAlias,
    pub native_identity: NativeIdentity,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerationRecord {
    pub id: LifecycleId,
    pub repository_id: LifecycleId,
    pub graph_id: LifecycleId,
    pub ordinal: u32,
    pub accepted_plan_hash: PlanHash,
    pub root_alias: RootAlias,
    pub relative_path: RelativePath,
    pub layout_version: u32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArtifactKind {
    Plan,
    Integration,
    Attempt,
    Deliverable,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactRecord {
    pub id: LifecycleId,
    pub generation_id: LifecycleId,
    pub kind: ArtifactKind,
    pub root_alias: RootAlias,
    pub relative_path: RelativePath,
    pub attempt_id: Option<AttemptId>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactDependency {
    pub artifact_id: LifecycleId,
    pub requires_artifact_id: LifecycleId,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Delivery {
    Pending,
    Delivered,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Retention {
    Required,
    Optional,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reclamation {
    Pending,
    Reclaimed,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Fact {
    Delivery(Delivery),
    Retention(Retention),
    Reclamation(Reclamation),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LifecycleFact {
    pub id: LifecycleId,
    pub artifact_id: LifecycleId,
    pub fact: Fact,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum JournalState {
    Intent,
    Completed,
    Blocked,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalEntry {
    pub id: LifecycleId,
    pub artifact_id: LifecycleId,
    pub operation_id: LifecycleId,
    pub step: u32,
    pub state: JournalState,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn workspace_types_validate_identifiers_aliases_and_relative_paths() {
        for bad in [
            "",
            "../escape",
            "/absolute",
            "C:/root",
            "a\\b",
            "a//b",
            "a/./b",
            "a/../b",
            "NUL.txt",
            "a.",
            "a ",
            "a\n",
        ] {
            assert!(RelativePath::new(bad).is_err(), "{bad:?}");
        }
        for good in ["run-1/attempt_2", "a/b.txt"] {
            assert_eq!(RelativePath::new(good).unwrap().as_str(), good);
        }
        for bad in ["", "../x", "a/b", "a b"] {
            assert!(LifecycleId::new(bad).is_err());
        }
        assert!(RootAlias::new("Upper").is_err());
        assert!(RootAlias::new("worktrees").is_ok());
        assert!(PlanHash::new("a".repeat(64)).is_ok());
        assert!(PlanHash::new("A".repeat(64)).is_err());
        // Serde invokes the same validating constructor through try_from.
        let invalid =
            serde::de::value::StringDeserializer::<serde::de::value::Error>::new("../bad".into());
        assert!(RelativePath::deserialize(invalid).is_err());
        for bad in ["../escape", "External", "/root", ""] {
            assert!(
                serde_json::from_value::<RootAlias>(serde_json::Value::String(bad.into())).is_err()
            );
        }
        let repository = RepositoryRecord {
            id: LifecycleId::new("repository-1").unwrap(),
            common_dir_identity: NativeIdentity::new("a".repeat(64)).unwrap(),
        };
        let generation = GenerationRecord {
            id: LifecycleId::new("generation-1").unwrap(),
            repository_id: repository.id.clone(),
            graph_id: LifecycleId::new("graph-1").unwrap(),
            ordinal: 1,
            accepted_plan_hash: PlanHash::new("b".repeat(64)).unwrap(),
            root_alias: RootAlias::new("worktrees").unwrap(),
            relative_path: RelativePath::new("run/generation-1").unwrap(),
            layout_version: 1,
        };
        let artifact = ArtifactRecord {
            id: LifecycleId::new("artifact-1").unwrap(),
            generation_id: generation.id.clone(),
            kind: ArtifactKind::Plan,
            root_alias: generation.root_alias.clone(),
            relative_path: RelativePath::new("run/generation-1/plan").unwrap(),
            attempt_id: None,
        };
        let encoded = serde_json::to_string(&generation).unwrap();
        assert_eq!(generation, serde_json::from_str(&encoded).unwrap());
        let encoded_artifact = serde_json::to_string(&artifact).unwrap();
        assert_eq!(artifact, serde_json::from_str(&encoded_artifact).unwrap());
        for public_json in [
            serde_json::to_string(&repository).unwrap(),
            encoded,
            encoded_artifact,
        ] {
            // Public records carry only aliases and contained relative locations.
            assert!(!public_json.contains("C:"));
            assert!(!public_json.contains("\\\\"));
            assert!(!public_json.contains("\":\"/"));
        }
        let mut invalid_record = serde_json::to_value(&generation).unwrap();
        invalid_record["relative_path"] = serde_json::Value::String("C:/external/root".into());
        assert!(serde_json::from_value::<GenerationRecord>(invalid_record).is_err());
    }
}
