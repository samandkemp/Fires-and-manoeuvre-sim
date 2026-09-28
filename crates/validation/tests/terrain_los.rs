//! Terrain, line of sight, and the layers derived from them (V1-V13).
//!
//! One test binary over several suites: each `tests/*.rs` is its own link unit, so
//! the files under `tests/terrain_los/` are modules of this one rather than binaries of their
//! own. `#[path]` is required because a test binary is a crate root - a bare `mod x;`
//! would look for `tests/x.rs` and become a separate binary again.

#[path = "terrain_los/los.rs"]
mod los;
#[path = "terrain_los/terrain.rs"]
mod terrain;
