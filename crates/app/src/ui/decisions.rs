//! The decision-layer dials - allocation, tasking, and the C2 gate on ground fires.
//!
//! An inherent `impl` block may live in any module of the crate that defines the
//! type, so these sections keep their `&mut self` signatures and the calls in
//! `Panel::show` are unchanged.

use bevy::prelude::*;
use bevy_egui::egui;
use sim_core::scenario::AllocationChoice;
use sim_core::sim::Side;

use super::Panel;

impl Panel<'_, '_, '_> {
    /// The decision layer: how fire is allocated, and whether sensors search.
    /// `docs/THEORY.md` §10.
    ///
    /// All three are live: switching between `optimal` and `independent` mid-battle is
    /// how the value of coordinating gets *seen* rather than argued about.
    pub(super) fn decision_section(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        egui::CollapsingHeader::new("Decisions")
            .default_open(false)
            .show(ui, |ui| {
                let mut allocation = self.sim.sim.allocation();
                egui::ComboBox::from_label("fire allocation")
                    .selected_text(match allocation {
                        AllocationChoice::Optimal => "optimal (Hungarian)",
                        AllocationChoice::Greedy => "greedy",
                        AllocationChoice::Independent => "independent (pre-Phase-10)",
                    })
                    .show_ui(ui, |ui| {
                        for (value, label) in [
                            (AllocationChoice::Optimal, "optimal (Hungarian)"),
                            (AllocationChoice::Greedy, "greedy"),
                            (AllocationChoice::Independent, "independent (pre-Phase-10)"),
                        ] {
                            ui.selectable_value(&mut allocation, value, label);
                        }
                    });
                if allocation != self.sim.sim.allocation() {
                    self.sim.sim.set_allocation(allocation);
                }

                let mut air_cap = self.sim.sim.max_batteries_per_air_target();
                if ui
                    .add(egui::Slider::new(&mut air_cap, 1..=4).text("max batteries/airframe"))
                    .on_hover_text(
                        "Measured on ad_c2 (10,000 paired trials): 2 buys no extra kills \
                         over 1 and costs a quarter of a round; 3 is worse on both counts.",
                    )
                    .changed()
                {
                    self.sim.sim.set_max_batteries_per_air_target(air_cap);
                }

                let mut need_c2 = self.sim.sim.fires_need_c2();
                if ui
                    .checkbox(&mut need_c2, "ground fires need a C2 post")
                    .on_hover_text(
                        "Off: the side coordinates its fires for free. On: only shooters \
                         inside a live post's (jammed) radius join the side-wide plan, and \
                         the rest each pick for themselves. Try the fires_c2 scenario.",
                    )
                    .changed()
                {
                    self.sim.sim.set_fires_need_c2(need_c2);
                }

                let mut tasking = self.sim.sim.sensor_tasking();
                if ui
                    .checkbox(&mut tasking, "sensors search by belief")
                    .changed()
                {
                    self.sim.sim.set_sensor_tasking(tasking);
                }
                ui.small(
                    "Only steerable sensors (a field of regard) can be tasked. Try the \
                     sensor_search scenario.",
                );
                let (blue, red) = (
                    self.sim.sim.belief_of(Side::Blue).entropy(),
                    self.sim.sim.belief_of(Side::Red).entropy(),
                );
                ui.small(format!(
                    "belief entropy: blue {blue:.2} / red {red:.2} nats"
                ));
            });
    }
}
