//! `sim_core` — the headless operational-research engine.
//!
//! Pure Rust, no Bevy, no I/O beyond scenario loading. Deterministic given
//! `(scenario, seed)`: all randomness comes from a seeded [`SimRng`] threaded in by the
//! caller. Subsystems (terrain, sensing, fires, …) live here as separate modules with
//! clean interfaces. See `docs/DESIGN.md` for each model's specification and the
//! analytical result its tests validate against.

// A pure OR engine has no need for `unsafe`; forbidding it documents that and is free.
#![forbid(unsafe_code)]
// Force a doc comment on every public item — the single best habit while learning, and
// it makes `cargo doc --open` a real reference.
#![warn(missing_docs)]

pub mod air;
pub mod air_defence;
pub mod ew;
pub mod fires;
pub mod game;
pub mod los;
pub mod movement;
pub mod pomdp;
pub mod scenario;
pub mod sensing;
pub mod sim;
pub mod suppression;
pub mod terrain;

/// The simulation's canonical random number generator.
///
/// `ChaCha8Rng` has a **portable, versioned-stable** output stream: the same seed yields
/// the same draws across platforms and `rand` releases, so an archived `(scenario, seed)`
/// pair reproduces bit-for-bit later. (`StdRng` deliberately does not promise this.)
/// Seed it with [`rand::SeedableRng::seed_from_u64`].
pub type SimRng = rand_chacha::ChaCha8Rng;
