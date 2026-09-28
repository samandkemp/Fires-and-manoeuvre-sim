//! The decision layer: allocation, sensor tasking, belief, and the identities that keep
//! them optional (V54-V58, V61, V70).
//!
//! One test binary over several suites: each `tests/*.rs` is its own link unit, so
//! the files under `tests/decision_layer/` are modules of this one rather than binaries of their
//! own. `#[path]` is required because a test binary is a crate root - a bare `mod x;`
//! would look for `tests/x.rs` and become a separate binary again.

#[path = "decision_layer/allocation.rs"]
mod allocation;
#[path = "decision_layer/carried_coverage.rs"]
mod carried_coverage;
#[path = "decision_layer/horizon.rs"]
mod horizon;
#[path = "decision_layer/identity.rs"]
mod identity;
#[path = "decision_layer/overkill.rs"]
mod overkill;
#[path = "decision_layer/tasking.rs"]
mod tasking;
