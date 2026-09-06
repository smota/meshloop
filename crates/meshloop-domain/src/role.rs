//! Prefixed Meshloop role and command ids (ADR 0017). Unprefixed names are rejected
//! so they cannot collide with Pi/Claude/Grok harness vocabulary.

use serde::{Deserialize, Serialize};

pub const PREFIX: &str = "meshloop:";

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MeshloopId(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RoleKind {
    Supervisor,
    Coordinator,
    Leaf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpawnPolicy {
    Never,
    LeavesOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleDefinition {
    pub id: MeshloopId,
    pub kind: RoleKind,
    pub spawn: SpawnPolicy,
    pub responsibility: &'static str,
    pub source: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdError {
    Empty,
    Unprefixed(String),
    Unknown(String),
}

impl MeshloopId {
    pub fn parse(raw: &str) -> Result<Self, IdError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(IdError::Empty);
        }
        if let Some(rest) = trimmed.strip_prefix(PREFIX) {
            if rest.is_empty()
                || !rest
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            {
                return Err(IdError::Unknown(trimmed.into()));
            }
            return Ok(Self(format!("{PREFIX}{rest}")));
        }
        // Accept MCP underscore form meshloop_plan and slash meshloop-plan as the same id.
        if let Some(rest) = trimmed.strip_prefix("meshloop_") {
            return Self::parse(&format!("{PREFIX}{rest}"));
        }
        if let Some(rest) = trimmed.strip_prefix("meshloop-") {
            return Self::parse(&format!("{PREFIX}{rest}"));
        }
        Err(IdError::Unprefixed(trimmed.into()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn slash(&self) -> String {
        format!("/{}", self.0)
    }

    pub fn mcp_tool(&self) -> String {
        self.0.replace(':', "_")
    }

    pub fn cli_verb(&self) -> &str {
        self.0.strip_prefix(PREFIX).unwrap_or(self.0.as_str())
    }
}

pub fn bundled_roles() -> Vec<RoleDefinition> {
    vec![
        RoleDefinition {
            id: MeshloopId("meshloop:origin".into()),
            kind: RoleKind::Supervisor,
            spawn: SpawnPolicy::Never,
            responsibility: "Originating agent session: intent, accept, inspect. Never a worker.",
            source: "bundled",
        },
        RoleDefinition {
            id: MeshloopId("meshloop:planner".into()),
            kind: RoleKind::Coordinator,
            spawn: SpawnPolicy::LeavesOnly,
            responsibility: "Turn origin intent into an executable multi-model graph. Does not implement.",
            source: "bundled",
        },
        RoleDefinition {
            id: MeshloopId("meshloop:scout".into()),
            kind: RoleKind::Leaf,
            spawn: SpawnPolicy::Never,
            responsibility: "Map code and constraints. Report-only.",
            source: "bundled",
        },
        RoleDefinition {
            id: MeshloopId("meshloop:worker".into()),
            kind: RoleKind::Leaf,
            spawn: SpawnPolicy::Never,
            responsibility: "Implement one task node in one worktree. No merge/push/spawn.",
            source: "bundled",
        },
        RoleDefinition {
            id: MeshloopId("meshloop:reviewer".into()),
            kind: RoleKind::Leaf,
            spawn: SpawnPolicy::Never,
            responsibility: "Review parent-pinned evidence. No edits, no spawn.",
            source: "bundled",
        },
    ]
}

pub fn bundled_commands() -> &'static [&'static str] {
    &[
        "meshloop:plan",
        "meshloop:run",
        "meshloop:status",
        "meshloop:accept",
        "meshloop:resume",
        "meshloop:cancel",
        "meshloop:inspect",
        "meshloop:integrate",
        "meshloop:roles",
        "meshloop:doctor",
        "meshloop:orchestrate",
        "meshloop:mcp",
    ]
}

pub fn lookup_role(id: &MeshloopId) -> Option<RoleDefinition> {
    bundled_roles().into_iter().find(|r| r.id == *id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unprefixed_harness_vocabulary() {
        for raw in [
            "reviewer",
            "planner",
            "scout",
            "worker",
            "orchestrate",
            "plan",
        ] {
            assert!(
                matches!(MeshloopId::parse(raw), Err(IdError::Unprefixed(_))),
                "{raw}"
            );
        }
    }

    #[test]
    fn accepts_colon_underscore_and_hyphen_forms() {
        let a = MeshloopId::parse("meshloop:reviewer").unwrap();
        let b = MeshloopId::parse("meshloop_reviewer").unwrap();
        let c = MeshloopId::parse("meshloop-reviewer").unwrap();
        assert_eq!(a, b);
        assert_eq!(b, c);
        assert_eq!(a.slash(), "/meshloop:reviewer");
        assert_eq!(a.mcp_tool(), "meshloop_reviewer");
    }

    #[test]
    fn bundled_ids_are_all_prefixed() {
        for role in bundled_roles() {
            assert!(role.id.as_str().starts_with(PREFIX));
        }
        for cmd in bundled_commands() {
            assert!(cmd.starts_with(PREFIX));
        }
    }

    #[test]
    fn origin_never_spawns() {
        let origin = lookup_role(&MeshloopId::parse("meshloop:origin").unwrap()).unwrap();
        assert_eq!(origin.kind, RoleKind::Supervisor);
        assert_eq!(origin.spawn, SpawnPolicy::Never);
    }
}
