//! Terrain, line of sight, and the layers derived from them (V1-V13).
//!
//! One test binary over several suites. Cargo builds every `tests/*.rs` as its own
//! binary linking `sim_core` afresh; files under `tests/terrain_los/` are modules of this
//! one instead, which is why the suite relinks once here rather than 2 times.
//! Each module below is the file it always was, moved rather than rewritten.
//!
//! The `#[path]` attributes are not decoration: a test binary is a *crate root*, so a
//! bare `mod terrain;` would look for `tests/terrain.rs` -- back at the top level, where it
//! would become its own binary again and undo the grouping.

#[path = "terrain_los/los.rs"]
mod los;
#[path = "terrain_los/terrain.rs"]
mod terrain;
