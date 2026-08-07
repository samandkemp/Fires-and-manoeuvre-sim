//! The air phases of the tick (`docs/DESIGN.md` §9): detecting airborne targets,
//! air-defence engagement, and strike release.
//!
//! These are appended to the tick per §9.6 and draw no RNG at all when the air and
//! air-defence lists are empty, which is what keeps a drone-free scenario bit-identical
//! to the pre-air engine (V52).
//!
//! # Where things live
//!
//! | Module | What it holds |
//! |---|---|
//! | [`detect`] | the glimpse process against airborne targets (§9.1) |
//! | [`engage`] | envelope and cueing gates, resolving shots, the C2 link (§9.4, §9.5, §11.2) |
//! | [`coordinate`] | the per-side coordinated assignment and its payoff (§11.2) |
//! | [`strike`] | release, the aim point, and whether the target is radiating (§9.3, §12.3) |
//! | [`damage`] | the Carleton kernel against batteries, posts and units (§2.3, §12) |
//!
//! All five are grandchildren of `sim`, so they still reach [`Sim`](crate::sim::Sim)'s
//! private fields — Rust makes a private item visible to the defining module *and its
//! descendants*, which is what let this split cost nothing in encapsulation. The one
//! consequence is spelling: a method `sim/mod.rs` calls needs `pub(in crate::sim)`, because
//! `pub(super)` from one level deeper now means "visible in `counter_air`".

mod coordinate;
mod damage;
mod detect;
mod engage;
mod strike;

/// Longest window the air-defence payoff will score a target over, seconds
/// (`docs/DESIGN.md` §11.2).
const AD_PLANNING_HORIZON_S: f32 = 60.0;

/// One coordinated battery's entry in its side's assignment: which battery it is, which
/// airframes it may engage this tick, and the slant range to each.
///
/// The two vectors are indexed by airframe, parallel to `Sim::air`, so a battery's row of
/// the payoff matrix is a pair of lookups rather than a search.
type CoordinatedBattery = (usize, Vec<bool>, Vec<f32>);
