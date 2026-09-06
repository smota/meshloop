//! Pure domain types: task graphs, execution state, evidence, policy, and harness
//! capability/error-taxonomy value types. No I/O, no concrete harnesses, no storage —
//! see docs/architecture/boundaries.md.

pub mod capability;
pub mod evidence;
pub mod policy;
pub mod state;
pub mod task_graph;
