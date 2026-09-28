//! Movement: least-risk pathing as dynamic programming, and the in-loop planner that
//! runs the same search (V25-V27, V38, V72-V74).
//!
//! One test binary over several suites: each `tests/*.rs` is its own link unit, so
//! the files under `tests/movement/` are modules of this one rather than binaries of their
//! own. `#[path]` is required because a test binary is a crate root - a bare `mod x;`
//! would look for `tests/x.rs` and become a separate binary again.

#[path = "movement/fire_risk.rs"]
mod fire_risk;
#[path = "movement/pathing.rs"]
mod pathing;
#[path = "movement/planning.rs"]
mod planning;
