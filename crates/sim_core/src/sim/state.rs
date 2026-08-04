//! What a placed asset *is*: the state structs the simulation advances.
//!
//! Data only — no behaviour beyond trivial accessors. The logic that reads and writes
//! these lives in the sibling modules (`setup`, `detection`, `engagement`,
//! `counter_air`).

use crate::ew::Jammer;
use crate::fires::WeaponType;
use crate::sensing::{SensorType, UnitType};
use crate::suppression::Suppression;
use glam::Vec2;

/// Which force an asset belongs to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    /// Blue force (the user's, conventionally).
    Blue,
    /// Red force.
    Red,
}

/// A placed jammer with its owning side.
#[derive(Clone, Copy, Debug)]
pub struct JammerState {
    /// Owning side (protects this side's units).
    pub side: Side,
    /// The jammer's effect parameters.
    pub jammer: Jammer,
}

/// A placed sensor with its resolved stat block.
#[derive(Clone, Debug)]
pub struct SensorState {
    /// Scenario id (unique per scenario, used in event display).
    pub id: String,
    /// Owning side.
    pub side: Side,
    /// World position, metres. For a carried sensor this is **kept in step with the
    /// airframe** each tick (see [`SensorState::carrier`]), so reading it is always safe.
    pub pos: Vec2,
    /// Facing, degrees (0° = east, CCW); only matters with a finite field of regard. A
    /// carried sensor faces where its airframe is pointing, synced each tick.
    pub facing_deg: f32,
    /// Resolved stat block.
    pub stats: SensorType,
    /// Index into `Sim::air` of the airframe carrying this sensor, if any. A carried
    /// sensor takes its position, height and facing from the airframe each tick — which
    /// is all a recce drone is: a mobile, elevated entry in the ordinary sensor list, so
    /// it flows through the ordinary detection loop with no special case.
    pub carrier: Option<usize>,
}

/// A placed unit with its resolved stat block.
#[derive(Clone, Debug)]
pub struct UnitState {
    /// Scenario id.
    pub id: String,
    /// Owning side.
    pub side: Side,
    /// World position, metres.
    pub pos: Vec2,
    /// Resolved stat block.
    pub stats: UnitType,
    /// Resolved weapon, if the unit carries one.
    pub weapon: Option<WeaponType>,
    /// Whether the *opposing* side currently holds a track on this unit.
    ///
    /// Derived from [`UnitState::last_seen_s`] and refreshed at each decision epoch, so
    /// every reader keeps working while the underlying model is now a decaying track
    /// rather than a permanent flag (`docs/DESIGN.md` §10.1).
    pub detected: bool,
    /// Sim time this unit was last observed by the opposing side, if ever.
    pub last_seen_s: Option<f64>,
    /// Sub-elements remaining (attrition removes them one at a time).
    pub elements: u32,
    /// Sub-elements the unit started with.
    pub initial_elements: u32,
    /// Suppression state (gates fire and movement).
    pub suppression: Suppression,
    /// Movement speed along the route, metres/second (`0` = static).
    pub speed_m_s: f32,
    /// Route waypoints (world metres); empty = no route.
    pub route: Vec<Vec2>,
    /// Index of the next waypoint to head for.
    pub route_idx: usize,
}

impl UnitState {
    /// Still in the fight (at least one element remaining).
    #[must_use]
    pub fn alive(&self) -> bool {
        self.elements > 0
    }

    /// Fractional strength in `[0, 1]` (remaining / initial), for display and Lanchester.
    #[must_use]
    pub fn strength(&self) -> f32 {
        if self.initial_elements == 0 {
            0.0
        } else {
            self.elements as f32 / self.initial_elements as f32
        }
    }
}

/// What the glimpse process needs to know about one candidate target, whether it is a
/// ground unit or an airframe. Bundled so `Sim::glimpse` can serve both passes with one
/// signature rather than nine positional arguments.
#[derive(Clone, Copy, Debug)]
pub(crate) struct GlimpseTarget {
    /// Which asset list [`GlimpseTarget::idx`] refers to — the other half of the
    /// line-of-sight cache key.
    pub kind: super::los_cache::TargetKind,
    /// Index of this target within its list.
    pub idx: usize,
    /// World position, metres.
    pub pos: Vec2,
    /// Height above the ground beneath it — the §1.2 actor height.
    pub height_m: f32,
    /// Signature in the *sensor's* modality.
    pub signature: f32,
    /// Terrain concealment in `[0, 1]`; always 0 for an airborne target, which is not
    /// standing in the cell below it (§9.1).
    pub concealment: f32,
    /// Owning side — selects whose jammers protect it.
    pub side: Side,
}
