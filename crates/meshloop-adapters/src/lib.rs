//! Harness, process, Git, and storage implementations. Never product policy
//! decisions — see docs/architecture/boundaries.md.

pub mod advisory;
pub mod check;
pub mod delivery;
pub mod git;
pub mod harness;
pub mod host_git;
pub mod jev;
pub mod process;
pub mod redact;
pub mod store;
pub mod workspace;

pub mod telemetry;
