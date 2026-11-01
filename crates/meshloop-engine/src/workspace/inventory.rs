//! Read-only inventory classification and legacy-adoption preview.
//! No mutation plan is produced and `apply` stays false.

use meshloop_domain::workspace::*;

const MAX_ENTRIES: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Presence {
    NotApplicable,
    Present,
    Missing,
    Inaccessible,
    Reparse,
}

#[derive(Debug, Clone)]
pub struct Observation {
    pub source: EntrySource,
    pub graph_id: Option<GraphId>,
    pub lifecycle_id: Option<LifecycleId>,
    pub location: Option<InventoryLocation>,
    pub withheld: bool,
    pub presence: Presence,
    pub legacy_marker: bool,
    pub ledger_record: bool,
    pub identity_confirmed: bool,
    pub git_registered: bool,
    pub driver_active: bool,
    pub configured_entry: bool,
}

#[derive(Debug, Clone)]
pub struct InventoryFacts {
    pub command_blockers: Vec<BlockerCode>,
    pub observations: Vec<Observation>,
}

pub fn classify(facts: &InventoryFacts) -> InventoryReport {
    let mut blockers = dedup(facts.command_blockers.clone());
    let confirmed = !blockers.contains(&BlockerCode::IdentityConflict);
    let mut merged = merge(&facts.observations);
    if merged.len() > MAX_ENTRIES {
        merged.truncate(MAX_ENTRIES);
        push(&mut blockers, BlockerCode::InventoryTruncated);
    }
    let entries = merged
        .iter()
        .map(|observation| classify_one(observation, confirmed))
        .collect();
    finish(&mut blockers);
    InventoryReport {
        mode: ReportMode::Inventory,
        apply: false,
        blockers,
        entries,
    }
}

pub fn adoption_preview(report: &InventoryReport) -> InventoryReport {
    let mut blockers = report.blockers.clone();
    push(&mut blockers, BlockerCode::AdoptionPreviewOnly);
    finish(&mut blockers);
    let entries = report
        .entries
        .iter()
        .filter(|entry| adoption_candidate(entry))
        .cloned()
        .map(|mut entry| {
            push(&mut entry.blockers, BlockerCode::AdoptionPreviewOnly);
            finish(&mut entry.blockers);
            entry
        })
        .collect();
    InventoryReport {
        mode: ReportMode::Preview,
        apply: false,
        blockers,
        entries,
    }
}

fn adoption_candidate(entry: &InventoryEntry) -> bool {
    matches!(
        entry.class,
        InventoryClass::Legacy | InventoryClass::Unknown | InventoryClass::Inaccessible
    ) || entry.blockers.iter().any(|blocker| {
        matches!(
            blocker,
            BlockerCode::UnknownOwnership
                | BlockerCode::LegacyOwnershipUnresolved
                | BlockerCode::ReparseBlocked
                | BlockerCode::PathInaccessible
                | BlockerCode::ExternalPathWithheld
                | BlockerCode::DriverActive
                | BlockerCode::IdentityConflict
        )
    })
}

fn classify_one(observation: &Observation, command_confirmed: bool) -> InventoryEntry {
    let identity = observation.identity_confirmed && command_confirmed && observation.ledger_record;
    let mut blockers = Vec::new();
    let mut class = match observation.presence {
        Presence::Reparse => {
            push(&mut blockers, BlockerCode::ReparseBlocked);
            InventoryClass::Inaccessible
        }
        Presence::Inaccessible => {
            push(&mut blockers, BlockerCode::PathInaccessible);
            InventoryClass::Inaccessible
        }
        Presence::Missing => {
            push(&mut blockers, BlockerCode::PathMissing);
            InventoryClass::Missing
        }
        Presence::Present | Presence::NotApplicable => InventoryClass::Unknown,
    };
    let ownership = if observation.legacy_marker {
        push(&mut blockers, BlockerCode::LegacyOwnershipUnresolved);
        if !matches!(class, InventoryClass::Inaccessible) {
            class = InventoryClass::Legacy;
        }
        OwnershipCode::LegacyUnbound
    } else if identity {
        match observation.presence {
            Presence::Present => class = InventoryClass::Active,
            Presence::NotApplicable if observation.driver_active => class = InventoryClass::Active,
            Presence::NotApplicable => {
                class = InventoryClass::Inaccessible;
                push(&mut blockers, BlockerCode::PathInaccessible);
            }
            Presence::Missing => class = InventoryClass::Missing,
            Presence::Inaccessible | Presence::Reparse => {}
        }
        OwnershipCode::LedgerOwned
    } else if observation.git_registered && !observation.ledger_record {
        push(&mut blockers, BlockerCode::UnknownOwnership);
        OwnershipCode::GitRegistered
    } else if observation.configured_entry && !observation.ledger_record {
        push(&mut blockers, BlockerCode::UnknownOwnership);
        OwnershipCode::ConfiguredRoot
    } else {
        push(&mut blockers, BlockerCode::UnknownOwnership);
        OwnershipCode::Unresolved
    };
    if observation.driver_active {
        push(&mut blockers, BlockerCode::DriverActive);
    }
    if observation.withheld {
        push(&mut blockers, BlockerCode::ExternalPathWithheld);
    }
    finish(&mut blockers);
    InventoryEntry {
        class,
        ownership,
        blockers,
        source: observation.source,
        graph_id: observation.graph_id.clone(),
        lifecycle_id: observation.lifecycle_id.clone(),
        location: observation.location.clone(),
        withheld: observation.withheld,
    }
}

fn merge(observations: &[Observation]) -> Vec<Observation> {
    let mut grouped: Vec<Observation> = Vec::new();
    let mut loose: Vec<Observation> = Vec::new();
    for observation in observations {
        let Some(key) = location_key(observation) else {
            loose.push(observation.clone());
            continue;
        };
        if let Some(existing) = grouped
            .iter_mut()
            .find(|item| location_key(item) == Some(key.clone()))
        {
            combine(existing, observation);
        } else {
            grouped.push(observation.clone());
        }
    }
    grouped.append(&mut loose);
    grouped.sort_by_key(entry_sort_key);
    grouped
}

fn combine(existing: &mut Observation, extra: &Observation) {
    if source_rank(extra.source) < source_rank(existing.source) {
        existing.source = extra.source;
    }
    if existing.graph_id.is_none() {
        existing.graph_id.clone_from(&extra.graph_id);
    }
    if existing.lifecycle_id.is_none() {
        existing.lifecycle_id.clone_from(&extra.lifecycle_id);
    }
    if existing.location.is_none() {
        existing.location.clone_from(&extra.location);
    }
    existing.withheld = existing.location.is_none() && (existing.withheld || extra.withheld);
    existing.presence = existing.presence.max(extra.presence);
    existing.legacy_marker |= extra.legacy_marker;
    existing.ledger_record |= extra.ledger_record;
    existing.identity_confirmed |= extra.ledger_record && extra.identity_confirmed;
    existing.git_registered |= extra.git_registered;
    existing.driver_active |= extra.driver_active;
    existing.configured_entry |= extra.configured_entry;
}

fn location_key(observation: &Observation) -> Option<String> {
    let location = observation.location.as_ref()?;
    Some(format!(
        "{}:{}:{}",
        location.scope.as_str(),
        location
            .root_alias
            .as_ref()
            .map(RootAlias::as_str)
            .unwrap_or(""),
        location.relative_path.as_str()
    ))
}

fn entry_sort_key(observation: &Observation) -> (String, String, String, String, String) {
    (
        observation.source.as_str().to_string(),
        observation
            .graph_id
            .as_ref()
            .map(GraphId::as_str)
            .unwrap_or("")
            .to_string(),
        observation
            .lifecycle_id
            .as_ref()
            .map(LifecycleId::as_str)
            .unwrap_or("")
            .to_string(),
        observation
            .location
            .as_ref()
            .map(|location| location.relative_path.as_str().to_string())
            .unwrap_or_default(),
        format!("{:?}", observation.presence),
    )
}

fn source_rank(source: EntrySource) -> u8 {
    match source {
        EntrySource::Artifact => 0,
        EntrySource::Generation => 1,
        EntrySource::Provisional => 2,
        EntrySource::Driver => 3,
        EntrySource::LegacyAttempt => 4,
        EntrySource::LegacyRun => 5,
        EntrySource::GitWorktree => 6,
        EntrySource::ConfiguredEntry => 7,
    }
}

fn push(codes: &mut Vec<BlockerCode>, code: BlockerCode) {
    if !codes.contains(&code) {
        codes.push(code);
    }
}

fn dedup(mut codes: Vec<BlockerCode>) -> Vec<BlockerCode> {
    finish(&mut codes);
    codes
}

fn finish(codes: &mut Vec<BlockerCode>) {
    codes.sort_by_key(|code| code.as_str());
    codes.dedup();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation(source: EntrySource) -> Observation {
        Observation {
            source,
            graph_id: None,
            lifecycle_id: None,
            location: None,
            withheld: false,
            presence: Presence::NotApplicable,
            legacy_marker: false,
            ledger_record: false,
            identity_confirmed: false,
            git_registered: false,
            driver_active: false,
            configured_entry: false,
        }
    }

    #[test]
    fn inventory_classification_uses_stable_reason_codes_and_never_applies() {
        let mut legacy = observation(EntrySource::LegacyRun);
        legacy.graph_id = Some(GraphId::new("legacy-run").unwrap());
        legacy.legacy_marker = true;
        legacy.ledger_record = true;

        let mut owned = observation(EntrySource::Generation);
        owned.ledger_record = true;
        owned.identity_confirmed = true;
        owned.presence = Presence::Present;
        owned.lifecycle_id = Some(LifecycleId::new("generation-1").unwrap());
        owned.location = Some(InventoryLocation {
            scope: LocationScope::LedgerRoot,
            root_alias: Some(RootAlias::new("worktrees").unwrap()),
            relative_path: RelativePath::new("run/generation-1").unwrap(),
        });

        let mut missing = owned.clone();
        missing.presence = Presence::Missing;
        missing.lifecycle_id = Some(LifecycleId::new("generation-2").unwrap());
        missing.location.as_mut().unwrap().relative_path =
            RelativePath::new("run/missing").unwrap();

        let mut git = observation(EntrySource::GitWorktree);
        git.git_registered = true;
        git.presence = Presence::Present;
        git.location = Some(InventoryLocation {
            scope: LocationScope::Repository,
            root_alias: None,
            relative_path: RelativePath::new("linked-wt").unwrap(),
        });

        let mut blocked = observation(EntrySource::ConfiguredEntry);
        blocked.configured_entry = true;
        blocked.presence = Presence::Reparse;

        let mut active_driver = observation(EntrySource::Driver);
        active_driver.ledger_record = true;
        active_driver.identity_confirmed = true;
        active_driver.driver_active = true;
        active_driver.lifecycle_id = Some(LifecycleId::new("driver-1").unwrap());

        let report = classify(&InventoryFacts {
            command_blockers: vec![BlockerCode::FenceMissing, BlockerCode::LedgerMissing],
            observations: vec![legacy, owned, missing, git, blocked, active_driver],
        });
        assert!(!report.apply);
        assert_eq!(report.mode, ReportMode::Inventory);
        assert_eq!(
            report.blockers,
            vec![BlockerCode::FenceMissing, BlockerCode::LedgerMissing]
        );
        let classes: Vec<_> = report
            .entries
            .iter()
            .map(|entry| (entry.class, entry.ownership, entry.blockers.clone()))
            .collect();
        assert!(classes.contains(&(
            InventoryClass::Legacy,
            OwnershipCode::LegacyUnbound,
            vec![BlockerCode::LegacyOwnershipUnresolved]
        )));
        assert!(classes.contains(&(
            InventoryClass::Active,
            OwnershipCode::LedgerOwned,
            Vec::new()
        )));
        assert!(classes.contains(&(
            InventoryClass::Missing,
            OwnershipCode::LedgerOwned,
            vec![BlockerCode::PathMissing]
        )));
        assert!(classes.contains(&(
            InventoryClass::Unknown,
            OwnershipCode::GitRegistered,
            vec![BlockerCode::UnknownOwnership]
        )));
        assert!(classes.contains(&(
            InventoryClass::Inaccessible,
            OwnershipCode::ConfiguredRoot,
            vec![BlockerCode::ReparseBlocked, BlockerCode::UnknownOwnership]
        )));
        assert!(classes.contains(&(
            InventoryClass::Active,
            OwnershipCode::LedgerOwned,
            vec![BlockerCode::DriverActive]
        )));

        let preview = adoption_preview(&report);
        assert!(!preview.apply);
        assert_eq!(preview.mode, ReportMode::Preview);
        assert!(preview.blockers.contains(&BlockerCode::AdoptionPreviewOnly));
        assert!(
            preview
                .entries
                .iter()
                .all(|entry| entry.blockers.contains(&BlockerCode::AdoptionPreviewOnly))
        );
        assert!(
            preview
                .entries
                .iter()
                .all(|entry| entry.class != InventoryClass::Active
                    || entry.blockers.contains(&BlockerCode::DriverActive))
        );
        let encoded = serde_json::to_string(&preview).unwrap();
        assert!(!encoded.contains("apply\":true"));
        assert!(!encoded.contains("C:"));
    }
}
