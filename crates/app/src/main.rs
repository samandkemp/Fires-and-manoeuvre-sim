//! Bevy front-end: renders the tactical map and drives the `sim_core` simulation.
//! Place sensors, units, jammers, drones and air-defence batteries; run the clock; watch
//! mutual detection, fires and the counter-air fight unfold. The app reads sim state and
//! issues placements; all modelling lives in `sim_core`.

use bevy::prelude::*;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};
use bevy::window::PrimaryWindow;
use bevy_egui::{egui, EguiContexts, EguiPlugin, EguiPrimaryContextPass};
use bevy_pancam::{PanCam, PanCamPlugin};
use sim_core::air::{AltitudeRef, FlightPlan, Terminal};
use sim_core::sim::{Side, Sim};
use sim_core::suppression::Suppression;
use sim_core::{los, sensing};

mod terrain_view;

/// The simulation plus the stat-block libraries placements draw from.
#[derive(Resource)]
struct SimRes {
    sim: Sim,
    data: terrain_view::LoadedData,
    placed: u32,
}

/// One selected asset. Selection spans both lists, so a box-select can pick up a mixed
/// group and the same commands apply to all of it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Selected {
    /// Index into [`Sim::units`].
    Unit(usize),
    /// Index into [`Sim::air`].
    Air(usize),
}

/// What a right-click on the map does.
///
/// Only *placement* is modal now. Selecting, moving, routing and deleting are driven by
/// left-click, modifiers and keys, so the common loop — pick a unit, give it a route —
/// no longer means toggling a radio button between every step.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ClickMode {
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
enum ResetKind {
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
struct UiState {
    mode: ClickMode,
    sensor_type_id: String,
    unit_type_id: String,
    air_type_id: String,
    air_defence_type_id: String,
    running: bool,
    ticks_per_frame: u32,
    /// Everything currently selected — units and air together.
    selected: Vec<Selected>,
    /// Where a left-drag box-select started, in world metres.
    drag_start: Option<Vec2>,
    /// Seed for "Re-run at seed" — reproducing a specific run is a first-class need for
    /// an OR tool, and was previously only reachable by editing the scenario file.
    seed: u64,
    /// Exposure window (s) for the Pd coverage overlay — live-tweakable.
    coverage_exposure_s: f32,
    /// Dials applied to the next placed drone, and to the selected one live.
    air_altitude_m: f32,
    air_altitude_amsl: bool,
    air_heading_deg: f32,
    air_speed_m_s: f32,
    air_orbit_radius_m: f32,
}

/// Interactive LOS probe (right-click in Probe mode places the observer).
#[derive(Resource, Default)]
struct Probe {
    observer: Option<Vec2>,
    demo_target: Option<Vec2>,
    last: Option<los::LosResult>,
}

/// The current coverage-overlay sprite, if on screen.
#[derive(Resource, Default)]
struct Overlay(Option<Entity>);

/// A sensor paired with its effective placement: (position, height above its own ground,
/// facing). A carried sensor reports its airframe's, not its own mount height.
type PlacedSensor<'a> = (&'a sim_core::sim::SensorState, (Vec2, f32, f32));

/// The map sprite, excluded from the camera so the two `Transform`s can be held at once.
type MapSpriteQuery<'w, 's> = Query<
    'w,
    's,
    (&'static mut Sprite, &'static mut Transform),
    (With<MapSprite>, Without<Camera2d>),
>;

/// The camera's transform and projection, for reframing on a scenario switch.
type CameraFrameQuery<'w, 's> =
    Query<'w, 's, (&'static mut Transform, &'static mut Projection), With<Camera2d>>;

/// The map's terrain sprite, so a scenario switch can re-texture it in place.
#[derive(Component)]
struct MapSprite;

/// A scenario the panel has asked to load. Applied by [`apply_scenario_load`] rather than
/// inline, because switching scenario re-textures the map and moves the camera — queries
/// the egui panel cannot hold at the same time as the ones it already borrows.
#[derive(Resource, Default)]
struct PendingLoad(Option<String>);

/// Eye height for the probe endpoints (metres).
const PROBE_HEIGHT_M: f32 = 2.0;
/// Exposure time for the Pd coverage overlay: colour shows P(detect by this long).
const COVERAGE_EXPOSURE_S: f32 = 60.0;

fn main() {
    let mut app = App::new();
    app.add_plugins(DefaultPlugins)
        .add_plugins(EguiPlugin::default())
        .add_plugins(PanCamPlugin)
        .init_resource::<Overlay>()
        .init_resource::<PendingLoad>()
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (advance_sim, draw_markers, draw_probe, apply_scenario_load),
        )
        .add_systems(EguiPrimaryContextPass, ui_panel);

    // Opt-in framebuffer capture: FIRES_SIM_SCREENSHOT=<path.png> saves one shot a few
    // frames in (and pre-runs the sim briefly so detections are visible).
    if std::env::var_os("FIRES_SIM_SCREENSHOT").is_some() {
        app.add_systems(Update, (capture_screenshot, screenshot_belief));
    }

    app.run();
}

/// The scenario to open: the first CLI argument, or `default`. A bare name resolves
/// inside `scenarios/`; anything with a separator or a `.toml` suffix is taken as a path.
fn requested_scenario() -> String {
    std::env::args()
        .nth(1)
        .filter(|a| !a.starts_with('-'))
        .unwrap_or_else(|| "default".to_owned())
}

fn setup(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let requested = requested_scenario();
    let data = terrain_view::load_scenario(&requested).unwrap_or_else(|e| {
        // A mistyped name should say so and list the alternatives, not panic opaquely.
        eprintln!("could not load scenario '{requested}': {e}");
        eprintln!("available: {}", terrain_view::list_scenarios().join(", "));
        std::process::exit(2);
    });
    let mut sim = Sim::new(&data.scenario, &data.libs, data.scenario.default_seed)
        .expect("default scenario should resolve");

    let terrain = sim.terrain();
    let handle = images.add(terrain_view::terrain_image(terrain));
    let cell = terrain.transform().cell_size_m();
    let extent_x = terrain.width() as f32 * cell;
    let extent_y = terrain.height() as f32 * cell;
    let center = Vec3::new(extent_x / 2.0, extent_y / 2.0, 0.0);

    // Map: one texture-blit sprite; 1 world unit = 1 metre.
    commands.spawn((
        Sprite::from_image(handle),
        Transform {
            translation: center,
            scale: Vec3::splat(cell),
            ..default()
        },
        MapSprite,
    ));

    // Whole map in frame at startup. Pan is **middle-drag only**: left-drag is what
    // box-selects, and giving it to the camera is what forced selection into a mode in
    // the first place. Right-click commands the selection or places.
    commands.spawn((
        Camera2d,
        Projection::Orthographic(OrthographicProjection {
            scale: (extent_x / 1200.0).max(extent_y / 680.0).max(1.0),
            ..OrthographicProjection::default_2d()
        }),
        Transform::from_translation(center),
        PanCam {
            grab_buttons: vec![MouseButton::Middle],
            ..default()
        },
    ));

    let screenshot_mode = std::env::var_os("FIRES_SIM_SCREENSHOT").is_some();
    if screenshot_mode {
        // A Red EW bubble for the Phase 8 capture (belief overlay lights it up).
        sim.add_jammer(Side::Red, Vec2::new(6800.0, 6500.0), 0.9, 1000.0);
    }
    commands.insert_resource(Probe {
        observer: screenshot_mode.then(|| Vec2::new(extent_x * 0.28, extent_y * 0.30)),
        demo_target: screenshot_mode.then(|| Vec2::new(extent_x * 0.75, extent_y * 0.72)),
        last: None,
    });
    commands.insert_resource(UiState {
        mode: ClickMode::Probe,
        sensor_type_id: data.libs.sensors.keys().next().cloned().unwrap_or_default(),
        unit_type_id: data.libs.units.keys().next().cloned().unwrap_or_default(),
        air_type_id: data.libs.air.keys().next().cloned().unwrap_or_default(),
        air_defence_type_id: data
            .libs
            .air_defence
            .keys()
            .next()
            .cloned()
            .unwrap_or_default(),
        running: screenshot_mode,
        // In screenshot mode, one tick/frame so the capture lands mid-bombardment
        // (suppression visible on the target) rather than in the aftermath.
        ticks_per_frame: 1,
        selected: Vec::new(),
        drag_start: None,
        seed: data.scenario.default_seed,
        coverage_exposure_s: COVERAGE_EXPOSURE_S,
        air_altitude_m: 400.0,
        air_altitude_amsl: false,
        air_heading_deg: 180.0,
        air_speed_m_s: 45.0,
        air_orbit_radius_m: 600.0,
    });
    commands.insert_resource(SimRes {
        sim,
        data,
        placed: 0,
    });
}

/// Load a scenario the panel asked for: rebuild the sim, re-texture the map, and frame
/// the new extent.
///
/// Separate from the panel because a scenario switch changes the *terrain*, so it must
/// hold mutable access to the map sprite and the camera — queries the panel cannot take
/// while it already borrows the camera immutably for its click handling.
fn apply_scenario_load(
    mut pending: ResMut<PendingLoad>,
    mut sim: ResMut<SimRes>,
    mut ui_state: ResMut<UiState>,
    mut probe: ResMut<Probe>,
    mut images: ResMut<Assets<Image>>,
    mut map: MapSpriteQuery,
    mut camera: CameraFrameQuery,
) {
    let Some(name) = pending.0.take() else {
        return;
    };
    let data = match terrain_view::load_scenario(&name) {
        Ok(d) => d,
        Err(e) => {
            // A bad scenario must not take the app down mid-session — keep the old one.
            error!("could not load scenario '{name}': {e}");
            return;
        }
    };
    let fresh = match Sim::new(&data.scenario, &data.libs, data.scenario.default_seed) {
        Ok(s) => s,
        Err(e) => {
            error!("scenario '{name}' does not resolve: {e}");
            return;
        }
    };

    sim.sim = fresh;
    sim.data = data;
    sim.placed = 0;
    ui_state.selected.clear();
    ui_state.running = false;
    probe.observer = None;

    // Re-texture the map and reframe, exactly as `setup` does for the first scenario.
    let terrain = sim.sim.terrain();
    let cell = terrain.transform().cell_size_m();
    let (extent_x, extent_y) = (
        terrain.width() as f32 * cell,
        terrain.height() as f32 * cell,
    );
    let centre = Vec3::new(extent_x / 2.0, extent_y / 2.0, 0.0);
    let handle = images.add(terrain_view::terrain_image(terrain));
    if let Ok((mut sprite, mut transform)) = map.single_mut() {
        *sprite = Sprite::from_image(handle);
        transform.translation = centre;
        transform.scale = Vec3::splat(cell);
    }
    if let Ok((mut cam_tf, mut projection)) = camera.single_mut() {
        cam_tf.translation = centre;
        if let Projection::Orthographic(o) = &mut *projection {
            o.scale = (extent_x / 1200.0).max(extent_y / 680.0).max(1.0);
        }
    }
    info!("loaded scenario '{}'", sim.data.scenario_name);
}

/// Click-pick radius, metres: how near a click must land to grab a marker.
const PICK_RADIUS_M: f32 = 400.0;
/// A left-drag shorter than this is a click, not a box-select.
const BOX_SELECT_MIN_M: f32 = 60.0;

/// The nearest live asset to `pos` within `max_dist_m`, ground or air.
fn nearest_asset(sim: &Sim, pos: Vec2, max_dist_m: f32) -> Option<Selected> {
    let unit = sim
        .nearest_unit(pos, max_dist_m)
        .map(|i| (Selected::Unit(i), sim.units()[i].pos.distance(pos)));
    let air = sim
        .nearest_air(pos, max_dist_m)
        .map(|i| (Selected::Air(i), sim.air()[i].pos.distance(pos)));
    match (unit, air) {
        (Some(u), Some(a)) => Some(if a.1 < u.1 { a.0 } else { u.0 }),
        (Some(u), None) => Some(u.0),
        (None, Some(a)) => Some(a.0),
        (None, None) => None,
    }
}

/// Every live asset on the map, for select-all.
fn all_live_assets(sim: &Sim) -> Vec<Selected> {
    let mut out: Vec<Selected> = sim
        .units()
        .iter()
        .enumerate()
        .filter(|(_, u)| u.alive())
        .map(|(i, _)| Selected::Unit(i))
        .collect();
    out.extend(
        sim.air()
            .iter()
            .enumerate()
            .filter(|(_, a)| a.alive)
            .map(|(i, _)| Selected::Air(i)),
    );
    out
}

/// Every live asset inside the rectangle spanned by two world corners.
fn assets_in_box(sim: &Sim, a: Vec2, b: Vec2) -> Vec<Selected> {
    let (lo, hi) = (a.min(b), a.max(b));
    let inside = |p: Vec2| p.x >= lo.x && p.x <= hi.x && p.y >= lo.y && p.y <= hi.y;
    let mut out: Vec<Selected> = sim
        .units()
        .iter()
        .enumerate()
        .filter(|(_, u)| u.alive() && inside(u.pos))
        .map(|(i, _)| Selected::Unit(i))
        .collect();
    out.extend(
        sim.air()
            .iter()
            .enumerate()
            .filter(|(_, a)| a.alive && inside(a.pos))
            .map(|(i, _)| Selected::Air(i)),
    );
    out
}

/// The centroid of a selection, for formation-preserving moves.
fn selection_centroid(sim: &Sim, selected: &[Selected]) -> Option<Vec2> {
    let mut sum = Vec2::ZERO;
    let mut n = 0.0f32;
    for sel in selected {
        let p = match sel {
            Selected::Unit(i) => sim.units().get(*i).map(|u| u.pos),
            Selected::Air(i) => sim.air().get(*i).map(|a| a.pos),
        };
        if let Some(p) = p {
            sum += p;
            n += 1.0;
        }
    }
    (n > 0.0).then(|| sum / n)
}

/// Move a whole selection to `target`, **preserving formation**: each asset keeps its
/// offset from the group centroid rather than every marker stacking on the click point.
fn move_selection(sim: &mut Sim, selected: &[Selected], target: Vec2) {
    let Some(centroid) = selection_centroid(sim, selected) else {
        return;
    };
    for sel in selected {
        match sel {
            Selected::Unit(i) => {
                let offset = sim.units()[*i].pos - centroid;
                sim.set_unit_pos(*i, target + offset);
            }
            Selected::Air(i) => {
                let offset = sim.air()[*i].pos - centroid;
                let air = sim.air_mut(*i);
                air.pos = target + offset;
                air.set_plan(FlightPlan::default());
            }
        }
    }
}

/// Append a waypoint to every selected asset, offset to preserve formation. A ground unit
/// gains a route waypoint; a drone gains a flight-plan leg.
fn append_waypoint(sim: &mut Sim, selected: &[Selected], target: Vec2) {
    let Some(centroid) = selection_centroid(sim, selected) else {
        return;
    };
    for sel in selected {
        match sel {
            Selected::Unit(i) => {
                let offset = sim.units()[*i].pos - centroid;
                sim.push_waypoint(*i, target + offset);
            }
            Selected::Air(i) => {
                let offset = sim.air()[*i].pos - centroid;
                let air = sim.air_mut(*i);
                if air.plan_complete() {
                    air.set_plan(FlightPlan::route(vec![target + offset]));
                } else {
                    air.plan.waypoints.push(target + offset);
                }
            }
        }
    }
}

/// Advance the sim clock: `ticks_per_frame` ticks of `dt_s` per rendered frame while
/// running (1 tick/frame ≈ 60× real time at 60 fps).
fn advance_sim(mut sim: ResMut<SimRes>, ui: Res<UiState>) {
    if ui.running {
        for _ in 0..ui.ticks_per_frame {
            sim.sim.step_one();
        }
    }
}

/// The control panel, plus right-click map actions (which live here so egui can claim
/// pointer events first).
// egui 0.34 (pulled by bevy_egui 0.40) renamed the panel API (SidePanel→Panel,
// min_width→min_size, wants_pointer_input→egui_wants_pointer_input). The deprecated
// names still work; adopt the new ones when the UI is next reworked.
#[allow(deprecated)]
#[allow(clippy::too_many_arguments)]
fn ui_panel(
    mut contexts: EguiContexts,
    mut sim: ResMut<SimRes>,
    mut ui_state: ResMut<UiState>,
    mut probe: ResMut<Probe>,
    mut overlay: ResMut<Overlay>,
    mut pending_load: ResMut<PendingLoad>,
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    window: Query<&Window, With<PrimaryWindow>>,
    camera: Query<(&Camera, &GlobalTransform), With<Camera2d>>,
) -> Result {
    let ctx = contexts.ctx_mut()?;
    let mut reset_pending = ResetKind::None;

    egui::SidePanel::left("controls")
        .min_width(230.0)
        .show(ctx, |ui| {
            ui.heading("Fires & Manoeuvre Sim");
            ui.label(format!("t = {:.0} s", sim.sim.time_s()));

            ui.separator();
            ui.label("Clock");
            ui.horizontal(|ui| {
                if ui
                    .button(if ui_state.running { "Pause" } else { "Run" })
                    .clicked()
                {
                    ui_state.running = !ui_state.running;
                }
                if ui.button("Step 10 s").clicked() {
                    let until = sim.sim.time_s() + 10.0;
                    sim.sim.run_until(until);
                }
            });
            ui.add(egui::Slider::new(&mut ui_state.ticks_per_frame, 1..=20).text("ticks/frame"));
            ui.horizontal(|ui| {
                if ui.button("Reset scenario").clicked() {
                    reset_pending = ResetKind::Scenario;
                }
                if ui.button("Clear all").clicked() {
                    reset_pending = ResetKind::Clear;
                }
            });
            ui.horizontal(|ui| {
                ui.add(
                    egui::DragValue::new(&mut ui_state.seed)
                        .speed(1.0)
                        .prefix("seed "),
                );
                if ui.button("Re-run at seed").clicked() {
                    reset_pending = ResetKind::Reseed;
                }
            });
            // Scenario picker: every *.toml in scenarios/ that parses as a scenario.
            let current = sim.data.scenario_name.clone();
            egui::ComboBox::from_label("scenario")
                .selected_text(&current)
                .show_ui(ui, |ui| {
                    for name in &sim.data.available {
                        if ui.selectable_label(*name == current, name).clicked() && *name != current
                        {
                            reset_pending = ResetKind::Load(name.clone());
                        }
                    }
                });

            ui.separator();
            ui.collapsing("Controls", |ui| {
                ui.small("Left-click select \u{b7} shift add \u{b7} drag box-select");
                ui.small("Right-click move here \u{b7} shift append waypoint");
                ui.small("Ctrl+A select all \u{b7} Del remove \u{b7} Esc clear");
                ui.small("Middle-drag pan \u{b7} scroll zoom");
            });
            ui.separator();
            ui.label("Right-click places:");
            ui.radio_value(&mut ui_state.mode, ClickMode::Probe, "nothing (move/route)");
            ui.radio_value(
                &mut ui_state.mode,
                ClickMode::PlaceBlueSensor,
                "Place Blue sensor",
            );
            ui.radio_value(
                &mut ui_state.mode,
                ClickMode::PlaceRedUnit,
                "Place Red unit",
            );
            ui.radio_value(
                &mut ui_state.mode,
                ClickMode::PlaceRedJammer,
                "Place Red jammer (EW)",
            );
            ui.radio_value(
                &mut ui_state.mode,
                ClickMode::PlaceRedAir,
                "Place Red drone",
            );
            ui.radio_value(
                &mut ui_state.mode,
                ClickMode::PlaceBlueAirDefence,
                "Place Blue air defence",
            );
            ui.radio_value(
                &mut ui_state.mode,
                ClickMode::AirOrbit,
                "Drone orbit here (radius below)",
            );

            // Selection readout. Drop anything that died or was cleared under us first,
            // so a stale index can never be commanded.
            {
                let sim_ref = &sim.sim;
                ui_state.selected.retain(|sel| match sel {
                    Selected::Unit(i) => sim_ref.units().get(*i).is_some_and(|u| u.alive()),
                    Selected::Air(i) => sim_ref.air().get(*i).is_some_and(|a| a.alive),
                });
            }
            match ui_state.selected.len() {
                0 => {
                    ui.label("Nothing selected");
                }
                1 => match ui_state.selected[0] {
                    Selected::Unit(i) => {
                        let u = &sim.sim.units()[i];
                        ui.label(format!(
                            "Selected: {} ({:?})  {}/{} elem  {:?}",
                            u.id, u.side, u.elements, u.initial_elements, u.suppression
                        ));
                    }
                    Selected::Air(i) => {
                        let a = &sim.sim.air()[i];
                        ui.label(format!("Selected: {} (drone, {:?})", a.id, a.side));
                    }
                },
                n => {
                    let air = ui_state
                        .selected
                        .iter()
                        .filter(|s| matches!(s, Selected::Air(_)))
                        .count();
                    ui.label(format!(
                        "Selected: {n} assets ({} ground, {air} air)",
                        n - air
                    ));
                }
            }

            egui::ComboBox::from_label("sensor type")
                .selected_text(&ui_state.sensor_type_id)
                .show_ui(ui, |ui| {
                    for key in sim.data.libs.sensors.keys() {
                        ui.selectable_value(&mut ui_state.sensor_type_id, key.clone(), key);
                    }
                });
            egui::ComboBox::from_label("unit type")
                .selected_text(&ui_state.unit_type_id)
                .show_ui(ui, |ui| {
                    for key in sim.data.libs.units.keys() {
                        ui.selectable_value(&mut ui_state.unit_type_id, key.clone(), key);
                    }
                });

            // --- Air (docs/DESIGN.md §9) ------------------------------------------
            ui.separator();
            egui::CollapsingHeader::new("Air")
                .default_open(true)
                .show(ui, |ui| {
                    egui::ComboBox::from_label("drone type")
                        .selected_text(&ui_state.air_type_id)
                        .show_ui(ui, |ui| {
                            for key in sim.data.libs.air.keys() {
                                ui.selectable_value(&mut ui_state.air_type_id, key.clone(), key);
                            }
                        });
                    egui::ComboBox::from_label("AD type")
                        .selected_text(&ui_state.air_defence_type_id)
                        .show_ui(ui, |ui| {
                            for key in sim.data.libs.air_defence.keys() {
                                ui.selectable_value(
                                    &mut ui_state.air_defence_type_id,
                                    key.clone(),
                                    key,
                                );
                            }
                        });
                    ui.add(
                        egui::Slider::new(&mut ui_state.air_altitude_m, 0.0..=2000.0)
                            .text("altitude m"),
                    );
                    ui.checkbox(
                        &mut ui_state.air_altitude_amsl,
                        "altitude is AMSL (terrain can mask)",
                    );
                    ui.add(
                        egui::Slider::new(&mut ui_state.air_heading_deg, 0.0..=359.0)
                            .text("heading °"),
                    );
                    ui.add(
                        egui::Slider::new(&mut ui_state.air_speed_m_s, 0.0..=120.0)
                            .text("speed m/s"),
                    );
                    ui.add(
                        egui::Slider::new(&mut ui_state.air_orbit_radius_m, 100.0..=2000.0)
                            .text("orbit radius m"),
                    );

                    // Selected drone: the dials above are applied live, so altitude and speed can
                    // be flown by hand while the clock runs.
                    let selected_air: Vec<usize> = ui_state
                        .selected
                        .iter()
                        .filter_map(|s| match s {
                            Selected::Air(i) => Some(*i),
                            Selected::Unit(_) => None,
                        })
                        .collect();
                    if let [only] = selected_air[..] {
                        let a = &sim.sim.air()[only];
                        let agl = a.actor_height(sim.sim.terrain());
                        ui.label(format!(
                            "Drone {}: {:.0} m ({agl:.0} AGL), {:.0} m/s, hdg {:.0}°",
                            a.id, a.altitude_m, a.speed_m_s, a.heading_deg
                        ));
                        ui.small(format!(
                            "munitions {}  {}",
                            a.munitions_left,
                            if a.detected { "DETECTED" } else { "undetected" }
                        ));
                    }
                    // The dials apply to the whole air selection, so a formation can be re-tasked
                    // in one go rather than drone by drone.
                    if !selected_air.is_empty()
                        && ui
                            .button(format!("Apply dials to {} drone(s)", selected_air.len()))
                            .clicked()
                    {
                        for i in selected_air {
                            let a = sim.sim.air_mut(i);
                            a.altitude_m = ui_state.air_altitude_m;
                            a.altitude_ref = if ui_state.air_altitude_amsl {
                                AltitudeRef::Amsl
                            } else {
                                AltitudeRef::Agl
                            };
                            a.heading_deg = ui_state.air_heading_deg;
                            a.speed_m_s = ui_state.air_speed_m_s;
                        }
                    }
                });

            ui.separator();
            ui.add(
                egui::Slider::new(&mut ui_state.coverage_exposure_s, 5.0..=180.0)
                    .text("exposure s"),
            );
            if ui.button("Coverage overlay (Pd)").clicked() {
                rebuild_coverage_overlay(
                    &sim,
                    ui_state.coverage_exposure_s,
                    &mut overlay,
                    &mut commands,
                    &mut images,
                );
            }
            if ui.button("Belief overlay (where Red could hide)").clicked() {
                rebuild_belief_overlay(
                    &sim,
                    ui_state.coverage_exposure_s,
                    &mut overlay,
                    &mut commands,
                    &mut images,
                );
            }
            ui.label("(coverage/belief from Blue sensors, vs 'afv')");

            ui.separator();
            if let Some(r) = &probe.last {
                ui.label(format!(
                    "LOS: {}  τ = {:.2}\nmask {:+.1} m, canopy {:.0} m",
                    if r.clear { "CLEAR" } else { "BLOCKED" },
                    r.transmittance,
                    r.mask_height,
                    r.canopy_length
                ));
            }

            ui.separator();
            let blue_detected = sim
                .sim
                .units()
                .iter()
                .filter(|u| u.side == Side::Blue && u.detected)
                .count();
            let red_detected = sim
                .sim
                .units()
                .iter()
                .filter(|u| u.side == Side::Red && u.detected)
                .count();
            ui.label(format!(
                "Detected: {red_detected} red / {blue_detected} blue"
            ));
            let elements = |side: Side| -> u32 {
                sim.sim
                    .units()
                    .iter()
                    .filter(|u| u.side == side)
                    .map(|u| u.elements)
                    .sum()
            };
            ui.label(format!(
                "Elements: blue {} / red {}",
                elements(Side::Blue),
                elements(Side::Red)
            ));
            let suppressed = sim
                .sim
                .units()
                .iter()
                .filter(|u| u.suppression != Suppression::Free && u.alive())
                .count();
            if suppressed > 0 {
                ui.label(format!("{suppressed} unit(s) under suppression"));
            }

            ui.label("Detections:");
            for e in sim.sim.events().iter().rev().take(5) {
                let s = &sim.sim.sensors()[e.sensor];
                let u = &sim.sim.units()[e.unit];
                ui.small(format!("t={:>4.0}s  {} spotted {}", e.time_s, s.id, u.id));
            }
            ui.label("Fires:");
            for e in sim.sim.fire_events().iter().rev().take(6) {
                let sh = &sim.sim.units()[e.shooter];
                let tg = &sim.sim.units()[e.target];
                ui.small(format!(
                    "t={:>4.0}s  {} hit {} \u{2013}{}{}",
                    e.time_s,
                    sh.id,
                    tg.id,
                    e.casualties,
                    if e.killed { " KILL" } else { "" }
                ));
            }

            // Air activity feed (docs/DESIGN.md §9).
            let air_alive = sim.sim.air().iter().filter(|a| a.alive).count();
            let air_lost = sim.sim.air().len() - air_alive;
            if !sim.sim.air().is_empty() || !sim.sim.air_defence().is_empty() {
                ui.label(format!(
                    "Air: {air_alive} flying / {air_lost} down   AD: {} batteries",
                    sim.sim.air_defence().len()
                ));
                for ad in sim.sim.air_defence() {
                    let mag = if ad.stats.magazine == 0 {
                        "∞".to_owned()
                    } else {
                        ad.magazine_left.to_string()
                    };
                    ui.small(format!(
                        "  {}: {} rounds, {} engaging{}",
                        ad.id,
                        mag,
                        ad.engagements.len(),
                        if ad.self_cue { "" } else { " (net-cued)" }
                    ));
                }
                ui.label("Air events:");
                for e in sim.sim.air_defence_events().iter().rev().take(4) {
                    let ad = &sim.sim.air_defence()[e.battery];
                    let a = &sim.sim.air()[e.air];
                    ui.small(format!(
                        "t={:>4.0}s  {} {} {}",
                        e.time_s,
                        ad.id,
                        if e.killed { "DOWNED" } else { "missed" },
                        a.id
                    ));
                }
                for e in sim.sim.strike_events().iter().rev().take(4) {
                    let a = &sim.sim.air()[e.air];
                    ui.small(format!(
                        "t={:>4.0}s  {} released \u{2013}{} elem",
                        e.time_s, a.id, e.casualties
                    ));
                }
            }

            ui.separator();
            ui.collapsing("Legend", |ui| {
                ui.small("○ sensor   ◇ unit   ▷ drone   ✕ destroyed");
                ui.small("blue = friendly, red = enemy");
                ui.small("white ring = detected");
                ui.small("amber ring = suppressed, red ring = pinned");
                ui.small("green bar = remaining strength");
                ui.small("faint line = movement route / flight plan");
                ui.small("yellow ring = selected (left-click, shift adds, drag boxes)");
                ui.small("magenta bubble = EW jammer");
                ui.small("teal ring = air-defence envelope");
                ui.small("yellow line = air-defence engagement");
                ui.small("drone triangle grows with altitude");
            });
        });

    // Apply a deferred reset (the sim can't be rebuilt while the egui closure borrows it).
    match reset_pending {
        ResetKind::Scenario => {
            let d = &sim.data;
            let fresh = Sim::new(&d.scenario, &d.libs, d.scenario.default_seed)
                .expect("default scenario resolves");
            sim.sim = fresh;
            sim.placed = 0;
            ui_state.selected.clear();
            ui_state.running = false;
            probe.observer = None;
            if let Some(e) = overlay.0.take() {
                commands.entity(e).despawn();
            }
        }
        ResetKind::Clear => {
            sim.sim.reset(0);
            sim.placed = 0;
            ui_state.selected.clear();
            if let Some(e) = overlay.0.take() {
                commands.entity(e).despawn();
            }
        }
        ResetKind::Reseed => {
            // Terrain is kept: only the stochastic stream changes, which is the same
            // separation the batch runner makes (docs/PROGRESS.md).
            let seed = ui_state.seed;
            // Split the borrow: `data` and `sim` are disjoint fields of the resource, so
            // reach them as fields rather than through `&sim.data` while `sim.sim` is
            // borrowed mutably.
            let SimRes {
                sim: engine, data, ..
            } = &mut *sim;
            if let Err(e) = engine.reset_to_scenario(&data.scenario, &data.libs, seed) {
                error!("could not replay at seed {seed}: {e}");
            }
            sim.placed = 0;
            ui_state.selected.clear();
            ui_state.running = false;
            if let Some(e) = overlay.0.take() {
                commands.entity(e).despawn();
            }
        }
        ResetKind::Load(name) => {
            pending_load.0 = Some(name);
            if let Some(e) = overlay.0.take() {
                commands.entity(e).despawn();
            }
        }
        ResetKind::None => {}
    }

    // ---- Map interaction -------------------------------------------------------
    // Left-click selects (shift adds, drag box-selects); right-click commands the
    // selection. Only placement is still modal, which is what removed the old
    // Select/Move/Route toggling.
    let world_cursor = || -> Option<Vec2> {
        let window = window.single().ok()?;
        let (cam, cam_tf) = camera.single().ok()?;
        window
            .cursor_position()
            .and_then(|c| cam.viewport_to_world_2d(cam_tf, c).ok())
    };
    let wants_pointer = ctx.wants_pointer_input();
    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);

    if keys.just_pressed(KeyCode::Escape) {
        ui_state.selected.clear();
    }
    let ctrl = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
    if ctrl && keys.just_pressed(KeyCode::KeyA) {
        ui_state.selected = all_live_assets(&sim.sim);
    }
    if keys.just_pressed(KeyCode::Delete) {
        for sel in std::mem::take(&mut ui_state.selected) {
            match sel {
                Selected::Unit(i) => sim.sim.remove_unit(i),
                Selected::Air(i) => sim.sim.remove_air(i),
            }
        }
    }

    // A left press starts a possible box-select; the release decides click vs box.
    if buttons.just_pressed(MouseButton::Left) && !wants_pointer {
        ui_state.drag_start = world_cursor();
    }
    if buttons.just_released(MouseButton::Left) {
        if let (Some(start), Some(end)) = (ui_state.drag_start.take(), world_cursor()) {
            if !wants_pointer {
                let picked = if start.distance(end) > BOX_SELECT_MIN_M {
                    assets_in_box(&sim.sim, start, end)
                } else {
                    nearest_asset(&sim.sim, end, PICK_RADIUS_M)
                        .into_iter()
                        .collect()
                };
                if shift {
                    // Toggle, so shift-clicking something already selected removes it.
                    for a in picked {
                        if let Some(at) = ui_state.selected.iter().position(|s| *s == a) {
                            ui_state.selected.remove(at);
                        } else {
                            ui_state.selected.push(a);
                        }
                    }
                } else {
                    ui_state.selected = picked;
                }
            }
        }
    }

    if buttons.just_pressed(MouseButton::Right) && !wants_pointer {
        if let Some(world) = world_cursor() {
            match ui_state.mode {
                ClickMode::Probe => {
                    // No placement mode set: right-click commands the selection. With
                    // nothing selected it falls back to placing the LOS probe.
                    if ui_state.selected.is_empty() {
                        probe.observer = Some(world);
                    } else if shift {
                        append_waypoint(&mut sim.sim, &ui_state.selected, world);
                    } else {
                        move_selection(&mut sim.sim, &ui_state.selected, world);
                    }
                }
                ClickMode::AirOrbit => {
                    let radius = ui_state.air_orbit_radius_m;
                    for sel in &ui_state.selected {
                        if let Selected::Air(i) = sel {
                            sim.sim
                                .air_mut(*i)
                                .set_plan(FlightPlan::orbit(world, radius, false));
                        }
                    }
                }
                ClickMode::PlaceRedAir => {
                    if let Some(stats) = sim.data.libs.air.get(&ui_state.air_type_id).cloned() {
                        sim.placed += 1;
                        let id = format!("uas-p{}", sim.placed);
                        let sensor = stats
                            .sensor
                            .as_ref()
                            .and_then(|s| sim.data.libs.sensors.get(s).cloned());
                        let payload = stats
                            .payload
                            .as_ref()
                            .and_then(|w| sim.data.libs.weapons.get(w).cloned());
                        let idx = sim.sim.add_air(
                            &id,
                            Side::Red,
                            world,
                            ui_state.air_altitude_m,
                            if ui_state.air_altitude_amsl {
                                AltitudeRef::Amsl
                            } else {
                                AltitudeRef::Agl
                            },
                            ui_state.air_heading_deg,
                            stats,
                            sensor,
                            payload,
                        );
                        sim.sim.air_mut(idx).speed_m_s = ui_state.air_speed_m_s;
                        ui_state.selected = vec![Selected::Air(idx)];
                    }
                }
                ClickMode::PlaceBlueAirDefence => {
                    if let Some(stats) = sim
                        .data
                        .libs
                        .air_defence
                        .get(&ui_state.air_defence_type_id)
                        .cloned()
                    {
                        sim.placed += 1;
                        let id = format!("ad-p{}", sim.placed);
                        let sensor = stats
                            .sensor
                            .as_ref()
                            .and_then(|s| sim.data.libs.sensors.get(s).cloned());
                        sim.sim
                            .add_air_defence(&id, Side::Blue, world, stats, true, sensor);
                    }
                }
                ClickMode::PlaceBlueSensor => {
                    sim.placed += 1;
                    let id = format!("obs-p{}", sim.placed);
                    let stats = sim.data.libs.sensors[&ui_state.sensor_type_id].clone();
                    sim.sim.add_sensor(&id, Side::Blue, world, 0.0, stats);
                }
                ClickMode::PlaceRedUnit => {
                    sim.placed += 1;
                    let id = format!("tgt-p{}", sim.placed);
                    let stats = sim.data.libs.units[&ui_state.unit_type_id].clone();
                    let weapon = stats
                        .weapon
                        .as_ref()
                        .and_then(|w| sim.data.libs.weapons.get(w).cloned());
                    sim.sim.add_unit(&id, Side::Red, world, stats, weapon);
                    ui_state.selected = vec![Selected::Unit(sim.sim.units().len() - 1)];
                }
                ClickMode::PlaceRedJammer => {
                    sim.sim.add_jammer(Side::Red, world, 0.9, 900.0);
                }
            }
        }
    }

    Ok(())
}

/// Pd coverage from the most recently placed Blue sensor against a reference target
/// (`afv` if defined): per cell, `P(detect by T) = 1 − e^{−λT}` painted as the overlay.
fn rebuild_coverage_overlay(
    sim: &SimRes,
    exposure_s: f32,
    overlay: &mut Overlay,
    commands: &mut Commands,
    images: &mut Assets<Image>,
) {
    // Most recently placed live Blue sensor. `sensor_active` matters now that a sensor
    // can be carried: a shot-down recce drone's sensor must not paint coverage.
    let Some((s_idx, sensor)) = sim
        .sim
        .sensors()
        .iter()
        .enumerate()
        .rev()
        .find(|(i, s)| s.side == Side::Blue && sim.sim.sensor_active(*i))
    else {
        return;
    };
    let reference = sim
        .data
        .libs
        .units
        .get("afv")
        .or_else(|| sim.data.libs.units.values().next())
        .expect("at least one unit type");

    let terrain = sim.sim.terrain();
    let t0 = std::time::Instant::now();
    let (h, w) = (terrain.height(), terrain.width());
    // Read the sensor's *effective* placement: for a carried sensor this is the
    // airframe's position, altitude and heading, not the ground mount height.
    let (s_pos, s_height, s_facing) = sim.sim.sensor_view(s_idx);
    let signature = reference.signature_in(sensor.stats.modality);
    let mut pd = ndarray::Array2::<f32>::zeros((h, w));
    ndarray::Zip::indexed(&mut pd).par_for_each(|(iy, ix), v| {
        let target = terrain.transform().cell_center(ix, iy);
        let lambda = sensing::detection_rate_against(
            terrain,
            &sensor.stats,
            s_pos,
            s_height,
            s_facing,
            target,
            reference.height_m,
            signature,
            sensing::concealment_at(terrain, target),
        );
        *v = 1.0 - (-lambda * exposure_s).exp();
    });
    info!(
        "coverage overlay for '{}' took {:?}",
        sensor.id,
        t0.elapsed()
    );

    let handle = images.add(terrain_view::viewshed_image(&pd));
    let cell = terrain.transform().cell_size_m();
    let (ex, ey) = (w as f32 * cell, h as f32 * cell);
    if let Some(e) = overlay.0.take() {
        commands.entity(e).despawn();
    }
    overlay.0 = Some(
        commands
            .spawn((
                Sprite::from_image(handle),
                Transform {
                    translation: Vec3::new(ex / 2.0, ey / 2.0, 1.0),
                    scale: Vec3::splat(cell),
                    ..default()
                },
            ))
            .id(),
    );
}

/// Belief overlay: assuming Blue has *not* detected Red, where could an `afv` be hiding?
/// The product of each Blue sensor's no-detection likelihood (including Red jamming),
/// normalised — mass concentrates in dead ground and inside Red's EW bubbles. This is the
/// Phase 8 partial-observability picture (POMDP negative information + EW).
fn rebuild_belief_overlay(
    sim: &SimRes,
    exposure_s: f32,
    overlay: &mut Overlay,
    commands: &mut Commands,
    images: &mut Assets<Image>,
) {
    // Live Blue sensors, each paired with its *effective* placement — a carried sensor
    // reports its airframe's position, altitude and heading (docs/DESIGN.md §9).
    let blue: Vec<PlacedSensor> = sim
        .sim
        .sensors()
        .iter()
        .enumerate()
        .filter(|(i, s)| s.side == Side::Blue && sim.sim.sensor_active(*i))
        .map(|(i, s)| (s, sim.sim.sensor_view(i)))
        .collect();
    if blue.is_empty() {
        return;
    }
    let reference = sim
        .data
        .libs
        .units
        .get("afv")
        .or_else(|| sim.data.libs.units.values().next())
        .expect("a unit type");
    let red_jammers: Vec<sim_core::ew::Jammer> = sim
        .sim
        .jammers()
        .iter()
        .filter(|j| j.side == Side::Red)
        .map(|j| j.jammer)
        .collect();

    let terrain = sim.sim.terrain();
    let (h, w) = (terrain.height(), terrain.width());
    let cell = terrain.transform().cell_size_m();

    // A moderate grid (parallel over cells): full-resolution over two long-range sensors
    // is ~10 s even threaded, so a light stride keeps the button interactive while still
    // being far crisper than before. Per cell: P(Red stays undetected for the exposure
    // window) = Π_sensors e^{−λ·T} — near 0 inside coverage, ~1 in dead ground/jamming.
    const STRIDE: usize = 3;
    let (cw, ch) = (w / STRIDE, h / STRIDE);
    let t0 = std::time::Instant::now();
    let mut belief = ndarray::Array2::<f32>::zeros((ch, cw));
    ndarray::Zip::indexed(&mut belief).par_for_each(|(cy, cx), v| {
        let ix = (cx * STRIDE + STRIDE / 2).min(w - 1);
        let iy = (cy * STRIDE + STRIDE / 2).min(h - 1);
        let pos = terrain.transform().cell_center(ix, iy);
        let concealment = sensing::concealment_at(terrain, pos);
        let mut p = 1.0f32;
        for (s, (s_pos, s_height, s_facing)) in &blue {
            let lambda = sensing::detection_rate_against(
                terrain,
                &s.stats,
                *s_pos,
                *s_height,
                *s_facing,
                pos,
                reference.height_m,
                reference.signature_in(s.stats.modality),
                concealment,
            ) * sim_core::ew::jamming_factor(pos, &red_jammers);
            p *= (-lambda * exposure_s).exp();
        }
        *v = p;
    });
    let sum = belief.sum();
    if sum > 0.0 {
        belief /= sum;
    }
    info!(
        "belief overlay ({} Blue sensors, {cw}x{ch}) took {:?}",
        blue.len(),
        t0.elapsed()
    );

    let handle = images.add(terrain_view::belief_image(&belief));
    let (ex, ey) = (w as f32 * cell, h as f32 * cell);
    if let Some(e) = overlay.0.take() {
        commands.entity(e).despawn();
    }
    overlay.0 = Some(
        commands
            .spawn((
                Sprite::from_image(handle),
                Transform {
                    translation: Vec3::new(ex / 2.0, ey / 2.0, 1.0),
                    scale: Vec3::splat(cell * STRIDE as f32),
                    ..default()
                },
            ))
            .id(),
    );
}

/// Force markers: sensors as circles, units as diamonds; blue/red by side; a white
/// ring marks units the enemy has detected. Sizes scale with zoom.
fn draw_markers(
    sim: Res<SimRes>,
    ui_state: Res<UiState>,
    camera: Query<&Projection, With<Camera2d>>,
    mut gizmos: Gizmos,
) {
    let px = match camera.single() {
        Ok(Projection::Orthographic(o)) => o.scale,
        _ => 1.0,
    };

    // Selection highlight: a yellow ring around every selected ground asset.
    for sel in &ui_state.selected {
        if let Selected::Unit(idx) = sel {
            if let Some(u) = sim.sim.units().get(*idx).filter(|u| u.alive()) {
                gizmos.circle_2d(
                    Isometry2d::from_translation(u.pos),
                    18.0 * px,
                    Color::srgb(1.0, 0.9, 0.2),
                );
            }
        }
    }
    let side_color = |side: Side| match side {
        Side::Blue => Color::srgb(0.25, 0.55, 1.0),
        Side::Red => Color::srgb(0.95, 0.30, 0.25),
    };

    // Routes: a faint line from each moving unit's current position through its
    // remaining waypoints, with a marker at each waypoint.
    for u in sim.sim.units() {
        if !u.alive() || u.route_idx >= u.route.len() {
            continue;
        }
        let c = side_color(u.side).with_alpha(0.5);
        let mut prev = u.pos;
        for &wp in &u.route[u.route_idx..] {
            gizmos.line_2d(prev, wp, c);
            gizmos.circle_2d(Isometry2d::from_translation(wp), 4.0 * px, c);
            prev = wp;
        }
    }

    // Jammers: a dashed-look bubble (two rings) in a magenta hue.
    for j in sim.sim.jammers() {
        let c = Color::srgb(0.85, 0.2, 0.9);
        gizmos.circle_2d(
            Isometry2d::from_translation(j.jammer.pos),
            j.jammer.radius_m,
            c.with_alpha(0.5),
        );
        gizmos.circle_2d(Isometry2d::from_translation(j.jammer.pos), 6.0 * px, c);
    }

    // Air defence: the engagement envelope as a ring, plus a live line to whatever it is
    // currently engaging — the counter-air fight made visible.
    for ad in sim.sim.air_defence() {
        let c = Color::srgb(0.2, 0.85, 0.75);
        gizmos.circle_2d(
            Isometry2d::from_translation(ad.pos),
            ad.stats.max_range_m,
            c.with_alpha(0.30),
        );
        if ad.stats.min_range_m > 0.0 {
            gizmos.circle_2d(
                Isometry2d::from_translation(ad.pos),
                ad.stats.min_range_m,
                c.with_alpha(0.20),
            );
        }
        gizmos.circle_2d(Isometry2d::from_translation(ad.pos), 9.0 * px, c);
        gizmos.circle_2d(Isometry2d::from_translation(ad.pos), 5.0 * px, c);
        for e in &ad.engagements {
            if let Some(target) = sim.sim.air().get(e.target).filter(|a| a.alive) {
                gizmos.line_2d(ad.pos, target.pos, Color::srgb(1.0, 0.85, 0.2));
            }
        }
    }

    // Air assets: a triangle pointing along the heading, so course is readable at a
    // glance; an orbit plan draws its circle.
    for (i, a) in sim.sim.air().iter().enumerate() {
        let c = side_color(a.side);
        if !a.alive {
            let g = Color::srgb(0.45, 0.45, 0.45);
            let d = 7.0 * px;
            gizmos.line_2d(a.pos + Vec2::new(-d, -d), a.pos + Vec2::new(d, d), g);
            gizmos.line_2d(a.pos + Vec2::new(-d, d), a.pos + Vec2::new(d, -d), g);
            continue;
        }
        // Remaining flight plan.
        let mut prev = a.pos;
        for &wp in a.plan.waypoints.iter().skip(a.route_idx) {
            gizmos.line_2d(prev, wp, c.with_alpha(0.4));
            prev = wp;
        }
        if let (Terminal::Orbit { radius_m, .. }, Some(centre)) =
            (a.plan.terminal, a.plan.destination())
        {
            gizmos.circle_2d(
                Isometry2d::from_translation(centre),
                radius_m,
                c.with_alpha(0.35),
            );
        }

        // Nose-forward triangle. Size grows a little with altitude so height reads on
        // the map without needing a label.
        let scale = (9.0 + a.altitude_m / 250.0) * px;
        let fwd = Vec2::from_angle(a.heading_deg.to_radians());
        let side = Vec2::new(-fwd.y, fwd.x);
        let nose = a.pos + fwd * scale * 1.6;
        let left = a.pos - fwd * scale * 0.7 + side * scale * 0.9;
        let right = a.pos - fwd * scale * 0.7 - side * scale * 0.9;
        gizmos.line_2d(nose, left, c);
        gizmos.line_2d(left, right, c);
        gizmos.line_2d(right, nose, c);

        if a.detected {
            gizmos.circle_2d(Isometry2d::from_translation(a.pos), 15.0 * px, Color::WHITE);
        }
        if ui_state.selected.contains(&Selected::Air(i)) {
            gizmos.circle_2d(
                Isometry2d::from_translation(a.pos),
                20.0 * px,
                Color::srgb(1.0, 0.9, 0.2),
            );
        }
    }

    for s in sim.sim.sensors() {
        // A carried sensor rides its airframe, which already has a marker.
        if s.carrier.is_some() {
            continue;
        }
        let c = side_color(s.side);
        gizmos.circle_2d(Isometry2d::from_translation(s.pos), 8.0 * px, c);
        gizmos.circle_2d(Isometry2d::from_translation(s.pos), 3.0 * px, c);
    }
    for u in sim.sim.units() {
        if !u.alive() {
            // Killed: a dim grey cross.
            let g = Color::srgb(0.4, 0.4, 0.4);
            let d = 8.0 * px;
            gizmos.line_2d(u.pos + Vec2::new(-d, -d), u.pos + Vec2::new(d, d), g);
            gizmos.line_2d(u.pos + Vec2::new(-d, d), u.pos + Vec2::new(d, -d), g);
            continue;
        }
        let c = side_color(u.side);
        gizmos.rect_2d(
            Isometry2d::new(u.pos, Rot2::degrees(45.0)),
            Vec2::splat(11.0 * px),
            c,
        );
        if u.detected {
            gizmos.circle_2d(Isometry2d::from_translation(u.pos), 13.0 * px, Color::WHITE);
        }
        // Suppression ring: amber = Suppressed, red = Pinned.
        match u.suppression {
            Suppression::Free => {}
            Suppression::Suppressed => {
                gizmos.circle_2d(
                    Isometry2d::from_translation(u.pos),
                    16.0 * px,
                    Color::srgb(0.95, 0.65, 0.1),
                );
            }
            Suppression::Pinned => {
                gizmos.circle_2d(
                    Isometry2d::from_translation(u.pos),
                    16.0 * px,
                    Color::srgb(0.95, 0.2, 0.15),
                );
            }
        }
        // A strength bar appears once the unit has lost an element.
        if u.strength() < 0.999 {
            let w = 24.0 * px;
            let y = u.pos.y - 16.0 * px;
            let left = u.pos.x - w / 2.0;
            gizmos.line_2d(
                Vec2::new(left, y),
                Vec2::new(left + w, y),
                Color::srgb(0.2, 0.2, 0.2),
            );
            gizmos.line_2d(
                Vec2::new(left, y),
                Vec2::new(left + w * u.strength(), y),
                Color::srgb(0.2, 0.9, 0.3),
            );
        }
    }
}

/// LOS probe line: observer → cursor (or demo target), coloured by the result.
fn draw_probe(
    sim: Res<SimRes>,
    mut probe: ResMut<Probe>,
    window: Query<&Window, With<PrimaryWindow>>,
    camera: Query<(&Camera, &GlobalTransform, &Projection), With<Camera2d>>,
    mut gizmos: Gizmos,
) {
    let Ok(window) = window.single() else { return };
    let Ok((cam, cam_tf, projection)) = camera.single() else {
        return;
    };
    let px = match projection {
        Projection::Orthographic(o) => o.scale,
        _ => 1.0,
    };

    let cursor_world = window
        .cursor_position()
        .and_then(|c| cam.viewport_to_world_2d(cam_tf, c).ok());

    let Some(obs) = probe.observer else { return };
    let Some(target) = cursor_world.or(probe.demo_target) else {
        return;
    };

    let r = los::line_of_sight(
        sim.sim.terrain(),
        obs,
        PROBE_HEIGHT_M,
        target,
        PROBE_HEIGHT_M,
    );

    let color = if !r.clear {
        Color::srgb(0.90, 0.20, 0.15)
    } else if r.transmittance < 0.5 {
        Color::srgb(0.95, 0.65, 0.10)
    } else {
        Color::srgb(0.15, 0.85, 0.25)
    };

    gizmos.line_2d(obs, target, color);
    gizmos.circle_2d(Isometry2d::from_translation(obs), 10.0 * px, Color::WHITE);
    gizmos.circle_2d(Isometry2d::from_translation(obs), 7.5 * px, color);
    if let Some(s) = r.blocked_at {
        let hit = obs + (target - obs).normalize_or_zero() * s;
        gizmos.circle_2d(
            Isometry2d::from_translation(hit),
            5.0 * px,
            Color::srgb(0.95, 0.2, 0.15),
        );
    }
    probe.last = Some(r);
}

/// In screenshot mode, compute the belief overlay once (a few frames in) so the capture
/// shows the Phase 8 partial-observability picture.
fn screenshot_belief(
    sim: Res<SimRes>,
    mut overlay: ResMut<Overlay>,
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut frame: Local<u32>,
    mut done: Local<bool>,
) {
    *frame += 1;
    if !*done && *frame == 12 {
        *done = true;
        rebuild_belief_overlay(&sim, 60.0, &mut overlay, &mut commands, &mut images);
    }
}

/// Save one screenshot of the primary window after a short warm-up, then stop.
fn capture_screenshot(mut commands: Commands, mut frame: Local<u32>) {
    *frame += 1;
    if *frame == 35 {
        if let Some(path) = std::env::var_os("FIRES_SIM_SCREENSHOT") {
            commands
                .spawn(Screenshot::primary_window())
                .observe(save_to_disk(std::path::PathBuf::from(path)));
        }
    }
}
