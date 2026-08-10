//! The append-only logs: everything the simulation did, in the order it did it.
//!
//! Every metric the experiments and the app report is read back from these rather than
//! accumulated alongside, so there is no second bookkeeping path to drift. They are also
//! the observation channel the POMDP layer consumes.
//!
//! Events hold **indices** into the asset lists, which is why removal tombstones instead
//! of shifting those lists (V54) - a shift would silently repoint recorded history at
//! the wrong asset.

use glam::Vec2;

/// One detection: emitted into the log the moment it happens.
#[derive(Clone, Debug, PartialEq)]
pub struct DetectionEvent {
    /// Sim time of the detection, seconds.
    pub time_s: f64,
    /// Index into `Sim::sensors` of the detecting sensor.
    pub sensor: usize,
    /// Index into `Sim::units` of the detected unit.
    pub unit: usize,
    /// Where the unit was when detected.
    pub unit_pos: Vec2,
}

/// What ground fires shot at (`docs/DESIGN.md` §12.4).
///
/// Ground fires used to iterate the unit list alone, which is what made counter-battery
/// against a SAM impossible to express. Every asset class has elements and takes §2.3 area
/// damage identically, so the only thing that had to change was *which lists are searched*
/// - this names the list.
///
/// Ordered so a fire log sorts by target list then index, which keeps the ordering stable
/// as new asset classes are added at the end.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum FireTarget {
    /// Index into `Sim::units`.
    Unit(usize),
    /// Index into `Sim::air_defence`.
    AirDefence(usize),
    /// Index into `Sim::c2`.
    C2(usize),
}

impl FireTarget {
    /// The unit index, if this was a unit. For readers that only care about the ground
    /// fight - most metrics, and every gate written before counter-battery existed.
    #[must_use]
    pub fn unit(self) -> Option<usize> {
        match self {
            Self::Unit(i) => Some(i),
            _ => None,
        }
    }
}

/// One resolved fires effect: a shooter's rounds killed elements of a target this epoch.
#[derive(Clone, Debug, PartialEq)]
pub struct FireEvent {
    /// Sim time, seconds.
    pub time_s: f64,
    /// Index of the shooting unit.
    pub shooter: usize,
    /// What was hit - a unit, an air-defence battery, or a C2 post.
    pub target: FireTarget,
    /// Sub-elements destroyed this epoch.
    pub casualties: u32,
    /// Whether this reduced the target to 0 elements (killed).
    pub killed: bool,
}

/// One air detection: a sensor picked up an enemy airframe (`docs/DESIGN.md` §9).
/// Separate from [`DetectionEvent`] because the two index different asset lists.
#[derive(Clone, Debug, PartialEq)]
pub struct AirDetectionEvent {
    /// Sim time of the detection, seconds.
    pub time_s: f64,
    /// Index into `Sim::sensors` of the detecting sensor.
    pub sensor: usize,
    /// Index into `Sim::air` of the detected airframe.
    pub air: usize,
    /// Where the airframe was when detected.
    pub air_pos: Vec2,
}

/// One resolved air-defence shot (`docs/DESIGN.md` §9.4).
#[derive(Clone, Debug, PartialEq)]
pub struct AirDefenceEvent {
    /// Sim time, seconds.
    pub time_s: f64,
    /// Index into `Sim::air_defence` of the firing battery.
    pub battery: usize,
    /// Index into `Sim::air` of the engaged airframe.
    pub air: usize,
    /// Did this shot destroy it?
    pub killed: bool,
}

/// One munition released by a strike drone (`docs/DESIGN.md` §9.3).
#[derive(Clone, Debug, PartialEq)]
pub struct StrikeEvent {
    /// Sim time, seconds.
    pub time_s: f64,
    /// Index into `Sim::air` of the releasing airframe.
    pub air: usize,
    /// The aim point the munition was released against.
    pub aim: Vec2,
    /// Where it actually burst (aim + the CEP-sampled miss).
    pub burst: Vec2,
    /// Ground sub-elements destroyed by the burst.
    pub casualties: u32,
}
