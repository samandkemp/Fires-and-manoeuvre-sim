//! Run, pause, step, speed and breakpoints - the controls that decide what is on screen
//! when something interesting happens.
//!
//! An inherent `impl` block may live in any module of the crate that defines the
//! type, so these sections keep their `&mut self` signatures and the calls in
//! `Panel::show` are unchanged.

use bevy::prelude::*;
use bevy_egui::egui;

use crate::resources::ResetKind;

use super::Panel;

impl Panel<'_, '_, '_> {
    /// Run/pause, manual stepping, playback speed, breakpoints, and the two resets.
    ///
    /// Speed is in **sim seconds per real second**, not ticks per frame. A battle is over
    /// in a few hundred seconds of sim time, so at one tick per rendered frame - the old
    /// control - everything interesting happened while you were still reading the panel.
    /// Below 1× the same run just takes longer to watch; the event log is identical
    /// either way (`main::advance_sim`).
    pub(super) fn clock(&mut self, ui: &mut egui::Ui) {
        let dt = self.sim.sim.dt_s();
        let epoch = self.sim.sim.epoch_s();
        ui.separator();
        ui.label("Clock");
        ui.horizontal(|ui| {
            if ui
                .button(if self.ui_state.running {
                    "Pause"
                } else {
                    "Run"
                })
                .clicked()
            {
                self.ui_state.running = !self.ui_state.running;
                self.ui_state.tick_budget_s = 0.0;
            }
            // Step by the two units the model actually has: the integration tick, and the
            // decision epoch where fires and allocation resolve.
            if ui
                .button(format!("+{dt:.0} s"))
                .on_hover_text("One integration tick")
                .clicked()
            {
                self.ui_state.running = false;
                self.sim.sim.step_one();
            }
            if ui
                .button(format!("+{epoch:.0} s"))
                .on_hover_text("One decision epoch: fires and allocation resolve")
                .clicked()
            {
                self.ui_state.running = false;
                let until = self.sim.sim.time_s() + f64::from(epoch);
                self.sim.sim.run_until(until);
            }
        });

        ui.add(
            egui::Slider::new(&mut self.ui_state.speed_x, 0.1..=120.0)
                .logarithmic(true)
                .suffix("x")
                .text("speed"),
        );
        ui.horizontal(|ui| {
            ui.label("preset");
            for (label, x) in [("0.2", 0.2), ("1", 1.0), ("10", 10.0), ("60", 60.0)] {
                let on = (self.ui_state.speed_x - x).abs() < 1.0e-3;
                if ui.selectable_label(on, label).clicked() {
                    self.ui_state.speed_x = x;
                }
            }
        });

        ui.horizontal(|ui| {
            ui.add(
                egui::DragValue::new(&mut self.ui_state.run_to_s)
                    .speed(10.0)
                    .range(0.0..=100_000.0)
                    .prefix("t = "),
            );
            // Headless-fast: this is `run_until`, the same call the batch harness makes,
            // so jumping ahead costs no more than the sim itself.
            if ui.button("Run to").clicked() {
                self.ui_state.running = false;
                self.sim.sim.run_until(f64::from(self.ui_state.run_to_s));
            }
        });

        ui.horizontal(|ui| {
            ui.label("pause on");
            ui.checkbox(&mut self.ui_state.breakpoints.detection, "contact")
                .on_hover_text("Any new detection, ground or air, either side");
            ui.checkbox(&mut self.ui_state.breakpoints.casualty, "loss")
                .on_hover_text("Any ground sub-element destroyed");
            ui.checkbox(&mut self.ui_state.breakpoints.air_action, "air")
                .on_hover_text("Any air-defence shot or munition release");
        });

        ui.horizontal(|ui| {
            if ui.button("Reset scenario").clicked() {
                self.reset = ResetKind::Scenario;
            }
            if ui.button("Clear all").clicked() {
                self.reset = ResetKind::Clear;
            }
        });
        ui.horizontal(|ui| {
            ui.add(
                egui::DragValue::new(&mut self.ui_state.seed)
                    .speed(1.0)
                    .prefix("seed "),
            );
            if ui.button("Re-run at seed").clicked() {
                self.reset = ResetKind::Reseed;
            }
        });
    }

    /// Every `*.toml` in `scenarios/` that parses as a scenario, switchable live.
    pub(super) fn scenario_picker(&mut self, ui: &mut egui::Ui) {
        let current = self.sim.data.scenario_name.clone();
        // Bind the two fields separately: the closure needs `available` immutably and
        // `reset` mutably, which is only allowed as disjoint borrows, not through `self`.
        let available = &self.sim.data.available;
        let reset = &mut self.reset;
        egui::ComboBox::from_label("scenario")
            .selected_text(&current)
            .show_ui(ui, |ui| {
                for name in available {
                    if ui.selectable_label(*name == current, name).clicked() && *name != current {
                        *reset = ResetKind::Load(name.clone());
                    }
                }
            });
    }

    /// The mouse and keyboard cheat sheet.
    pub(super) fn controls_help(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.collapsing("Controls", |ui| {
            ui.small("Left-click select \u{b7} shift add \u{b7} drag box-select");
            ui.small("Selects units, drones, AD batteries and C2 posts");
            ui.small("Right-click move here \u{b7} shift append waypoint");
            ui.small("Ctrl+A select all \u{b7} Del remove \u{b7} Esc clear");
            ui.small("Middle-drag pan \u{b7} scroll zoom");
            ui.small("Space run/pause \u{b7} . step one tick");
        });
    }
}
