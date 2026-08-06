//! `sim_core` — the headless OR engine.
//!
//! Pure Rust, no Bevy, no I/O beyond scenario loading. Deterministic given
//! `(scenario, seed)`: every random draw comes from a seeded [`SimRng`] the caller
//! threads in. Each subsystem is its own module. `docs/DESIGN.md` has the spec for every
//! model and the analytical result its gate checks against.

// A pure OR engine has no need for `unsafe`; forbidding it documents that and is free.
#![forbid(unsafe_code)]
// Force a doc comment on every public item — the single best habit while learning, and
// it makes `cargo doc --open` a real reference.
#![warn(missing_docs)]

pub mod air;
pub mod air_defence;
pub mod allocation;
pub mod c2;
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
