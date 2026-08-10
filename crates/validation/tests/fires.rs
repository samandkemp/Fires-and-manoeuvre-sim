//! Direct and indirect fires: hit probability, area damage, and what may be shot at
//! (V14-V24, V68).
//!
//! One test binary over several suites. Cargo builds every `tests/*.rs` as its own
//! binary linking `sim_core` afresh; files under `tests/fires/` are modules of this
//! one instead, which is why the suite relinks once here rather than 2 times.
//! Each module below is the file it always was, moved rather than rewritten.
//!
//! The `#[path]` attributes are not decoration: a test binary is a *crate root*, so a
//! bare `mod hit_and_area;` would look for `tests/hit_and_area.rs` -- back at the top level, where it
//! would become its own binary again and undo the grouping.

#[path = "fires/hit_and_area.rs"]
mod hit_and_area;
#[path = "fires/indirect_eligibility.rs"]
mod indirect_eligibility;
