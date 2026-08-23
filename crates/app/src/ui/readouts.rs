//! What the battle currently looks like: force strength, the event feed, and the legend.
//!
//! An inherent `impl` block may live in any module of the crate that defines the
//! type, so these sections keep their `&mut self` signatures and the calls in
//! `Panel::show` are unchanged.

use bevy::prelude::*;
use bevy_egui::egui;
use sim_core::sim::Side;
use sim_core::suppression::Suppression;

use super::Panel;

impl Panel<'_, '_, '_> {
    /// Who has seen whom, who is left, and who is suppressed.
    pub(super) fn force_summary(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        let units = self.sim.sim.units();
        let detected = |side: Side| {
            units
                .iter()
                .filter(|u| u.side == side && u.detected)
                .count()
        };
        ui.label(format!(
            "Detected: {} red / {} blue",
            detected(Side::Red),
            detected(Side::Blue)
        ));
        let elements = |side: Side| -> u32 {
            units
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
        let suppressed = units
            .iter()
            .filter(|u| u.suppression != Suppression::Free && u.alive())
            .count();
        if suppressed > 0 {
            ui.label(format!("{suppressed} unit(s) under suppression"));
        }
    }

    /// The most recent detections and fires, newest first.
    pub(super) fn event_feed(&mut self, ui: &mut egui::Ui) {
        let sim = &self.sim.sim;
        ui.label("Detections:");
        for e in sim.events().iter().rev().take(5) {
            let (s, u) = (&sim.sensors()[e.sensor], &sim.units()[e.unit]);
            ui.small(format!("t={:>4.0}s  {} spotted {}", e.time_s, s.id, u.id));
        }
        ui.label("Fires:");
        for e in sim.fire_events().iter().rev().take(6) {
            // The target may be a battery or a post now, not only a unit (§12.4).
            let (sh, tg) = (&sim.units()[e.shooter].id, sim.fire_target_id(e.target));
            ui.small(format!(
                "t={:>4.0}s  {} hit {} \u{2013}{}{}",
                e.time_s,
                sh,
                tg,
                e.casualties,
                if e.killed { " KILL" } else { "" }
            ));
        }
    }

    /// What every marker on the map means.
    pub(super) fn legend(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.collapsing("Legend", |ui| {
            for line in [
                "○ sensor   ◇ unit   ▷ drone   ✕ destroyed",
                "blue = friendly, red = enemy",
                "white ring = detected",
                "amber ring = suppressed, red ring = pinned",
                "green bar = remaining strength",
                "faint line = movement route / flight plan",
                "yellow ring = selected (left-click, shift adds, drag boxes)",
                "faint wedge = sensor field of regard (swings when tasked)",
                "magenta bubble = EW jammer",
                "teal ring = air-defence envelope",
                "amber square + wide ring = C2 post and its coordination radius",
                "yellow line = air-defence engagement",
                "drone triangle grows with altitude",
            ] {
                ui.small(line);
            }
        });
    }
}
