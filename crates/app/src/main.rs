//! Bevy front-end: renders the tactical map and drives the `sim_core` simulation.
//! Phase 2: the sensor-placement duel — place Blue sensors and Red units, run the
//! clock, watch mutual detection unfold. The app reads sim state and issues placements;
//! all modelling lives in `sim_core`.

use bevy::prelude::*;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};
use bevy::window::PrimaryWindow;
use bevy_egui::{egui, EguiContexts, EguiPlugin, EguiPrimaryContextPass};
use bevy_pancam::{PanCam, PanCamPlugin};
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

/// What a right-click on the map does.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ClickMode {
    /// Set the LOS-probe observer.
    Probe,
    /// Select the nearest unit (to move, route, or inspect it).
    Select,
    /// Place a Blue sensor of the selected type.
    PlaceBlueSensor,
    /// Place a Red unit of the selected type.
    PlaceRedUnit,
    /// Place a Red jammer (EW bubble that hides nearby Red units).
    PlaceRedJammer,
    /// Append route waypoints to the currently selected unit.
    RouteSelected,
    /// Move the currently selected unit to the click (clearing its route).
    MoveSelected,
}

/// A reset requested from the panel, applied after the egui closure releases the sim.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ResetKind {
    None,
    /// Rebuild the sim fresh from the default scenario.
    Scenario,
    /// Clear all placed assets, keeping the terrain.
    Clear,
}

/// Panel state: interaction mode, selected types, clock control.
#[derive(Resource)]
struct UiState {
    mode: ClickMode,
    sensor_type_id: String,
    unit_type_id: String,
    running: bool,
    ticks_per_frame: u32,
    /// The currently selected unit (for move/route/inspect), if any.
    selected: Option<usize>,
    /// Exposure window (s) for the Pd coverage overlay — live-tweakable.
    coverage_exposure_s: f32,
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
        .add_systems(Startup, setup)
        .add_systems(Update, (advance_sim, draw_markers, draw_probe))
        .add_systems(EguiPrimaryContextPass, ui_panel);

    // Opt-in framebuffer capture: FIRES_SIM_SCREENSHOT=<path.png> saves one shot a few
    // frames in (and pre-runs the sim briefly so detections are visible).
    if std::env::var_os("FIRES_SIM_SCREENSHOT").is_some() {
        app.add_systems(Update, (capture_screenshot, screenshot_belief));
    }

    app.run();
}

fn setup(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let data = terrain_view::load_default().expect("default scenario should load");
    let mut sim = Sim::new(
        &data.scenario,
        &data.terrain_params,
        &data.sensor_types,
        &data.unit_types,
        &data.weapon_types,
        data.scenario.default_seed,
    )
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
    ));

    // Whole map in frame at startup; pancam takes over (left/middle drag — right-click
    // is the action button).
    commands.spawn((
        Camera2d,
        Projection::Orthographic(OrthographicProjection {
            scale: (extent_x / 1200.0).max(extent_y / 680.0).max(1.0),
            ..OrthographicProjection::default_2d()
        }),
        Transform::from_translation(center),
        PanCam {
            grab_buttons: vec![MouseButton::Left, MouseButton::Middle],
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
        sensor_type_id: data.sensor_types.keys().next().cloned().unwrap_or_default(),
        unit_type_id: data.unit_types.keys().next().cloned().unwrap_or_default(),
        running: screenshot_mode,
        // In screenshot mode, one tick/frame so the capture lands mid-bombardment
        // (suppression visible on the target) rather than in the aftermath.
        ticks_per_frame: 1,
        selected: None,
        coverage_exposure_s: COVERAGE_EXPOSURE_S,
    });
    commands.insert_resource(SimRes {
        sim,
        data,
        placed: 0,
    });
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
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    buttons: Res<ButtonInput<MouseButton>>,
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

            ui.separator();
            ui.label("Right-click on map:");
            ui.radio_value(&mut ui_state.mode, ClickMode::Select, "Select unit");
            ui.radio_value(
                &mut ui_state.mode,
                ClickMode::MoveSelected,
                "Move selected unit",
            );
            ui.radio_value(
                &mut ui_state.mode,
                ClickMode::RouteSelected,
                "Route selected (click waypoints)",
            );
            ui.radio_value(&mut ui_state.mode, ClickMode::Probe, "LOS probe observer");
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

            // Selected-unit readout.
            if let Some(idx) = ui_state.selected {
                if let Some(u) = sim.sim.units().get(idx).filter(|u| u.alive()) {
                    ui.label(format!(
                        "Selected: {} ({:?})  {}/{} elem  {:?}",
                        u.id, u.side, u.elements, u.initial_elements, u.suppression
                    ));
                } else {
                    ui_state.selected = None; // stale (cleared/killed)
                }
            }

            egui::ComboBox::from_label("sensor type")
                .selected_text(&ui_state.sensor_type_id)
                .show_ui(ui, |ui| {
                    for key in sim.data.sensor_types.keys() {
                        ui.selectable_value(&mut ui_state.sensor_type_id, key.clone(), key);
                    }
                });
            egui::ComboBox::from_label("unit type")
                .selected_text(&ui_state.unit_type_id)
                .show_ui(ui, |ui| {
                    for key in sim.data.unit_types.keys() {
                        ui.selectable_value(&mut ui_state.unit_type_id, key.clone(), key);
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

            ui.separator();
            ui.collapsing("Legend", |ui| {
                ui.small("○ sensor   ◇ unit   ✕ destroyed");
                ui.small("blue = friendly, red = enemy");
                ui.small("white ring = detected");
                ui.small("amber ring = suppressed, red ring = pinned");
                ui.small("green bar = remaining strength");
                ui.small("faint line = movement route");
                ui.small("yellow ring = selected unit");
                ui.small("magenta bubble = EW jammer");
            });
        });

    // Apply a deferred reset (the sim can't be rebuilt while the egui closure borrows it).
    match reset_pending {
        ResetKind::Scenario => {
            let d = &sim.data;
            let fresh = Sim::new(
                &d.scenario,
                &d.terrain_params,
                &d.sensor_types,
                &d.unit_types,
                &d.weapon_types,
                d.scenario.default_seed,
            )
            .expect("default scenario resolves");
            sim.sim = fresh;
            sim.placed = 0;
            ui_state.selected = None;
            ui_state.running = false;
            probe.observer = None;
            if let Some(e) = overlay.0.take() {
                commands.entity(e).despawn();
            }
        }
        ResetKind::Clear => {
            sim.sim.reset(0);
            sim.placed = 0;
            ui_state.selected = None;
            if let Some(e) = overlay.0.take() {
                commands.entity(e).despawn();
            }
        }
        ResetKind::None => {}
    }

    // Map actions: right-click, unless egui wants the pointer.
    if buttons.just_pressed(MouseButton::Right) && !ctx.wants_pointer_input() {
        let window = window.single()?;
        let (cam, cam_tf) = camera.single()?;
        if let Some(world) = window
            .cursor_position()
            .and_then(|c| cam.viewport_to_world_2d(cam_tf, c).ok())
        {
            match ui_state.mode {
                ClickMode::Probe => probe.observer = Some(world),
                ClickMode::Select => ui_state.selected = sim.sim.nearest_unit(world, 400.0),
                ClickMode::MoveSelected => {
                    if let Some(idx) = ui_state.selected {
                        sim.sim.set_unit_pos(idx, world);
                    }
                }
                ClickMode::RouteSelected => {
                    if let Some(idx) = ui_state.selected {
                        sim.sim.push_waypoint(idx, world);
                    }
                }
                ClickMode::PlaceBlueSensor => {
                    sim.placed += 1;
                    let id = format!("obs-p{}", sim.placed);
                    let stats = sim.data.sensor_types[&ui_state.sensor_type_id].clone();
                    sim.sim.add_sensor(&id, Side::Blue, world, 0.0, stats);
                }
                ClickMode::PlaceRedUnit => {
                    sim.placed += 1;
                    let id = format!("tgt-p{}", sim.placed);
                    let stats = sim.data.unit_types[&ui_state.unit_type_id].clone();
                    let weapon = stats
                        .weapon
                        .as_ref()
                        .and_then(|w| sim.data.weapon_types.get(w).cloned());
                    sim.sim.add_unit(&id, Side::Red, world, stats, weapon);
                    // Auto-select the new unit so it can be routed/moved immediately.
                    ui_state.selected = Some(sim.sim.units().len() - 1);
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
    let Some(sensor) = sim
        .sim
        .sensors()
        .iter()
        .rev()
        .find(|s| s.side == Side::Blue)
    else {
        return;
    };
    let reference = sim
        .data
        .unit_types
        .get("afv")
        .or_else(|| sim.data.unit_types.values().next())
        .expect("at least one unit type");

    let terrain = sim.sim.terrain();
    let t0 = std::time::Instant::now();
    let (h, w) = (terrain.height(), terrain.width());
    let mut pd = ndarray::Array2::<f32>::zeros((h, w));
    ndarray::Zip::indexed(&mut pd).par_for_each(|(iy, ix), v| {
        let target = terrain.transform().cell_center(ix, iy);
        let lambda = sensing::detection_rate(
            terrain,
            &sensor.stats,
            sensor.pos,
            sensor.facing_deg,
            reference,
            target,
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
    let blue: Vec<_> = sim
        .sim
        .sensors()
        .iter()
        .filter(|s| s.side == Side::Blue)
        .collect();
    if blue.is_empty() {
        return;
    }
    let reference = sim
        .data
        .unit_types
        .get("afv")
        .or_else(|| sim.data.unit_types.values().next())
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
        let mut p = 1.0f32;
        for s in &blue {
            let lambda =
                sensing::detection_rate(terrain, &s.stats, s.pos, s.facing_deg, reference, pos)
                    * sim_core::ew::jamming_factor(pos, &red_jammers);
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

    // Selection highlight: a yellow ring around the selected unit.
    if let Some(idx) = ui_state.selected {
        if let Some(u) = sim.sim.units().get(idx).filter(|u| u.alive()) {
            gizmos.circle_2d(
                Isometry2d::from_translation(u.pos),
                18.0 * px,
                Color::srgb(1.0, 0.9, 0.2),
            );
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

    for s in sim.sim.sensors() {
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
