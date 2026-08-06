//! Bevy front-end: renders the tactical map and drives the `sim_core` simulation.
//! Place sensors, units, jammers, drones and air-defence batteries; run the clock; watch
//! mutual detection, fires and the counter-air fight unfold. The app reads sim state and
//! issues placements; all modelling lives in `sim_core`.
//!
//! # Where things live
//!
//! | Module | What it does |
//! |---|---|
//! | this file | app wiring, startup, the clock, scenario switching |
//! | [`state`] | the resources and shared types |
//! | [`ui`] | the control panel, one method per section |
//! | [`input`] | mouse and keyboard on the map |
//! | [`selection`] | what is selected, and commanding it |
//! | [`overlays`] | the coverage and belief rasters |
//! | [`markers`] | force markers, routes, envelopes, the LOS probe |
//! | [`terrain_view`] | turning terrain into a texture |

use bevy::prelude::*;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};
use bevy_egui::{egui, EguiContexts, EguiPlugin, EguiPrimaryContextPass};
use bevy_pancam::{PanCam, PanCamPlugin};
use sim_core::sim::{Side, Sim};

mod input;
mod markers;
mod overlays;
mod selection;
mod state;
mod terrain_view;
mod ui;

use state::{
    CameraFrameQuery, CameraQuery, ClickMode, MapSprite, MapSpriteQuery, Overlay, PendingLoad,
    Probe, ResetKind, SimRes, UiState, WindowQuery, COVERAGE_EXPOSURE_S,
};

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
            (
                advance_sim,
                markers::draw_markers,
                markers::draw_probe,
                apply_scenario_load,
            ),
        )
        .add_systems(EguiPrimaryContextPass, ui_panel);

    // Opt-in framebuffer capture: FIRES_SIM_SCREENSHOT=<path.png> saves one shot a few
    // frames in (and pre-runs the sim briefly so detections are visible).
    if std::env::var_os("FIRES_SIM_SCREENSHOT").is_some() {
        app.add_systems(Update, (capture_screenshot, overlays::screenshot_belief));
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
        c2_type_id: data.libs.c2.keys().next().cloned().unwrap_or_default(),
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

/// Advance the sim clock: `ticks_per_frame` ticks of `dt_s` per rendered frame while
/// running (1 tick/frame ≈ 60× real time at 60 fps).
fn advance_sim(mut sim: ResMut<SimRes>, ui: Res<UiState>) {
    if ui.running {
        for _ in 0..ui.ticks_per_frame {
            sim.sim.step_one();
        }
    }
}

/// The control panel and the map's click handling, in that order.
///
/// They share a system because egui must claim pointer events **first**: a click on a
/// slider must not also drop a sensor on the map behind it. Splitting them into two
/// systems would put that ordering at the mercy of the schedule.
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
    window: WindowQuery,
    camera: CameraQuery,
) -> Result {
    let ctx = contexts.ctx_mut()?.clone();

    let mut panel = ui::Panel {
        sim: &mut sim,
        ui_state: &mut ui_state,
        probe: &probe,
        overlay: &mut overlay,
        commands: &mut commands,
        images: &mut images,
        reset: ResetKind::None,
    };
    egui::SidePanel::left("controls")
        .min_width(230.0)
        .show(&ctx, |ui| panel.show(ui));
    let reset = panel.reset;

    ui::apply_reset(
        reset,
        &mut sim,
        &mut ui_state,
        &mut probe,
        &mut overlay,
        &mut pending_load,
        &mut commands,
    );

    input::handle_map(
        &ctx,
        &mut sim,
        &mut ui_state,
        &mut probe,
        &buttons,
        &keys,
        &window,
        &camera,
    );
    Ok(())
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
