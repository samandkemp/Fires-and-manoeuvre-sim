//! The append-only logs: everything the simulation did, in the order it did it.
//!
//! Every metric the experiments and the app report is read back from these rather than
//! accumulated alongside, so there is no second bookkeeping path to drift. They are also
//! the observation channel the POMDP layer consumes.
//!
//! Events hold **indices** into the asset lists, which is why removal tombstones instead
//! of shifting those lists (V54) — a shift would silently repoint recorded history at
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

/// One resolved fires effect: a shooter's rounds killed elements of a target this epoch.
#[derive(Clone, Debug, PartialEq)]
pub struct FireEvent {
    /// Sim time, seconds.
    pub time_s: f64,
    /// Index of the shooting unit.
    pub shooter: usize,
    /// Index of the target unit.
    pub target: usize,
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
