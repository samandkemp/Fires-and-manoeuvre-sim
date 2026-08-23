//! The control panel, one method per section.
//!
//! [`Panel::show`] reads as a table of contents for the whole UI; each section below is
//! small enough to hold in your head. Everything the panel needs is bundled into
//! [`Panel`] rather than passed as a dozen arguments.
//!
//! **Resets are deferred.** A button that rebuilds the sim cannot run while the egui
//! closure still borrows it, so the sections only *record* what was asked for in
//! [`Panel::reset`], and [`apply_reset`] carries it out afterwards.

use bevy::prelude::*;
use bevy_egui::egui;
use sim_core::sim::Sim;

use crate::overlays;
use crate::state::{Overlay, PendingLoad, Probe, ResetKind, SimRes, UiState};

mod air;
mod clock;
mod decisions;
mod dials;
mod overlay_panel;
mod placement;
mod readouts;

/// Everything the panel draws from, gathered so each section takes only `&mut self`.
pub struct Panel<'a, 'w, 's> {
    pub sim: &'a mut SimRes,
    pub ui_state: &'a mut UiState,
    pub probe: &'a Probe,
    pub overlay: &'a mut Overlay,
    pub commands: &'a mut Commands<'w, 's>,
    /// Set by a section, carried out by [`apply_reset`] once egui lets go of the sim.
    pub reset: ResetKind,
}

impl Panel<'_, '_, '_> {
    /// Draw the whole panel, top to bottom.
    pub fn show(&mut self, ui: &mut egui::Ui) {
        ui.heading("Fires & Manoeuvre Sim");
        ui.label(format!("t = {:.0} s", self.sim.sim.time_s()));

        self.clock(ui);
        self.scenario_picker(ui);
        self.controls_help(ui);
        self.placement_modes(ui);
        self.selection_readout(ui);
        self.type_pickers(ui);
        self.air_section(ui);
        self.decision_section(ui);
        self.model_dials(ui);
        self.overlay_buttons(ui);
        self.probe_readout(ui);
        self.force_summary(ui);
        self.event_feed(ui);
        self.air_feed(ui);
        self.legend(ui);
    }
}

/// Carry out a reset the panel asked for.
///
/// Deferred out of the panel because rebuilding the sim needs mutable access that the
/// egui closure still holds while it draws.
pub fn apply_reset(
    reset: ResetKind,
    sim: &mut SimRes,
    ui_state: &mut UiState,
    probe: &mut Probe,
    overlay: &mut Overlay,
    pending_load: &mut PendingLoad,
    commands: &mut Commands,
) {
    // Every reset drops the overlay: it was computed against the old world. The cached
    // terrain goes too, because a scenario switch is the one thing that changes it - and a
    // background task holding the old map would paint the new one with the wrong raster.
    let drop_overlay = |overlay: &mut Overlay, commands: &mut Commands| {
        overlays::clear_overlay(overlay, commands);
        overlay.terrain = None;
    };
    match reset {
        ResetKind::None => return,
        ResetKind::Scenario => {
            let d = &sim.data;
            sim.sim = Sim::new(&d.scenario, &d.libs, d.scenario.default_seed)
                .expect("default scenario resolves");
            ui_state.running = false;
            probe.observer = None;
        }
        ResetKind::Clear => {
            sim.sim.reset(0);
        }
        ResetKind::Reseed => {
            // Terrain is kept: only the stochastic stream changes, the same separation
            // the batch runner makes.
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
            ui_state.running = false;
        }
        ResetKind::Load(name) => {
            pending_load.0 = Some(name);
            drop_overlay(overlay, commands);
            return; // the rest is `apply_scenario_load`'s job
        }
    }
    sim.placed = 0;
    ui_state.selected.clear();
    drop_overlay(overlay, commands);
}
