//! The app's resources and the small types the panel and input handlers share.
//!
//! Data only. Anything that *does* something lives in the module that owns that job:
//! [`crate::ui`] draws the panel, [`crate::input`] reads the mouse, [`crate::overlays`]
//! and [`crate::markers`] draw over the map.

use bevy::prelude::*;
use bevy::tasks::Task;
use bevy::window::PrimaryWindow;
use sim_core::los;
use sim_core::sim::{Side, Sim};
use std::sync::Arc;

use crate::terrain_view;

/// The simulation plus the stat-block libraries placements draw from.
#[derive(Resource)]
pub struct SimRes {
    pub sim: Sim,
    pub data: terrain_view::LoadedData,
    /// How many assets the user has placed by hand, for generating unique ids.
    pub placed: u32,
}

/// One selected asset. Selection spans every asset list, so a box-select can pick up a
/// mixed group and the same commands apply to all of it.
///
/// Batteries and posts are here because they are placeable, and anything placeable has to
/// be removable - an asset you can put on the map but never take off is a trap. They are
/// *emplaced*, so the only command they answer is "be somewhere else"; a right-click drags
/// them rather than routing them, which is the decision an emplacement represents anyway.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Selected {
    /// Index into `Sim::units`.
    Unit(usize),
    /// Index into `Sim::air`.
    Air(usize),
    /// Index into `Sim::air_defence`.
    AirDefence(usize),
    /// Index into `Sim::c2`.
    C2(usize),
}

/// What a right-click on the map does.
///
/// Only *placement* is modal. Selecting, moving, routing and deleting are driven by
/// left-click, modifiers and keys, so the common loop - pick a unit, give it a route -
/// no longer means toggling a radio button between every step.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ClickMode {
    /// Set the LOS-probe observer.
    Probe,
    /// Place a sensor of the selected type.
    PlaceSensor,
    /// Place a unit of the selected type.
    PlaceUnit,
    /// Place a jammer (an EW bubble that hides nearby friendly units).
    PlaceJammer,
    /// Place a drone of the selected air type, at the panel's altitude/heading/speed.
    PlaceAir,
    /// Place an air-defence battery of the selected type.
    PlaceAirDefence,
    /// Place a C2 post, which coordinates nearby air defence (DESIGN §11).
    PlaceC2,
    /// Send the selected drone(s) to orbit the click at the panel's radius.
    AirOrbit,
    /// Give the selected unit(s) an objective to plan their own way to (§10.5).
    SetObjective,
}

impl ClickMode {
    /// Whether this mode places an asset, and so reads [`UiState::place_side`].
    ///
    /// Probing and orbiting act on what is already there, so offering a side for them would
    /// be a control that does nothing.
    #[must_use]
    pub fn places_an_asset(self) -> bool {
        !matches!(self, Self::Probe | Self::AirOrbit | Self::SetObjective)
    }
}

/// A reset requested from the panel, applied after the egui closure releases the sim.
#[derive(Clone, PartialEq, Eq)]
pub enum ResetKind {
    /// Nothing requested this frame.
    None,
    /// Rebuild the sim fresh from the currently loaded scenario.
    Scenario,
    /// Clear all placed assets, keeping the terrain.
    Clear,
    /// Load a different scenario by name, rebuilding terrain and all assets.
    Load(String),
    /// Replay the current scenario at the panel's seed, keeping the terrain.
    Reseed,
}

/// What stops the clock the instant it happens.
///
/// A battle resolves in a couple of hundred seconds of sim time, and the moments worth
/// watching - first contact, the round that kills, the missile leaving the rail - each
/// last a single tick. Slowing playback down is not enough on its own: you still have to
/// be looking at the right pixel at the right moment. A breakpoint pauses *on* the tick
/// that produced the event, leaving the map showing the instant it happened.
///
/// Detected by watching the sim's own event logs grow, so a breakpoint can never
/// disagree with what the feed reports.
#[derive(Default, Clone, Copy)]
pub struct Breakpoints {
    /// Any new detection, ground or air, either side.
    pub detection: bool,
    /// Any ground sub-element destroyed, by fires or by an air-delivered munition.
    pub casualty: bool,
    /// Any air-defence shot, or any munition released by a strike drone.
    pub air_action: bool,
}

impl Breakpoints {
    /// Is anything armed? Used to skip the log snapshot entirely when nothing is.
    pub fn any(self) -> bool {
        self.detection || self.casualty || self.air_action
    }
}

/// Log lengths before a tick, so what the tick *added* can be read off afterwards.
#[derive(Clone, Copy)]
pub struct LogMarks {
    pub detections: usize,
    pub air_detections: usize,
    pub fires: usize,
    pub air_defence: usize,
    pub strikes: usize,
}

impl LogMarks {
    /// Snapshot where every log currently ends.
    pub fn take(sim: &Sim) -> Self {
        Self {
            detections: sim.events().len(),
            air_detections: sim.air_events().len(),
            fires: sim.fire_events().len(),
            air_defence: sim.air_defence_events().len(),
            strikes: sim.strike_events().len(),
        }
    }

    /// Did anything armed in `bp` happen since this mark?
    pub fn tripped(self, sim: &Sim, bp: Breakpoints) -> bool {
        if bp.detection
            && (sim.events().len() > self.detections
                || sim.air_events().len() > self.air_detections)
        {
            return true;
        }
        // Casualties, not shots: a burst that hurt nobody is not a moment worth stopping
        // for, and the fires log records an entry only when rounds actually told.
        if bp.casualty
            && (sim.fire_events()[self.fires..]
                .iter()
                .any(|e| e.casualties > 0)
                || sim.strike_events()[self.strikes..]
                    .iter()
                    .any(|e| e.casualties > 0))
        {
            return true;
        }
        bp.air_action
            && (sim.air_defence_events().len() > self.air_defence
                || sim.strike_events().len() > self.strikes)
    }
}

/// Panel state: interaction mode, selected types, clock control.
#[derive(Resource)]
pub struct UiState {
    /// What a right-click places, if anything.
    pub mode: ClickMode,
    /// Which force the next placed asset joins.
    ///
    /// Placement used to hardcode a side per mode - Blue sensors, Red units, Blue air
    /// defence - which meant the counter-sensing fight the whole tool is about could not be
    /// set up from the map at all. One side control over side-agnostic modes covers both
    /// forces with half the modes.
    pub place_side: Side,
    /// Caution used when giving a unit an objective: metres of movement cost it will spend
    /// to avoid one unit of exposure (§5.1).
    pub risk_weight: f32,
    /// Whose sensors the map overlays are computed from.
    pub overlay_side: Side,
    pub sensor_type_id: String,
    pub unit_type_id: String,
    pub air_type_id: String,
    pub air_defence_type_id: String,
    pub c2_type_id: String,
    /// Is the clock running?
    pub running: bool,
    /// Sim seconds per real second while running.
    ///
    /// The sim still advances only in whole `dt_s` ticks - the wall clock decides *when*
    /// a tick happens, never how big it is - so playback speed changes nothing about the
    /// result. Below 1× the same run simply takes longer to watch.
    pub speed_x: f32,
    /// Fraction of a tick carried over from the previous frame, so a speed that does not
    /// divide the frame time evenly still advances at the right average rate.
    pub tick_budget_s: f32,
    /// Screenshot rig only: step exactly this many ticks per rendered frame, ignoring the
    /// wall clock. The capture must land at a reproducible sim time, and a wall-clock
    /// rate would make it depend on how fast the machine happens to be.
    pub ticks_per_frame: Option<u32>,
    /// What pauses the clock automatically.
    pub breakpoints: Breakpoints,
    /// Target for the "run to" button, in sim seconds.
    pub run_to_s: f32,
    /// Everything currently selected - units and air together.
    pub selected: Vec<Selected>,
    /// Where a left-drag box-select started, in world metres.
    pub drag_start: Option<Vec2>,
    /// Seed for "Re-run at seed" - reproducing a specific run is a first-class need for
    /// an OR tool, and was previously only reachable by editing the scenario file.
    pub seed: u64,
    /// Exposure window (s) for the Pd coverage overlay - live-tweakable.
    pub coverage_exposure_s: f32,
    /// Dials applied to the next placed drone, and to the selected one live.
    pub air_altitude_m: f32,
    pub air_altitude_amsl: bool,
    pub air_heading_deg: f32,
    pub air_speed_m_s: f32,
    pub air_orbit_radius_m: f32,
}

/// Interactive LOS probe (right-click in Probe mode places the observer).
#[derive(Resource, Default)]
pub struct Probe {
    pub observer: Option<Vec2>,
    pub demo_target: Option<Vec2>,
    pub last: Option<los::LosResult>,
}

/// Which overlay is on screen, or being computed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OverlayKind {
    /// Per-cell `P(detect by T)` for the most recent sensor of the chosen side.
    Coverage,
    /// Where the enemy could still be, given nothing has been seen (§8.2).
    BeliefSnapshot,
    /// The belief the simulation is actually flying on, as the tasking layer reads it.
    SimBelief,
}

impl OverlayKind {
    /// Label for the panel and the status line.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Coverage => "coverage (Pd)",
            Self::BeliefSnapshot => "belief snapshot",
            Self::SimBelief => "sim belief",
        }
    }
}

/// What an overlay was asked for, and what it was computed against.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct OverlayRequest {
    /// Which overlay.
    pub kind: OverlayKind,
    /// Whose sensors it is computed from.
    pub side: Side,
    /// Exposure window, seconds.
    pub exposure_s: f32,
}

/// The map overlay: what is showing, what is being computed, and whether it still describes
/// the simulation.
///
/// An overlay used to be a synchronous snapshot: pressing a button blocked the frame for as
/// long as the raster took - up to seconds for a long-ranged sensor - and the result then
/// silently went on describing where a sensor *used to be*. Both halves are fixed here: the
/// raster is computed on a background task, and a fingerprint of the assets it was built
/// from says when it has stopped being true.
#[derive(Resource, Default)]
pub struct Overlay {
    /// The sprite currently on screen.
    pub sprite: Option<Entity>,
    /// Terrain shared with background tasks. Cloned once per scenario, because terrain does
    /// not change during a run - which is what makes sharing it safe and cheap.
    pub terrain: Option<Arc<sim_core::terrain::TerrainGrid>>,
    /// The raster being computed, if one is in flight.
    pub job: Option<Task<OverlayRaster>>,
    /// What that job is computing.
    pub pending: Option<OverlayRequest>,
    /// What is on screen now.
    pub showing: Option<OverlayRequest>,
    /// Fingerprint of the assets the visible overlay was computed from.
    pub built_from: u64,
    /// Recompute automatically when the fingerprint changes.
    pub auto: bool,
    /// A finished raster waiting to be painted.
    pub ready: Option<OverlayRaster>,
    /// Real seconds since the last automatic rebuild.
    ///
    /// Auto-refresh is throttled because the fingerprint moves whenever anything is
    /// detected, and a running battle changes it several times a second. Without a floor,
    /// finishing a raster would immediately start the next one and a core would spin
    /// permanently for a picture nobody could read changing that fast.
    pub since_auto_s: f32,
}

/// A finished raster, on its way back from a background task.
pub struct OverlayRaster {
    /// The values, row-major, already normalised for painting.
    pub cells: ndarray::Array2<f32>,
    /// What it was computed for.
    pub request: OverlayRequest,
    /// The fingerprint at the moment the inputs were snapshotted.
    pub fingerprint: u64,
    /// How long it took, for the status line.
    pub took: std::time::Duration,
}

/// A cheap summary of everything an overlay depends on.
///
/// Positions are quantised to a metre before hashing: a drone drifting continuously would
/// otherwise change the fingerprint every tick and an auto-refreshing overlay would never
/// stop recomputing. A metre is far below the resolution any overlay is painted at.
#[must_use]
pub fn overlay_fingerprint(sim: &Sim) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for (i, s) in sim.sensors().iter().enumerate() {
        let (pos, height, facing) = sim.sensor_view(i);
        sim.sensor_active(i).hash(&mut h);
        (pos.x as i64, pos.y as i64, height as i64, facing as i64).hash(&mut h);
        (s.side == Side::Blue).hash(&mut h);
    }
    for j in sim.jammers() {
        (j.jammer.pos.x as i64, j.jammer.pos.y as i64).hash(&mut h);
        (j.jammer.power as i64, j.jammer.radius_m as i64).hash(&mut h);
    }
    // Belief moves with what has been seen, not only with where things are.
    sim.events().len().hash(&mut h);
    h.finish()
}

/// The map sprite, excluded from the camera so the two `Transform`s can be held at once.
pub type MapSpriteQuery<'w, 's> = Query<
    'w,
    's,
    (&'static mut Sprite, &'static mut Transform),
    (With<MapSprite>, Without<Camera2d>),
>;

/// The camera's transform and projection, for reframing on a scenario switch.
pub type CameraFrameQuery<'w, 's> =
    Query<'w, 's, (&'static mut Transform, &'static mut Projection), With<Camera2d>>;

/// The primary window, for turning a cursor position into world metres.
pub type WindowQuery<'w, 's> = Query<'w, 's, &'static Window, With<PrimaryWindow>>;

/// The camera, for that same conversion.
pub type CameraQuery<'w, 's> =
    Query<'w, 's, (&'static Camera, &'static GlobalTransform), With<Camera2d>>;

/// The map's terrain sprite, so a scenario switch can re-texture it in place.
#[derive(Component)]
pub struct MapSprite;

/// A scenario the panel has asked to load. Applied by `apply_scenario_load` rather than
/// inline, because switching scenario re-textures the map and moves the camera - queries
/// the egui panel cannot hold at the same time as the ones it already borrows.
#[derive(Resource, Default)]
pub struct PendingLoad(pub Option<String>);

/// Eye height for the probe endpoints (metres).
pub const PROBE_HEIGHT_M: f32 = 2.0;
/// Exposure time for the Pd coverage overlay: colour shows P(detect by this long).
pub const COVERAGE_EXPOSURE_S: f32 = 60.0;

/// Playback speed the app starts at, in sim seconds per real second.
///
/// 10× is roughly the fastest a battle can be followed by eye: at 60× - what one
/// tick per frame used to give - a ten-minute engagement is over in ten seconds and the
/// detections that decide it flick past in one or two frames.
pub const DEFAULT_SPEED_X: f32 = 10.0;
/// Most ticks one frame may run, whatever the speed. A frame that stalls (a scenario
/// load, a window drag) hands back a huge delta; without this the next frame would try
/// to swallow the whole gap at once and appear to hang.
pub const MAX_TICKS_PER_FRAME: u32 = 64;
/// Longest frame delta the clock will honour, seconds. Same reason as above, applied
/// before the multiply rather than after, so it holds at every speed.
pub const MAX_FRAME_DELTA_S: f32 = 0.25;
