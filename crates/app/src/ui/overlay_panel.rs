//! Overlay requests and the line-of-sight probe readout.
//!
//! An inherent `impl` block may live in any module of the crate that defines the
//! type, so these sections keep their `&mut self` signatures and the calls in
//! `Panel::show` are unchanged.

use bevy::prelude::*;
use bevy_egui::egui;
use sim_core::sim::Side;

use crate::overlays;
use crate::state::{OverlayKind, OverlayRequest};

use super::Panel;

impl Panel<'_, '_, '_> {
    /// The map overlays and the exposure window they are computed over.
    pub(super) fn overlay_buttons(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.add(
            egui::Slider::new(&mut self.ui_state.coverage_exposure_s, 5.0..=180.0)
                .text("exposure s"),
        );
        ui.horizontal(|ui| {
            ui.label("from side:");
            let side = &mut self.ui_state.overlay_side;
            ui.selectable_value(side, Side::Blue, "Blue");
            ui.selectable_value(side, Side::Red, "Red");
        });

        let request = |kind| OverlayRequest {
            kind,
            side: self.ui_state.overlay_side,
            exposure_s: self.ui_state.coverage_exposure_s,
        };
        let mut asked: Option<OverlayRequest> = None;
        ui.horizontal(|ui| {
            if ui.button("Coverage (Pd)").clicked() {
                asked = Some(request(OverlayKind::Coverage));
            }
            if ui.button("Belief snapshot").clicked() {
                asked = Some(request(OverlayKind::BeliefSnapshot));
            }
        });
        if ui.button("Belief the sim is flying on").clicked() {
            asked = Some(request(OverlayKind::SimBelief));
        }
        if let Some(r) = asked {
            overlays::request_overlay(self.sim, r, self.overlay);
        }

        // Auto-refresh, and the status line that makes a stale overlay obvious. Before
        // this, an overlay silently went on describing where a sensor used to be.
        ui.checkbox(&mut self.overlay.auto, "keep it up to date");
        if ui.button("Clear overlay").clicked() {
            overlays::clear_overlay(self.overlay, self.commands);
        }
        if self.overlay.pending.is_some() {
            ui.label("computing... (the map stays live)");
        } else if let Some(showing) = self.overlay.showing {
            let stale = crate::state::overlay_fingerprint(&self.sim.sim) != self.overlay.built_from;
            let side = if showing.side == Side::Blue {
                "Blue"
            } else {
                "Red"
            };
            if stale && !self.overlay.auto {
                ui.colored_label(
                    egui::Color32::from_rgb(200, 140, 60),
                    format!(
                        "{} ({side}) - STALE, assets have moved",
                        showing.kind.label()
                    ),
                );
            } else {
                ui.label(format!("{} ({side})", showing.kind.label()));
            }
        }
    }

    /// The last LOS probe result.
    pub(super) fn probe_readout(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        if let Some(r) = &self.probe.last {
            ui.label(format!(
                "LOS: {}  τ = {:.2}\nmask {:+.1} m, canopy {:.0} m",
                if r.clear { "CLEAR" } else { "BLOCKED" },
                r.transmittance,
                r.mask_height,
                r.canopy_length
            ));
        }
    }
}
