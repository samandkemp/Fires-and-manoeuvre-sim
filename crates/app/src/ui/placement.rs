//! Placing assets, choosing what is selected, and the type pickers those two share.
//!
//! An inherent `impl` block may live in any module of the crate that defines the
//! type, so these sections keep their `&mut self` signatures and the calls in
//! `Panel::show` are unchanged.

use bevy::prelude::*;
use bevy_egui::egui;
use sim_core::sim::Side;

use crate::state::{ClickMode, Selected};

use super::Panel;

impl Panel<'_, '_, '_> {
    /// What a right-click places. The only remaining mode in the UI.
    pub(super) fn placement_modes(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.label("Right-click places:");
        let mode = &mut self.ui_state.mode;
        for (value, label) in [
            (ClickMode::Probe, "nothing (move/route)"),
            (ClickMode::PlaceSensor, "Sensor"),
            (ClickMode::PlaceUnit, "Unit"),
            (ClickMode::PlaceJammer, "Jammer (EW)"),
            (ClickMode::PlaceAir, "Drone"),
            (ClickMode::PlaceAirDefence, "Air defence"),
            (ClickMode::PlaceC2, "C2 post"),
            (ClickMode::AirOrbit, "Drone orbit here (radius below)"),
            (ClickMode::SetObjective, "Objective for selected unit(s)"),
        ] {
            ui.radio_value(mode, value, label);
        }

        // The exchange rate the objective is planned against (§5.1): how many metres of
        // movement cost this unit will spend to avoid one unit of exposure. At 0 it takes
        // the short way regardless of who is watching, which is the comparison worth making.
        if self.ui_state.mode == ClickMode::SetObjective {
            ui.add(
                egui::Slider::new(&mut self.ui_state.risk_weight, 0.0..=800.0)
                    .text("caution (risk weight)"),
            );
            ui.label("0 = shortest route; higher trades distance for cover");
        }

        // Which force the asset joins. Shown only for modes that place something, so the
        // control is never present while doing nothing. Both sides can field every asset
        // class, which is what makes the counter-sensing fight - positioning to see without
        // being seen - something the map can express rather than only a scenario file.
        if self.ui_state.mode.places_an_asset() {
            ui.horizontal(|ui| {
                ui.label("for side:");
                let side = &mut self.ui_state.place_side;
                ui.selectable_value(side, Side::Blue, "Blue");
                ui.selectable_value(side, Side::Red, "Red");
            });
        }
    }

    /// What is selected - after dropping anything that died or was cleared under us, so a
    /// stale index can never be commanded.
    pub(super) fn selection_readout(&mut self, ui: &mut egui::Ui) {
        let sim_ref = &self.sim.sim;
        self.ui_state.selected.retain(|sel| match sel {
            Selected::Unit(i) => sim_ref.units().get(*i).is_some_and(|u| u.alive()),
            Selected::Air(i) => sim_ref.air().get(*i).is_some_and(|a| a.alive),
            Selected::AirDefence(i) => sim_ref.air_defence().get(*i).is_some_and(|d| d.alive()),
            Selected::C2(i) => sim_ref.c2().get(*i).is_some_and(|c| c.alive()),
        });
        match self.ui_state.selected.len() {
            0 => {
                ui.label("Nothing selected");
            }
            1 => match self.ui_state.selected[0] {
                Selected::Unit(i) => {
                    let u = &self.sim.sim.units()[i];
                    ui.label(format!(
                        "Selected: {} ({:?})  {}/{} elem  {:?}",
                        u.id, u.side, u.elements, u.initial_elements, u.suppression
                    ));
                }
                Selected::Air(i) => {
                    let a = &self.sim.sim.air()[i];
                    ui.label(format!("Selected: {} (drone, {:?})", a.id, a.side));
                }
                Selected::AirDefence(i) => {
                    let d = &self.sim.sim.air_defence()[i];
                    let rounds = if d.magazine_left == u32::MAX {
                        "unlimited".to_owned()
                    } else {
                        format!("{} rounds", d.magazine_left)
                    };
                    ui.label(format!(
                        "Selected: {} (air defence, {:?})  {}/{} up  {rounds}",
                        d.id,
                        d.side,
                        d.elements,
                        d.stats.element_count.max(1)
                    ));
                }
                Selected::C2(i) => {
                    let c = &self.sim.sim.c2()[i];
                    // Say how many batteries it is actually holding together: that number
                    // is the post's whole reason to exist, and it changes as things move.
                    let covered = self
                        .sim
                        .sim
                        .air_defence()
                        .iter()
                        .filter(|d| d.side == c.side && d.alive() && c.covers(d.pos))
                        .count();
                    ui.label(format!(
                        "Selected: {} (C2 post, {:?})  coordinating {covered}",
                        c.id, c.side
                    ));
                }
            },
            n => {
                let mut counts = [0_usize; 4];
                for s in &self.ui_state.selected {
                    counts[match s {
                        Selected::Unit(_) => 0,
                        Selected::Air(_) => 1,
                        Selected::AirDefence(_) => 2,
                        Selected::C2(_) => 3,
                    }] += 1;
                }
                let parts: Vec<String> = ["ground", "air", "AD", "C2"]
                    .iter()
                    .zip(counts)
                    .filter(|(_, c)| *c > 0)
                    .map(|(name, c)| format!("{c} {name}"))
                    .collect();
                ui.label(format!("Selected: {n} assets ({})", parts.join(", ")));
            }
        }
    }

    /// Which sensor and unit type the next placement uses.
    pub(super) fn type_pickers(&mut self, ui: &mut egui::Ui) {
        let sensors = &self.sim.data.libs.sensors;
        let chosen = &mut self.ui_state.sensor_type_id;
        egui::ComboBox::from_label("sensor type")
            .selected_text(chosen.clone())
            .show_ui(ui, |ui| {
                for key in sensors.keys() {
                    ui.selectable_value(chosen, key.clone(), key);
                }
            });
        let units = &self.sim.data.libs.units;
        let chosen = &mut self.ui_state.unit_type_id;
        egui::ComboBox::from_label("unit type")
            .selected_text(chosen.clone())
            .show_ui(ui, |ui| {
                for key in units.keys() {
                    ui.selectable_value(chosen, key.clone(), key);
                }
            });
    }
}
