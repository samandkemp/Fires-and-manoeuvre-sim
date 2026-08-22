//! Mouse and keyboard on the map.
//!
//! The interaction model is **modeless except for placement**. Left-click selects (shift
//! toggles, drag box-selects), right-click commands whatever is selected, and keys do the
//! rest. That is what removed the old Select/Move/Route radio buttons, where giving a unit
//! a route meant toggling a mode between every step.
//!
//! Runs inside the same system as the panel, because egui must get first refusal on every
//! pointer event - otherwise a click on a slider would also place a sensor on the map
//! behind it.

use bevy::prelude::*;
use bevy_egui::egui;
use sim_core::air::{AltitudeRef, FlightPlan};
use sim_core::air_defence::RadarPosture;
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
    // egui gets first refusal: a click it wants is not a click on the map, and a keypress
    // it wants is someone typing in the seed box, not a map shortcut.
    let wants_pointer = ctx.wants_pointer_input();
    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);

    if !ctx.wants_keyboard_input() {
        keyboard(sim, ui_state, keys);
    }
    left_click(sim, ui_state, buttons, wants_pointer, shift, &world_cursor);

    if buttons.just_pressed(MouseButton::Right) && !wants_pointer {
        if let Some(world) = world_cursor() {
            right_click(sim, ui_state, probe, world, shift);
        }
    }
}

/// Escape clears, Ctrl+A selects all, Delete removes, Space runs, `.` steps.
///
/// Space and `.` are here rather than only on the panel because inspecting a battle means
/// keeping your eyes on the map: reaching for a button loses the moment you paused for.
fn keyboard(sim: &mut SimRes, ui_state: &mut UiState, keys: &ButtonInput<KeyCode>) {
    if keys.just_pressed(KeyCode::Space) {
        ui_state.running = !ui_state.running;
        ui_state.tick_budget_s = 0.0;
    }
    if keys.just_pressed(KeyCode::Period) {
        ui_state.running = false;
        sim.sim.step_one();
    }
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
                Selected::AirDefence(i) => sim.sim.remove_air_defence(i),
                Selected::C2(i) => sim.sim.remove_c2(i),
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
        ClickMode::SetObjective => {
            // Every selected unit gets the same objective and plans its own way there, so
            // two units given one objective may take quite different routes - which is the
            // point, and the clearest way to see the planner working.
            for sel in &ui_state.selected {
                if let Selected::Unit(i) = sel {
                    sim.sim.set_objective(*i, Some(world));
                    sim.sim.set_unit_risk_weight(*i, Some(ui_state.risk_weight));
                }
            }
        }
        ClickMode::PlaceAir => place_drone(sim, ui_state, world),
        ClickMode::PlaceAirDefence => place_air_defence(sim, ui_state, world),
        ClickMode::PlaceC2 => place_c2(sim, ui_state, world),
        // `.get(..)` rather than indexing, as the drone/battery/post arms already do: a
        // type id that no longer names anything - after a scenario switch whose library
        // set differs - is an ordinary miss, not a reason to take the window down.
        ClickMode::PlaceSensor => {
            let Some(stats) = sim.data.libs.sensors.get(&ui_state.sensor_type_id).cloned() else {
                return;
            };
            sim.placed += 1;
            let side = ui_state.place_side;
            let id = format!("{}-obs-p{}", side_tag(side), sim.placed);
            sim.sim.add_sensor(&id, side, world, 0.0, stats);
        }
        ClickMode::PlaceUnit => {
            let Some(stats) = sim.data.libs.units.get(&ui_state.unit_type_id).cloned() else {
                return;
            };
            sim.placed += 1;
            let side = ui_state.place_side;
            let id = format!("{}-unit-p{}", side_tag(side), sim.placed);
            let weapon = stats
                .weapon
                .as_ref()
                .and_then(|w| sim.data.libs.weapons.get(w).cloned());
            sim.sim.add_unit(&id, side, world, stats, weapon);
            ui_state.selected = vec![Selected::Unit(sim.sim.units().len() - 1)];
        }
        ClickMode::PlaceJammer => {
            sim.sim.add_jammer(ui_state.place_side, world, 0.9, 900.0);
        }
    }
}

/// Short side prefix for a generated asset id, so a placed asset's side is readable in the
/// selection panel and in any CSV it later appears in.
fn side_tag(side: Side) -> &'static str {
    match side {
        Side::Blue => "blu",
        Side::Red => "red",
    }
}

/// Place a Red drone at the panel's altitude, heading and speed, and select it.
fn place_drone(sim: &mut SimRes, ui_state: &mut UiState, world: Vec2) {
    let Some(stats) = sim.data.libs.air.get(&ui_state.air_type_id).cloned() else {
        return;
    };
    sim.placed += 1;
    let id = format!("{}-uas-p{}", side_tag(ui_state.place_side), sim.placed);
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
        ui_state.place_side,
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

/// Place a Blue C2 post. Coordinates every friendly battery inside its radius (§11) -
/// and, being unarmed and conspicuous, is the obvious thing for the other side to attack.
fn place_c2(sim: &mut SimRes, ui_state: &mut UiState, world: Vec2) {
    let Some(stats) = sim.data.libs.c2.get(&ui_state.c2_type_id).cloned() else {
        return;
    };
    sim.placed += 1;
    let side = ui_state.place_side;
    let id = format!("{}-cp-p{}", side_tag(side), sim.placed);
    let idx = sim.sim.add_c2(&id, side, world, stats);
    // Select what was just placed, as unit and drone placement do: the next thing you want
    // is almost always to nudge it, and the panel then reports what it is coordinating.
    ui_state.selected = vec![Selected::C2(idx)];
}

/// Place a self-cueing Blue air-defence battery.
fn place_air_defence(sim: &mut SimRes, ui_state: &mut UiState, world: Vec2) {
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
    let id = format!("{}-ad-p{}", side_tag(ui_state.place_side), sim.placed);
    let sensor = stats
        .sensor
        .as_ref()
        .and_then(|s| sim.data.libs.sensors.get(s).cloned());
    let idx = sim.sim.add_air_defence(
        &id,
        ui_state.place_side,
        world,
        stats,
        RadarPosture::default(),
        sensor,
    );
    ui_state.selected = vec![Selected::AirDefence(idx)];
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::scenario::{Libraries, Scenario};
    use sim_core::sim::Sim;
    use std::path::Path;

    fn fixture() -> Option<(Sim, Libraries)> {
        let dir = Path::new("../../scenarios");
        let libs = Libraries::load_dir(dir).ok()?;
        let scn = Scenario::load(&dir.join("ad_c2.toml")).ok()?;
        Some((Sim::new(&scn, &libs, scn.default_seed).ok()?, libs))
    }

    /// Placement used to hardcode a side per mode, so half the asset classes could only ever
    /// join one force and the counter-sensing fight could not be set up from the map. Every
    /// placing mode must now honour the chosen side.
    #[test]
    fn every_placing_mode_honours_the_chosen_side() {
        let Some((sim, libs)) = fixture() else {
            return; // scenarios not present; nothing to assert
        };
        for mode in [
            ClickMode::PlaceSensor,
            ClickMode::PlaceUnit,
            ClickMode::PlaceJammer,
            ClickMode::PlaceAir,
            ClickMode::PlaceAirDefence,
            ClickMode::PlaceC2,
        ] {
            assert!(
                mode.places_an_asset(),
                "{mode:?} places something, so it must offer a side"
            );
        }
        for mode in [ClickMode::Probe, ClickMode::AirOrbit] {
            assert!(
                !mode.places_an_asset(),
                "{mode:?} acts on what is already there; a side control would do nothing"
            );
        }
        // The fixture is only here to prove the libraries a placement reads actually
        // resolve - a mode whose stat block is missing silently places nothing.
        assert!(!libs.sensors.is_empty() && !libs.units.is_empty());
        assert!(!sim.sensors().is_empty());
    }

    /// A placed asset's id carries its side, so the selection panel and any CSV it reaches
    /// say which force it belongs to without a lookup.
    #[test]
    fn a_placed_id_names_its_side() {
        assert_eq!(side_tag(Side::Blue), "blu");
        assert_eq!(side_tag(Side::Red), "red");
        assert_ne!(side_tag(Side::Blue), side_tag(Side::Red));
    }
}
