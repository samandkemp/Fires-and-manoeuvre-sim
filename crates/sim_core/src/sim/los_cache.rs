//! A line-of-sight memo for the per-tick sensing loop.
//!
//! # Why this exists
//!
//! The glimpse loop tests every (sensor, untracked target) pair *every tick*, and each
//! test walks the terrain grid - ~77 µs on a 10 km map. Profiling the shipped scenarios
//! showed the tick cost tracking the undetected-unit count almost exactly: a unit that
//! stays hidden behind a ridge is re-walked by every sensor, every tick, for the whole
//! run, always to be told the same thing.
//!
//! Most of those endpoints have not moved. Emplaced guns, mast sensors and dug-in
//! infantry sit still for the entire battle, and terrain never changes mid-run, so the
//! answer cannot change either.
//!
//! # Why it is exact, not approximate
//!
//! An entry is reused only when all four endpoint quantities - both positions and both
//! actor heights - compare **exactly equal** to the ones cached. No tolerance, no
//! rounding, no staleness window. Anything that moves by so much as one float ulp misses
//! the cache and pays the full traversal, so a cache hit is the same computation the
//! cache miss would have performed. That is why the event logs stay bit-identical (V18,
//! V24, V52) rather than merely close.
//!
//! Terrain is not part of the key because a [`Sim`](super::Sim) owns its terrain for
//! life: `reset_to_scenario` deliberately keeps the map. The cache is still cleared on
//! reset, since the asset lists it is indexed by are rebuilt.

use crate::los::LosResult;
use glam::Vec2;

/// The two endpoints a query was asked about: positions and actor heights.
///
/// Compared with `==` on every field, so any movement at all - down to one float ulp -
/// misses the cache. That exactness is what keeps a hit indistinguishable from a miss.
#[derive(Clone, Copy, PartialEq)]
pub(super) struct Endpoints {
    pub a: Vec2,
    pub h_a: f32,
    pub b: Vec2,
    pub h_b: f32,
}

/// Which (sensor, target) pair a slot belongs to.
#[derive(Clone, Copy)]
pub(super) struct Key {
    pub sensor: usize,
    pub kind: TargetKind,
    pub target: usize,
}

/// One remembered query: the endpoints it was computed for, and what it returned.
#[derive(Clone, Copy)]
struct Entry {
    at: Endpoints,
    los: LosResult,
}

/// Which asset list a target index refers to. The two lists are numbered independently,
/// so a slot needs both to be unambiguous.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum TargetKind {
    /// Index into `Sim::units`.
    Ground,
    /// Index into `Sim::air`.
    Air,
}

/// A `sensors × (units + air)` table of remembered LOS results.
///
/// Flat `Vec` rather than a map: the key is a pair of small integers, so indexing
/// arithmetic beats hashing, and it keeps iteration order out of the picture entirely
/// (the determinism contract bars hash order from reaching sim state).
#[derive(Default)]
pub(super) struct LosCache {
    slots: Vec<Option<Entry>>,
    sensors: usize,
    units: usize,
    air: usize,
    hits: u64,
    misses: u64,
}

impl LosCache {
    /// Resize and clear if the asset lists have changed shape - placing or removing an
    /// asset renumbers the table, so the old contents are meaningless rather than merely
    /// stale.
    pub(super) fn fit(&mut self, sensors: usize, units: usize, air: usize) {
        if self.sensors == sensors && self.units == units && self.air == air {
            return;
        }
        self.sensors = sensors;
        self.units = units;
        self.air = air;
        self.slots.clear();
        self.slots.resize(sensors * (units + air), None);
    }

    /// Forget everything (used on `Sim::reset`).
    pub(super) fn clear(&mut self) {
        self.slots.clear();
        self.sensors = 0;
        self.units = 0;
        self.air = 0;
        self.hits = 0;
        self.misses = 0;
    }

    /// Row-major slot for one (sensor, target) pair, or `None` if either index is beyond
    /// the table - which happens between an asset being added and the next `fit`.
    fn slot(&self, key: Key) -> Option<usize> {
        let stride = self.units + self.air;
        let offset = match key.kind {
            TargetKind::Ground if key.target < self.units => key.target,
            TargetKind::Air if key.target < self.air => self.units + key.target,
            _ => return None,
        };
        (key.sensor < self.sensors).then(|| key.sensor * stride + offset)
    }

    /// The cached result for this pair if the endpoints are unchanged, else `None`.
    pub(super) fn get(&mut self, key: Key, at: Endpoints) -> Option<LosResult> {
        let hit = self
            .slot(key)
            .and_then(|i| self.slots[i])
            .filter(|e| e.at == at)
            .map(|e| e.los);
        if hit.is_some() {
            self.hits += 1;
        } else {
            self.misses += 1;
        }
        hit
    }

    /// Remember a freshly computed result against the endpoints that produced it.
    pub(super) fn put(&mut self, key: Key, at: Endpoints, los: LosResult) {
        if let Some(i) = self.slot(key) {
            self.slots[i] = Some(Entry { at, los });
        }
    }

    /// Hits and misses since the last reset - for the bench harness and for checking the
    /// cache is actually earning its keep on a given scenario.
    pub(super) fn stats(&self) -> (u64, u64) {
        (self.hits, self.misses)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::los::LosResult;

    fn dummy(t: f32) -> LosResult {
        LosResult {
            clear: true,
            transmittance: t,
            mask_height: -1.0,
            blocked_at: None,
            canopy_length: 0.0,
        }
    }

    fn at(bx: f32) -> Endpoints {
        Endpoints {
            a: Vec2::new(1.0, 2.0),
            h_a: 2.0,
            b: Vec2::new(bx, 40.0),
            h_b: 1.8,
        }
    }

    const KEY: Key = Key {
        sensor: 1,
        kind: TargetKind::Ground,
        target: 2,
    };

    // The invalidation contract the whole optimisation rests on: identical endpoints
    // reuse the answer, and *any* movement re-computes. If this ever softened into an
    // approximate comparison, a slow-moving unit would keep a stale sightline and the
    // event logs would silently diverge from the un-cached model.
    #[test]
    fn a_hit_needs_exactly_equal_endpoints() {
        let mut c = LosCache::default();
        c.fit(3, 4, 0);
        c.put(KEY, at(30.0), dummy(0.5));

        assert_eq!(c.get(KEY, at(30.0)).map(|l| l.transmittance), Some(0.5));

        // One ulp of movement is still movement.
        let nudged = 30.0f32 + f32::EPSILON * 30.0;
        assert_ne!(nudged, 30.0, "test needs a genuinely different float");
        assert!(c.get(KEY, at(nudged)).is_none(), "moved target must miss");
    }

    #[test]
    fn other_pairs_do_not_share_a_slot() {
        let mut c = LosCache::default();
        c.fit(3, 4, 2);
        c.put(KEY, at(30.0), dummy(0.5));

        // Same index, different list: ground 2 and air 2 are different assets.
        let air = Key {
            kind: TargetKind::Air,
            ..KEY
        };
        assert!(c.get(air, at(30.0)).is_none());
        // Same target, different sensor.
        let other = Key { sensor: 0, ..KEY };
        assert!(c.get(other, at(30.0)).is_none());
    }

    #[test]
    fn resizing_the_asset_lists_forgets_everything() {
        let mut c = LosCache::default();
        c.fit(3, 4, 0);
        c.put(KEY, at(30.0), dummy(0.5));
        assert!(c.get(KEY, at(30.0)).is_some());

        // Placing a unit renumbers the table, so old contents are meaningless.
        c.fit(3, 5, 0);
        assert!(c.get(KEY, at(30.0)).is_none());
    }

    #[test]
    fn out_of_range_indices_are_ignored_rather_than_panicking() {
        let mut c = LosCache::default();
        c.fit(1, 1, 0);
        let wild = Key {
            sensor: 99,
            kind: TargetKind::Ground,
            target: 99,
        };
        c.put(wild, at(30.0), dummy(0.5)); // must not panic
        assert!(c.get(wild, at(30.0)).is_none());
    }
}
