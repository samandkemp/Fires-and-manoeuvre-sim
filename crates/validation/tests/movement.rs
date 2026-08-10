//! Movement: least-risk pathing as dynamic programming, and the in-loop planner that
//! runs the same search (V25-V27, V38, V72-V74).
//!
//! One test binary over several suites. Cargo builds every `tests/*.rs` as its own
//! binary linking `sim_core` afresh; files under `tests/movement/` are modules of this
//! one instead, which is why the suite relinks once here rather than 2 times.
//! Each module below is the file it always was, moved rather than rewritten.
//!
//! The `#[path]` attributes are not decoration: a test binary is a *crate root*, so a
//! bare `mod pathing;` would look for `tests/pathing.rs` -- back at the top level, where it
//! would become its own binary again and undo the grouping.

#[path = "movement/pathing.rs"]
mod pathing;
#[path = "movement/planning.rs"]
mod planning;
