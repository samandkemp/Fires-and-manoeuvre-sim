//! The decision layer: allocation, sensor tasking, belief, and the identities that keep
//! them optional (V54-V58, V61, V70).
//!
//! One test binary over several suites. Cargo builds every `tests/*.rs` as its own
//! binary linking `sim_core` afresh; files under `tests/decision_layer/` are modules of this
//! one instead, which is why the suite relinks once here rather than 5 times.
//! Each module below is the file it always was, moved rather than rewritten.
//!
//! The `#[path]` attributes are not decoration: a test binary is a *crate root*, so a
//! bare `mod allocation;` would look for `tests/allocation.rs` -- back at the top level, where it
//! would become its own binary again and undo the grouping.

#[path = "decision_layer/allocation.rs"]
mod allocation;
#[path = "decision_layer/carried_coverage.rs"]
mod carried_coverage;
#[path = "decision_layer/identity.rs"]
mod identity;
#[path = "decision_layer/overkill.rs"]
mod overkill;
#[path = "decision_layer/tasking.rs"]
mod tasking;
