//! The model dials, editable while the simulation runs.
//!
//! Watching a rule change a battle is what a sweep cannot give you: a sweep reports what a
//! dial is worth on average over hundreds of trials and says nothing about what it *does*.
//! A change here takes effect on the next tick.
//!
//! Three dials are **read-only**. `dt_s` and `epoch_s` decide what a tick and an epoch mean,
//! so changing them mid-run would make the two halves of a trial answer different questions;
//! `belief_cells` sizes rasters that would have to be rebuilt and re-keyed. All three are
//! scenario-level, and are shown greyed rather than hidden.

use bevy::prelude::*;
use bevy_egui::egui;
use sim_core::doctrine::DoctrineMode;
use sim_core::sim::Side;

use super::Panel;
use crate::resources::SimRes;

impl Panel<'_, '_, '_> {
    /// Every live model dial, grouped by the subsystem it belongs to.
    pub(crate) fn model_dials(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Model dials")
            .default_open(false)
            .show(ui, |ui| {
                suppression(ui, self.sim);
                ui.separator();
                tracks(ui, self.sim);
                ui.separator();
                movement(ui, self.sim);
                ui.separator();
                doctrine(ui, self.sim);
                ui.separator();
                fixed(ui, self.sim);
            });
    }
}

/// Suppression (`docs/THEORY.md` §4.3): what a near miss does and how fast it wears off.
fn suppression(ui: &mut egui::Ui, sim: &mut SimRes) {
    ui.label(egui::RichText::new("Suppression").strong());
    let (mut p, mut recover, mut radius, mut factor) = sim.sim.suppression_dials();
    if ui
        .add(egui::Slider::new(&mut p, 0.0..=1.0).text("p(suppress) per near miss"))
        .changed()
    {
        sim.sim.set_p_suppress(p);
    }
    if ui
        .add(egui::Slider::new(&mut recover, 0.0..=1.0).text("recovery per s"))
        .changed()
    {
        sim.sim.set_recover_per_s(recover);
    }
    if ui
        .add(egui::Slider::new(&mut radius, 0.0..=200.0).text("near-miss radius m"))
        .changed()
    {
        sim.sim.set_suppression_radius_m(radius);
    }
    if ui
        .add(egui::Slider::new(&mut factor, 0.0..=1.0).text("suppressed fire factor"))
        .changed()
    {
        sim.sim.set_suppressed_fire_factor(factor);
    }
}

/// The track lifecycle (§10.1) - the dials that decide whether jamming can *break* a track
/// rather than only prevent one.
fn tracks(ui: &mut egui::Ui, sim: &mut SimRes) {
    ui.label(egui::RichText::new("Tracks").strong());
    let (mut hold, mut maintain) = sim.sim.track_dials();
    if ui
        .add(egui::Slider::new(&mut hold, 0.0..=180.0).text("track hold s"))
        .changed()
    {
        sim.sim.set_track_hold_s(hold);
    }
    if ui
        .add(egui::Slider::new(&mut maintain, 0.0..=1.0).text("p(maintain) per epoch"))
        .changed()
    {
        sim.sim.set_track_maintain_p(maintain);
    }
}

/// Movement (§5.1, §10.5) and the allocation horizon (§10.6).
fn movement(ui: &mut egui::Ui, sim: &mut SimRes) {
    ui.label(egui::RichText::new("Movement and planning").strong());
    let (mut risk, mut margin, mut fire, mut horizon) = sim.sim.movement_dials();
    if ui
        .add(egui::Slider::new(&mut risk, 0.0..=1200.0).text("default caution"))
        .changed()
    {
        sim.sim.set_risk_weight(risk);
    }
    if ui
        .add(egui::Slider::new(&mut fire, 0.0..=4.0).text("fire risk weight"))
        .changed()
    {
        sim.sim.set_fire_risk_weight(fire);
    }
    if ui
        .add(egui::Slider::new(&mut margin, 0.0..=0.5).text("re-path margin"))
        .changed()
    {
        sim.sim.set_repath_margin(margin);
    }
    if ui
        .add(egui::Slider::new(&mut horizon, 1..=12).text("allocation horizon (epochs)"))
        .changed()
    {
        sim.sim.set_allocation_horizon(horizon);
    }
}

/// Doctrine (§13): how each side applies the priority list it was given.
///
/// The list itself is scenario-level and shown read-only. Editing an ordered list of names in
/// a panel is a different piece of work, and the *mode* is the half that answers the question
/// the model exists to ask - what directive control costs against optimal control.
fn doctrine(ui: &mut egui::Ui, sim: &mut SimRes) {
    ui.label(egui::RichText::new("Doctrine").strong());
    for side in [Side::Blue, Side::Red] {
        let label = if side == Side::Blue { "Blue" } else { "Red" };
        let mut mode = sim.sim.doctrine_mode(side);
        ui.horizontal(|ui| {
            ui.label(label);
            let before = mode;
            ui.selectable_value(&mut mode, DoctrineMode::Strict, "strict");
            ui.selectable_value(&mut mode, DoctrineMode::Weighted, "weighted");
            if mode != before {
                sim.sim.set_doctrine_mode(side, mode);
            }
        });
        let priority = sim.sim.doctrine_priority(side);
        ui.label(
            egui::RichText::new(format!("   priority: {}", priority.join(" > ")))
                .small()
                .weak(),
        );
    }
}

/// The three that cannot move while a run is in progress, shown rather than hidden.
fn fixed(ui: &mut egui::Ui, sim: &SimRes) {
    let (dt, epoch) = sim.sim.clock_dials();
    ui.label(
        egui::RichText::new(format!(
            "scenario-level: dt {dt:.1} s, epoch {epoch:.0} s, belief grid fixed"
        ))
        .small()
        .weak(),
    );
    ui.label(
        egui::RichText::new("these decide what a tick and an epoch mean, so changing them mid-run would make the two halves of a trial answer different questions")
            .small()
            .weak(),
    );
}

#[cfg(test)]
mod tests {
    use sim_core::doctrine::DoctrineMode;
    use sim_core::scenario::{Libraries, Scenario};
    use sim_core::sim::{Side, Sim};
    use std::path::Path;

    fn fixture() -> Option<Sim> {
        let dir = Path::new("../../scenarios");
        let libs = Libraries::load_dir(dir).ok()?;
        let scn = Scenario::load(&dir.join("covered_approach.toml")).ok()?;
        Sim::new(&scn, &libs, scn.default_seed).ok()
    }

    /// Every live dial must read back what was set. A slider that silently fails to take is
    /// worse than no slider: the model appears to be insensitive to a dial that was never
    /// actually changed.
    #[test]
    fn every_live_dial_reads_back_what_was_set() {
        let Some(mut sim) = fixture() else {
            return;
        };
        sim.set_p_suppress(0.42);
        sim.set_recover_per_s(0.11);
        sim.set_suppression_radius_m(77.0);
        sim.set_suppressed_fire_factor(0.33);
        let (p, r, rad, f) = sim.suppression_dials();
        assert!((p - 0.42).abs() < 1e-6 && (r - 0.11).abs() < 1e-6);
        assert!((rad - 77.0).abs() < 1e-6 && (f - 0.33).abs() < 1e-6);

        sim.set_track_hold_s(65.0);
        sim.set_track_maintain_p(0.8);
        let (hold, maintain) = sim.track_dials();
        assert!((hold - 65.0).abs() < 1e-6 && (maintain - 0.8).abs() < 1e-6);

        sim.set_risk_weight(250.0);
        sim.set_repath_margin(0.2);
        sim.set_fire_risk_weight(1.5);
        sim.set_allocation_horizon(5);
        let (risk, margin, fire, horizon) = sim.movement_dials();
        assert!((risk - 250.0).abs() < 1e-6 && (margin - 0.2).abs() < 1e-6);
        assert!((fire - 1.5).abs() < 1e-6 && horizon == 5);
    }

    /// Probabilities are clamped rather than trusted, so a slider that is ever given a bad
    /// value cannot put the model into a state no scenario could load.
    #[test]
    fn out_of_range_values_are_clamped_not_stored() {
        let Some(mut sim) = fixture() else {
            return;
        };
        sim.set_p_suppress(5.0);
        sim.set_track_maintain_p(-1.0);
        sim.set_allocation_horizon(0);
        let (p, ..) = sim.suppression_dials();
        let (_, maintain) = sim.track_dials();
        let (.., horizon) = sim.movement_dials();
        assert!((p - 1.0).abs() < 1e-6, "p_suppress must clamp to 1");
        assert!(maintain.abs() < 1e-6, "track_maintain_p must clamp to 0");
        assert_eq!(horizon, 1, "an allocation horizon of 0 is not a horizon");
    }

    #[test]
    fn doctrine_mode_is_settable_per_side() {
        let Some(mut sim) = fixture() else {
            return;
        };
        sim.set_doctrine_mode(Side::Red, DoctrineMode::Weighted);
        assert_eq!(sim.doctrine_mode(Side::Red), DoctrineMode::Weighted);
        assert_eq!(
            sim.doctrine_mode(Side::Blue),
            DoctrineMode::Strict,
            "setting one side's doctrine must not touch the other's"
        );
    }
}
