//! Mouse and keyboard on the map.
//!
//! The interaction model is **modeless except for placement**. Left-click selects (shift
//! toggles, drag box-selects), right-click commands whatever is selected, and keys do the
//! rest. That is what removed the old Select/Move/Route radio buttons, where giving a unit
//! a route meant toggling a mode between every step.
//!
//! Runs inside the same system as the panel, because egui must get first refusal on every
//! pointer event — otherwise a click on a slider would also place a sensor on the map
//! behind it.

use bevy::prelude::*;
use bevy_egui::egui;
use sim_core::air::{AltitudeRef, FlightPlan};
use sim_core::sim::Side;

use crate::selection::{
    all_live_assets, append_waypoint, assets_in_box, move_selection, nearest_asset,
    BOX_SELECT_MIN_M, PICK_RADIUS_M,
};
use crate::state::{CameraQuery, ClickMode, Probe, Selected, SimRes, UiState, WindowQuery};

/// Handle one frame of map input.
// egui 0.34 renamed `wants_pointer_input` to `egui_wants_pointer_input`; the old name
// still works. Adopt the new one when the UI is next reworked (see `main::ui_panel`).
#[allow(deprecated)]
#[allow(clippy::too_many_arguments)]
pub fn handle_map(
    ctx: &egui::Context,
    sim: &mut SimRes,
    ui_state: &mut UiState,
    probe: &mut Probe,
    buttons: &ButtonInput<MouseButton>,
    keys: &ButtonInput<KeyCode>,
    window: &WindowQuery,
    camera: &CameraQuery,
) {
    let world_cursor = || -> Option<Vec2> {
        let window = window.single().ok()?;
        let (cam, cam_tf) = camera.single().ok()?;
        window
            .cursor_position()
            .and_then(|c| cam.viewport_to_world_2d(cam_tf, c).ok())
    };
    // egui gets first refusal: a click it wants is not a click on the map.
    let wants_pointer = ctx.wants_pointer_input();
    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);

    keyboard(sim, ui_state, keys);
    left_click(sim, ui_state, buttons, wants_pointer, shift, &world_cursor);

    if buttons.just_pressed(MouseButton::Right) && !wants_pointer {
        if let Some(world) = world_cursor() {
            right_click(sim, ui_state, probe, world, shift);
        }
    }
}

/// Escape clears, Ctrl+A selects all, Delete removes.
fn keyboard(sim: &mut SimRes, ui_state: &mut UiState, keys: &ButtonInput<KeyCode>) {
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
}

/// A left press starts a possible box-select; the release decides click versus box.
fn left_click(
    sim: &SimRes,
    ui_state: &mut UiState,
    buttons: &ButtonInput<MouseButton>,
    wants_pointer: bool,
    shift: bool,
    world_cursor: &impl Fn() -> Option<Vec2>,
) {
    if buttons.just_pressed(MouseButton::Left) && !wants_pointer {
        ui_state.drag_start = world_cursor();
    }
    if !buttons.just_released(MouseButton::Left) {
        return;
    }
    let (Some(start), Some(end)) = (ui_state.drag_start.take(), world_cursor()) else {
        return;
    };
    if wants_pointer {
        return;
    }
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

/// Right-click either commands the selection or places an asset, depending on the mode.
fn right_click(
    sim: &mut SimRes,
    ui_state: &mut UiState,
    probe: &mut Probe,
    world: Vec2,
    shift: bool,
) {
    match ui_state.mode {
        ClickMode::Probe => {
            // No placement mode set: right-click commands the selection. With nothing
            // selected it falls back to placing the LOS probe.
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
        ClickMode::PlaceRedAir => place_drone(sim, ui_state, world),
        ClickMode::PlaceBlueAirDefence => place_air_defence(sim, ui_state, world),
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

/// Place a Red drone at the panel's altitude, heading and speed, and select it.
fn place_drone(sim: &mut SimRes, ui_state: &mut UiState, world: Vec2) {
    let Some(stats) = sim.data.libs.air.get(&ui_state.air_type_id).cloned() else {
        return;
    };
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

/// Place a self-cueing Blue air-defence battery.
fn place_air_defence(sim: &mut SimRes, ui_state: &UiState, world: Vec2) {
    let Some(stats) = sim
        .data
        .libs
        .air_defence
        .get(&ui_state.air_defence_type_id)
        .cloned()
    else {
        return;
    };
    sim.placed += 1;
    let id = format!("ad-p{}", sim.placed);
    let sensor = stats
        .sensor
        .as_ref()
        .and_then(|s| sim.data.libs.sensors.get(s).cloned());
    sim.sim
        .add_air_defence(&id, Side::Blue, world, stats, true, sensor);
}
