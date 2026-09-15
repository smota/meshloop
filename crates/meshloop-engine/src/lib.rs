//! Use cases, scheduling, ports, recovery and integration coordination. Depends on
//! meshloop-domain and meshloop-context; never constructs concrete adapters — see
//! docs/architecture/boundaries.md.

pub mod agent;
pub mod converge;
pub mod orchestrate;
pub mod orchestrator;
pub mod origin;
pub mod planner;
pub mod ports;
pub mod recovery;
pub mod router;
pub mod run_loop;
pub mod slice;
pub mod verify;
