//! The air section: what to place, how it flies, and what it has done.
//!
//! An inherent `impl` block may live in any module of the crate that defines the
//! type, so these sections keep their `&mut self` signatures and the calls in
//! `Panel::show` are unchanged.

use bevy::prelude::*;
use bevy_egui::egui;
use sim_core::air::AltitudeRef;

use crate::state::Selected;

use super::Panel;

impl Panel<'_, '_, '_> {
    /// Drone and air-defence types, the flight dials, and applying them to a selection.
    /// `docs/DESIGN.md` §9.
    pub(super) fn air_section(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        egui::CollapsingHeader::new("Air")
            .default_open(true)
            .show(ui, |ui| {
                self.air_type_pickers(ui);
                let s = &mut *self.ui_state;
                ui.add(egui::Slider::new(&mut s.air_altitude_m, 0.0..=2000.0).text("altitude m"));
                ui.checkbox(
                    &mut s.air_altitude_amsl,
                    "altitude is AMSL (terrain can mask)",
                );
                ui.add(egui::Slider::new(&mut s.air_heading_deg, 0.0..=359.0).text("heading °"));
                ui.add(egui::Slider::new(&mut s.air_speed_m_s, 0.0..=120.0).text("speed m/s"));
                ui.add(
                    egui::Slider::new(&mut s.air_orbit_radius_m, 100.0..=2000.0)
                        .text("orbit radius m"),
                );
                self.air_dials(ui);
            });
    }

    pub(super) fn air_type_pickers(&mut self, ui: &mut egui::Ui) {
        let air = &self.sim.data.libs.air;
        let chosen = &mut self.ui_state.air_type_id;
        egui::ComboBox::from_label("drone type")
            .selected_text(chosen.clone())
            .show_ui(ui, |ui| {
                for key in air.keys() {
                    ui.selectable_value(chosen, key.clone(), key);
                }
            });
        let ad = &self.sim.data.libs.air_defence;
        let chosen = &mut self.ui_state.air_defence_type_id;
        egui::ComboBox::from_label("AD type")
            .selected_text(chosen.clone())
            .show_ui(ui, |ui| {
                for key in ad.keys() {
                    ui.selectable_value(chosen, key.clone(), key);
                }
            });
        let c2 = &self.sim.data.libs.c2;
        if !c2.is_empty() {
            let chosen = &mut self.ui_state.c2_type_id;
            egui::ComboBox::from_label("C2 type")
                .selected_text(chosen.clone())
                .show_ui(ui, |ui| {
                    for key in c2.keys() {
                        ui.selectable_value(chosen, key.clone(), key);
                    }
                });
        }
    }

    /// The selected drone's readout, and the button that pushes the dials above onto
    /// every selected drone - so a formation is re-tasked in one go, not one at a time.
    pub(super) fn air_dials(&mut self, ui: &mut egui::Ui) {
        let selected_air: Vec<usize> = self
            .ui_state
            .selected
            .iter()
            .filter_map(|s| match s {
                Selected::Air(i) => Some(*i),
                _ => None,
            })
            .collect();
        if let [only] = selected_air[..] {
            let a = &self.sim.sim.air()[only];
            let agl = a.actor_height(self.sim.sim.terrain());
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
        if !selected_air.is_empty()
            && ui
                .button(format!("Apply dials to {} drone(s)", selected_air.len()))
                .clicked()
        {
            for i in selected_air {
                let a = self.sim.sim.air_mut(i);
                a.altitude_m = self.ui_state.air_altitude_m;
                a.altitude_ref = if self.ui_state.air_altitude_amsl {
                    AltitudeRef::Amsl
                } else {
                    AltitudeRef::Agl
                };
                a.heading_deg = self.ui_state.air_heading_deg;
                a.speed_m_s = self.ui_state.air_speed_m_s;
            }
        }
    }

    /// The counter-air picture: what is flying, what each battery is doing, and the
    /// recent engagements and releases (`docs/DESIGN.md` §9).
    pub(super) fn air_feed(&mut self, ui: &mut egui::Ui) {
        let sim = &self.sim.sim;
        if sim.air().is_empty() && sim.air_defence().is_empty() {
            return;
        }
        let air_alive = sim.air().iter().filter(|a| a.alive).count();
        let air_lost = sim.air().len() - air_alive;
        ui.label(format!(
            "Air: {air_alive} flying / {air_lost} down   AD: {} batteries",
            sim.air_defence().len()
        ));
        for ad in sim.air_defence() {
            if !ad.alive() {
                ui.small(format!("  {}: DESTROYED", ad.id));
                continue;
            }
            let mag = if ad.stats.magazine == 0 {
                "∞".to_owned()
            } else {
                ad.magazine_left.to_string()
            };
            ui.small(format!(
                "  {}: {}/{} up, {} rounds, {} engaging{}",
                ad.id,
                ad.elements,
                ad.stats.element_count,
                mag,
                ad.engagements.len(),
                if ad.self_cue { "" } else { " (net-cued)" }
            ));
        }
        // C2 (docs/DESIGN.md §11): which batteries are coordinating, and whether the post
        // holding them together is still alive.
        for post in sim.c2() {
            let covered = sim
                .air_defence()
                .iter()
                .filter(|ad| ad.side == post.side && ad.alive() && post.covers(ad.pos))
                .count();
            ui.small(if post.alive() {
                format!("  {}: C2 post, coordinating {covered}", post.id)
            } else {
                format!("  {}: C2 post DESTROYED - defence decohered", post.id)
            });
        }
        ui.label("Air events:");
        for e in sim.air_defence_events().iter().rev().take(4) {
            let (ad, a) = (&sim.air_defence()[e.battery], &sim.air()[e.air]);
            ui.small(format!(
                "t={:>4.0}s  {} {} {}",
                e.time_s,
                ad.id,
                if e.killed { "DOWNED" } else { "missed" },
                a.id
            ));
        }
        for e in sim.strike_events().iter().rev().take(4) {
            let a = &sim.air()[e.air];
            ui.small(format!(
                "t={:>4.0}s  {} released \u{2013}{} elem",
                e.time_s, a.id, e.casualties
            ));
        }
    }
}
