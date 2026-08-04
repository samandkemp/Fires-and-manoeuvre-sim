//! The app's resources and the small types the panel and input handlers share.
//!
//! Data only. Anything that *does* something lives in the module that owns that job:
//! [`crate::ui`] draws the panel, [`crate::input`] reads the mouse, [`crate::overlays`]
//! and [`crate::markers`] draw over the map.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use sim_core::los;
use sim_core::sim::Sim;

use crate::terrain_view;

/// The simulation plus the stat-block libraries placements draw from.
#[derive(Resource)]
pub struct SimRes {
    pub sim: Sim,
    pub data: terrain_view::LoadedData,
    /// How many assets the user has placed by hand, for generating unique ids.
    pub placed: u32,
}

/// One selected asset. Selection spans both lists, so a box-select can pick up a mixed
/// group and the same commands apply to all of it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Selected {
    /// Index into `Sim::units`.
    Unit(usize),
    /// Index into `Sim::air`.
    Air(usize),
}

/// What a right-click on the map does.
///
/// Only *placement* is modal. Selecting, moving, routing and deleting are driven by
/// left-click, modifiers and keys, so the common loop — pick a unit, give it a route —
/// no longer means toggling a radio button between every step.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ClickMode {
    /// Set the LOS-probe observer.
    Probe,
    /// Place a Blue sensor of the selected type.
    PlaceBlueSensor,
    /// Place a Red unit of the selected type.
    PlaceRedUnit,
    /// Place a Red jammer (EW bubble that hides nearby Red units).
    PlaceRedJammer,
    /// Place a Red drone of the selected air type, at the panel's altitude/heading/speed.
    PlaceRedAir,
    /// Place a Blue air-defence battery of the selected type.
    PlaceBlueAirDefence,
    /// Send the selected drone(s) to orbit the click at the panel's radius.
    AirOrbit,
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

/// Panel state: interaction mode, selected types, clock control.
#[derive(Resource)]
pub struct UiState {
    /// What a right-click places, if anything.
    pub mode: ClickMode,
    pub sensor_type_id: String,
    pub unit_type_id: String,
    pub air_type_id: String,
    pub air_defence_type_id: String,
    /// Is the clock running?
    pub running: bool,
    pub ticks_per_frame: u32,
    /// Everything currently selected — units and air together.
    pub selected: Vec<Selected>,
    /// Where a left-drag box-select started, in world metres.
    pub drag_start: Option<Vec2>,
    /// Seed for "Re-run at seed" — reproducing a specific run is a first-class need for
    /// an OR tool, and was previously only reachable by editing the scenario file.
    pub seed: u64,
    /// Exposure window (s) for the Pd coverage overlay — live-tweakable.
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

/// The current coverage-overlay sprite, if on screen.
#[derive(Resource, Default)]
pub struct Overlay(pub Option<Entity>);

/// A sensor paired with its effective placement: (position, height above its own ground,
/// facing). A carried sensor reports its airframe's, not its own mount height.
pub type PlacedSensor<'a> = (&'a sim_core::sim::SensorState, (Vec2, f32, f32));

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
/// inline, because switching scenario re-textures the map and moves the camera — queries
/// the egui panel cannot hold at the same time as the ones it already borrows.
#[derive(Resource, Default)]
pub struct PendingLoad(pub Option<String>);

/// Eye height for the probe endpoints (metres).
pub const PROBE_HEIGHT_M: f32 = 2.0;
/// Exposure time for the Pd coverage overlay: colour shows P(detect by this long).
pub const COVERAGE_EXPOSURE_S: f32 = 60.0;
