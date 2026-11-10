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
    pub graph_id: GraphId,
    pub ordinal: u32,
    pub accepted_plan_hash: PlanHash,
    pub root_alias: RootAlias,
    pub relative_path: RelativePath,
    pub layout_version: u32,
    /// Explicit producer marker. `None` on every record written before issue #142
    /// and by scoped or adoption producers; absent fields are not serialized, so
    /// those payloads are unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub producer: Option<Producer>,
    /// Base branch (`refs/heads/<name>`, never under `refs/heads/meshloop/`) the run
    /// was started from. Landing is judged against this ref only. `None` means the
    /// run is never judged landed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_ref: Option<GitRef>,
}
/// Who wrote a generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Producer {
    /// The run loop, for a run it created (issue #142).
    RunLoop,
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
    /// The worktree was removed by cleanup or restart (issue #142).
    Reclaimed,
    /// Registration succeeded but the worktree was never added.
    NotCreated,
    /// Recovery found neither the directory nor a Git registration.
    Absent,
}
impl Reclamation {
    /// The artifact no longer describes an existing worktree it owns.
    pub fn retires(self) -> bool {
        !matches!(self, Self::Pending)
    }
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
            graph_id: GraphId::new("graph-1").unwrap(),
            ordinal: 1,
            accepted_plan_hash: PlanHash::new("b".repeat(64)).unwrap(),
            root_alias: RootAlias::new("worktrees").unwrap(),
            relative_path: RelativePath::new("run/generation-1").unwrap(),
            layout_version: 1,
            producer: None,
            base_ref: None,
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

// Graph identity deliberately preserves the complete pre-S3 logical grammar.
text_type!(GraphId, crate::task_graph::graph_id_is_legal);
text_type!(ObjectId, |s: &str| (s.len() == 40 || s.len() == 64)
    && s.bytes()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
fn git_ref_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && !value.starts_with('/')
        && !value.ends_with('/')
        && !value.contains("//")
        && !value.contains("..")
        && !value.contains("@{")
        && value.split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && !part.starts_with('-')
                && !part.ends_with('.')
                && !part.ends_with(".lock")
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
        })
}
text_type!(GitRef, git_ref_name);
/// How a receipt claims accepted content reached the destination ref.
/// Independent of task status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LandingStrategy {
    FastForward,
    Merge,
    Rewrite,
}
impl LandingStrategy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FastForward => "fast-forward",
            Self::Merge => "merge",
            Self::Rewrite => "rewrite",
        }
    }
}
/// Stable reasons a delivery receipt failed closed. An empty list is a pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeliveryReason {
    AcceptedNotLanded,
    MissingRecoveryObject,
    PlannerDirty,
    RewrittenDestination,
    StaleDestination,
    UnreadableRef,
}
impl DeliveryReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AcceptedNotLanded => "accepted-not-landed",
            Self::MissingRecoveryObject => "missing-recovery-object",
            Self::PlannerDirty => "planner-dirty",
            Self::RewrittenDestination => "rewritten-destination",
            Self::StaleDestination => "stale-destination",
            Self::UnreadableRef => "unreadable-ref",
        }
    }
}
/// Local claim that accepted content landed. Holding one is not proof.
/// A pass does not close tasks, publish, or authorize deletion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryReceipt {
    pub strategy: LandingStrategy,
    pub accepted_commit: ObjectId,
    pub destination_ref: GitRef,
    /// Commit the destination ref was expected to name. A different live
    /// commit is stale. Rewrite proofs require this value.
    pub recorded_destination: Option<ObjectId>,
    /// Root tree of the accepted commit, for a rewrite whose commit id changed.
    pub accepted_tree: Option<ObjectId>,
    /// Object ids that must still be present. The check never deletes them.
    pub recovery_objects: Vec<ObjectId>,
}
/// Read-only delivery verdict. `apply` is always false.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryReceiptReport {
    pub passed: bool,
    pub apply: bool,
    pub reasons: Vec<DeliveryReason>,
    pub observed_destination: Option<ObjectId>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortableLocation {
    pub root_alias: RootAlias,
    pub relative_path: RelativePath,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunBinding {
    pub graph_id: GraphId,
    pub repository_id: LifecycleId,
    pub location: PortableLocation,
    pub layout_version: u32,
    pub active_generation: Option<LifecycleId>,
    pub current_revision: Option<LifecycleId>,
}
/// Layout written by the run loop (issue #142): the run location is
/// `<root>/<graph>` and holds `integrate`, `task-<n>-attempt-<m>` and
/// `integrate-<tag>` worktrees on `meshloop/<graph>/...` branches. It describes
/// placement only; ownership is the explicit `Producer::RunLoop` marker.
///
/// A run the run loop creates is bound by its first generation: that record pins
/// graph, repository, root alias, run location, layout and base branch. It has no
/// `workspace_run_bindings` row, which stays reserved for scoped runs whose events
/// and attempts carry generation and revision scope.
pub const RUN_LOOP_LAYOUT_VERSION: u32 = 2;
impl GenerationRecord {
    /// True for any generation the run loop wrote with its layout.
    pub fn is_run_loop(&self) -> bool {
        self.producer == Some(Producer::RunLoop) && self.layout_version == RUN_LOOP_LAYOUT_VERSION
    }
    /// True for the binding (first) generation of a run the run loop created.
    pub fn is_run_loop_binding(&self) -> bool {
        self.is_run_loop() && self.ordinal == 1
    }
}
/// A base branch the run loop may judge landing against: `refs/heads/<name>`,
/// never one of its own `refs/heads/meshloop/...` branches.
pub fn run_loop_base_ref(symbolic: &str) -> Option<GitRef> {
    let name = symbolic.strip_prefix("refs/heads/")?;
    if name.is_empty() || name == "meshloop" || name.starts_with("meshloop/") {
        return None;
    }
    GitRef::new(symbolic).ok()
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProvisionalKind {
    Planning,
    ControlOutput,
    ReviewPack,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProvisionalOwner {
    pub id: LifecycleId,
    pub repository_id: LifecycleId,
    pub graph_id: Option<GraphId>,
    pub kind: ProvisionalKind,
    pub location: PortableLocation,
    pub provenance: LifecycleId,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProvisionalPromotion {
    pub owner_id: LifecycleId,
    pub generation_id: LifecycleId,
    pub artifact_id: LifecycleId,
}
/// Metadata is exportable; exact protected plan JSON is held by the engine snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanRevision {
    pub id: LifecycleId,
    pub repository_id: LifecycleId,
    pub graph_id: GraphId,
    pub generation_id: Option<LifecycleId>,
    pub predecessor: Option<LifecycleId>,
    pub plan_hash: PlanHash,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthorizationKind {
    Human,
    InheritedAdr0028,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RevisionDecision {
    Accepted,
    Declined,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevisionAuthorization {
    pub id: LifecycleId,
    pub revision_id: LifecycleId,
    pub kind: AuthorizationKind,
    pub decision: RevisionDecision,
    pub actor: LifecycleId,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionScope {
    pub repository_id: LifecycleId,
    pub graph_id: GraphId,
    pub generation_id: LifecycleId,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RunScope {
    ActiveGeneration,
    Generation(LifecycleId),
    Legacy,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProducerTarget {
    RootBootstrap(RootAlias),
    Provisional(LifecycleId),
    Artifact(LifecycleId),
    Driver(LifecycleId),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProducerKind {
    BootstrapRoot,
    Plan,
    Cache,
    Spawn,
    Worktree,
    Commit,
    Merge,
    RetainRef,
    ReviewPack,
    Deliverable,
    Landing,
    Verify,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpectedState {
    pub source_artifact: Option<LifecycleId>,
    pub source: Option<ObjectId>,
    pub base: Option<ObjectId>,
    pub location: Option<PortableLocation>,
    pub native_identity: Option<NativeIdentity>,
    pub head: Option<ObjectId>,
    pub reference: Option<RelativePath>,
    pub registered: bool,
    pub exists: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProducerOperation {
    pub id: LifecycleId,
    pub repository_id: LifecycleId,
    pub graph_id: Option<GraphId>,
    pub generation_id: Option<LifecycleId>,
    pub target: ProducerTarget,
    pub kind: ProducerKind,
    pub before: ExpectedState,
    pub after: ExpectedState,
    pub max_steps: u32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperationState {
    Completed,
    Blocked,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationStep {
    pub id: LifecycleId,
    pub operation_id: LifecycleId,
    pub step: u32,
    pub observed: ExpectedState,
    pub state: OperationState,
    pub error_code: Option<LifecycleId>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessIdentity {
    pub pid: u32,
    pub creation_token: LifecycleId,
    pub tree_token: LifecycleId,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DriverTarget {
    Execution(ExecutionScope),
    Provisional {
        repository_id: LifecycleId,
        owner_id: LifecycleId,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DriverPhase {
    HandoffPending,
    Starting,
    Running,
    Exited,
    Lost,
    Blocked,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriverRecord {
    pub id: LifecycleId,
    pub nonce: LifecycleId,
    pub target: DriverTarget,
    pub process: Option<ProcessIdentity>,
    pub phase: DriverPhase,
    pub heartbeat: u64,
    pub error_code: Option<LifecycleId>,
    pub diagnostic: Option<PortableLocation>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CancellationTarget {
    Execution {
        scope: ExecutionScope,
        driver_id: LifecycleId,
        task_id: Option<crate::task_graph::TaskId>,
    },
    Provisional {
        repository_id: LifecycleId,
        owner_id: LifecycleId,
        driver_id: LifecycleId,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancellationRequest {
    pub id: LifecycleId,
    pub target: CancellationTarget,
    pub process: ProcessIdentity,
    pub created_at: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CancellationStatus {
    Pending,
    Applied,
    Blocked,
    Stale,
    OwnerLost,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancellationObservation {
    pub request: CancellationRequest,
    pub status: CancellationStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactBinding {
    pub artifact_id: LifecycleId,
    pub revision_id: LifecycleId,
    pub logical_tag: Option<GraphId>,
}

/// Read-only inventory and adoption-preview report. `apply` is always false.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InventoryReport {
    pub mode: ReportMode,
    pub apply: bool,
    pub blockers: Vec<BlockerCode>,
    pub entries: Vec<InventoryEntry>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReportMode {
    Inventory,
    Preview,
}
impl ReportMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Inventory => "inventory",
            Self::Preview => "preview",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InventoryClass {
    Active,
    Inaccessible,
    Legacy,
    Missing,
    Unknown,
}
impl InventoryClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Inaccessible => "inaccessible",
            Self::Legacy => "legacy",
            Self::Missing => "missing",
            Self::Unknown => "unknown",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OwnershipCode {
    ConfiguredRoot,
    GitRegistered,
    LedgerOwned,
    LegacyUnbound,
    Unresolved,
}
impl OwnershipCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ConfiguredRoot => "configured-root",
            Self::GitRegistered => "git-registered",
            Self::LedgerOwned => "ledger-owned",
            Self::LegacyUnbound => "legacy-unbound",
            Self::Unresolved => "unresolved",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BlockerCode {
    AdoptionPreviewOnly,
    DriverActive,
    ExternalPathWithheld,
    FenceInaccessible,
    FenceMissing,
    GitUnavailable,
    IdentityConflict,
    InventoryTruncated,
    LedgerInaccessible,
    LedgerMissing,
    LegacyOwnershipUnresolved,
    PathInaccessible,
    PathMissing,
    ReparseBlocked,
    RepositoryUnbound,
    SchemaUpgradeRequired,
    UnknownOwnership,
}
impl BlockerCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AdoptionPreviewOnly => "adoption-preview-only",
            Self::DriverActive => "driver-active",
            Self::ExternalPathWithheld => "external-path-withheld",
            Self::FenceInaccessible => "fence-inaccessible",
            Self::FenceMissing => "fence-missing",
            Self::GitUnavailable => "git-unavailable",
            Self::IdentityConflict => "identity-conflict",
            Self::InventoryTruncated => "inventory-truncated",
            Self::LedgerInaccessible => "ledger-inaccessible",
            Self::LedgerMissing => "ledger-missing",
            Self::LegacyOwnershipUnresolved => "legacy-ownership-unresolved",
            Self::PathInaccessible => "path-inaccessible",
            Self::PathMissing => "path-missing",
            Self::ReparseBlocked => "reparse-blocked",
            Self::RepositoryUnbound => "repository-unbound",
            Self::SchemaUpgradeRequired => "schema-upgrade-required",
            Self::UnknownOwnership => "unknown-ownership",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EntrySource {
    Artifact,
    ConfiguredEntry,
    Driver,
    Generation,
    GitWorktree,
    LegacyAttempt,
    LegacyRun,
    Provisional,
}
impl EntrySource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Artifact => "artifact",
            Self::ConfiguredEntry => "configured-entry",
            Self::Driver => "driver",
            Self::Generation => "generation",
            Self::GitWorktree => "git-worktree",
            Self::LegacyAttempt => "legacy-attempt",
            Self::LegacyRun => "legacy-run",
            Self::Provisional => "provisional",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LocationScope {
    ConfiguredRoot,
    LedgerRoot,
    Repository,
}
impl LocationScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ConfiguredRoot => "configured-root",
            Self::LedgerRoot => "ledger-root",
            Self::Repository => "repository",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InventoryLocation {
    pub scope: LocationScope,
    pub root_alias: Option<RootAlias>,
    pub relative_path: RelativePath,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InventoryEntry {
    pub class: InventoryClass,
    pub ownership: OwnershipCode,
    pub blockers: Vec<BlockerCode>,
    pub source: EntrySource,
    pub graph_id: Option<GraphId>,
    pub lifecycle_id: Option<LifecycleId>,
    pub location: Option<InventoryLocation>,
    pub withheld: bool,
}

#[cfg(test)]
mod graph_identity_tests {
    use super::*;
    #[test]
    fn b1_graph_identity_preserves_complete_existing_grammar() {
        for value in [
            ".".to_string(),
            "..".to_string(),
            "release.1".to_string(),
            "._-Az09".to_string(),
            "a".repeat(5000),
        ] {
            let id = GraphId::new(value.clone()).unwrap();
            assert_eq!(id.as_str(), value);
            let encoded = serde_json::to_string(&id).unwrap();
            assert_eq!(id, serde_json::from_str::<GraphId>(&encoded).unwrap());
        }
        for value in ["", "/", "a/b", "a\\b", "white space", "é"] {
            assert!(GraphId::new(value).is_err());
        }
        for path in [".", "..", "run/../escape", "/root", "C:/root"] {
            assert!(RelativePath::new(path).is_err());
        }
        assert!(RelativePath::new("run/opaque-generation/artifact").is_ok());
    }
}

#[cfg(test)]
mod inventory_reason_codes {
    use super::*;

    fn assert_code<T: serde::Serialize>(value: T, code: &str) {
        assert_eq!(
            serde_json::to_value(value).unwrap(),
            serde_json::json!(code)
        );
    }

    #[test]
    fn inventory_reason_codes_serialize_as_stable_kebab_case() {
        for (value, code) in [
            (InventoryClass::Active, "active"),
            (InventoryClass::Inaccessible, "inaccessible"),
            (InventoryClass::Legacy, "legacy"),
            (InventoryClass::Missing, "missing"),
            (InventoryClass::Unknown, "unknown"),
        ] {
            assert_eq!(value.as_str(), code);
            assert_code(value, code);
        }
        for (value, code) in [
            (OwnershipCode::ConfiguredRoot, "configured-root"),
            (OwnershipCode::GitRegistered, "git-registered"),
            (OwnershipCode::LedgerOwned, "ledger-owned"),
            (OwnershipCode::LegacyUnbound, "legacy-unbound"),
            (OwnershipCode::Unresolved, "unresolved"),
        ] {
            assert_eq!(value.as_str(), code);
            assert_code(value, code);
        }
        for (value, code) in [
            (BlockerCode::AdoptionPreviewOnly, "adoption-preview-only"),
            (BlockerCode::DriverActive, "driver-active"),
            (BlockerCode::ExternalPathWithheld, "external-path-withheld"),
            (BlockerCode::FenceInaccessible, "fence-inaccessible"),
            (BlockerCode::FenceMissing, "fence-missing"),
            (BlockerCode::GitUnavailable, "git-unavailable"),
            (BlockerCode::IdentityConflict, "identity-conflict"),
            (BlockerCode::InventoryTruncated, "inventory-truncated"),
            (BlockerCode::LedgerInaccessible, "ledger-inaccessible"),
            (BlockerCode::LedgerMissing, "ledger-missing"),
            (
                BlockerCode::LegacyOwnershipUnresolved,
                "legacy-ownership-unresolved",
            ),
            (BlockerCode::PathInaccessible, "path-inaccessible"),
            (BlockerCode::PathMissing, "path-missing"),
            (BlockerCode::ReparseBlocked, "reparse-blocked"),
            (BlockerCode::RepositoryUnbound, "repository-unbound"),
            (
                BlockerCode::SchemaUpgradeRequired,
                "schema-upgrade-required",
            ),
            (BlockerCode::UnknownOwnership, "unknown-ownership"),
        ] {
            assert_eq!(value.as_str(), code);
            assert_code(value, code);
        }
        let report = InventoryReport {
            mode: ReportMode::Inventory,
            apply: false,
            blockers: vec![BlockerCode::LedgerMissing, BlockerCode::FenceMissing],
            entries: Vec::new(),
        };
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["apply"], false);
        assert_eq!(json["mode"], "inventory");
        assert!(!json.to_string().contains("C:"));
        assert!(!json.to_string().contains("\":\"/"));
    }
}

#[cfg(test)]
mod delivery_receipt_codes {
    use super::*;

    #[test]
    fn delivery_reason_codes_serialize_as_stable_kebab_case() {
        for (value, code) in [
            (DeliveryReason::AcceptedNotLanded, "accepted-not-landed"),
            (
                DeliveryReason::MissingRecoveryObject,
                "missing-recovery-object",
            ),
            (DeliveryReason::PlannerDirty, "planner-dirty"),
            (
                DeliveryReason::RewrittenDestination,
                "rewritten-destination",
            ),
            (DeliveryReason::StaleDestination, "stale-destination"),
            (DeliveryReason::UnreadableRef, "unreadable-ref"),
        ] {
            assert_eq!(value.as_str(), code);
            assert_eq!(
                serde_json::to_value(value).unwrap(),
                serde_json::json!(code)
            );
        }
        for (value, code) in [
            (LandingStrategy::FastForward, "fast-forward"),
            (LandingStrategy::Merge, "merge"),
            (LandingStrategy::Rewrite, "rewrite"),
        ] {
            assert_eq!(value.as_str(), code);
            assert_eq!(
                serde_json::to_value(value).unwrap(),
                serde_json::json!(code)
            );
        }
        assert!(GitRef::new("refs/heads/main").is_ok());
        assert!(GitRef::new("HEAD").is_ok());
        for refused in ["", "-rf", "refs/../main", "refs/heads/main.lock", "a b"] {
            assert!(GitRef::new(refused).is_err(), "{refused}");
        }
    }
}
