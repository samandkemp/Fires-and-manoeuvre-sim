//! Direct and indirect fires: hit probability, area damage, and what may be shot at
//! (V14-V24, V68).
//!
//! One test binary over several suites: each `tests/*.rs` is its own link unit, so
//! the files under `tests/fires/` are modules of this one rather than binaries of their
//! own. `#[path]` is required because a test binary is a crate root - a bare `mod x;`
//! would look for `tests/x.rs` and become a separate binary again.

#[path = "fires/hit_and_area.rs"]
mod hit_and_area;
#[path = "fires/indirect_eligibility.rs"]
mod indirect_eligibility;
