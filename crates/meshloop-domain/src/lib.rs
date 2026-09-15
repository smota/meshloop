//! Pure domain types: task graphs, execution state, evidence, policy, harness
//! capability/error-taxonomy value types, and the diagnostic lattice. No I/O, no
//! concrete harnesses, no storage — see docs/architecture/boundaries.md.

#![forbid(unsafe_code)]

pub mod capability;
pub mod diagnostic;
pub mod digest;
pub mod evidence;
pub mod policy;
pub mod role;
pub mod state;
pub mod task_graph;
