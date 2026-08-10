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
use sim_core::air::AltitudeRef;
use sim_core::scenario::AllocationChoice;
use sim_core::sim::{Side, Sim};
use sim_core::suppression::Suppression;

use crate::overlays;
use crate::state::{ClickMode, Overlay, PendingLoad, Probe, ResetKind, Selected, SimRes, UiState};

/// Everything the panel draws from, gathered so each section takes only `&mut self`.
pub struct Panel<'a, 'w, 's> {
    pub sim: &'a mut SimRes,
    pub ui_state: &'a mut UiState,
    pub probe: &'a Probe,
    pub overlay: &'a mut Overlay,
    pub commands: &'a mut Commands<'w, 's>,
    pub images: &'a mut Assets<Image>,
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
        self.overlay_buttons(ui);
        self.probe_readout(ui);
        self.force_summary(ui);
        self.event_feed(ui);
        self.air_feed(ui);
        self.legend(ui);
    }

    /// Run/pause, manual stepping, playback speed, breakpoints, and the two resets.
    ///
    /// Speed is in **sim seconds per real second**, not ticks per frame. A battle is over
    /// in a few hundred seconds of sim time, so at one tick per rendered frame - the old
    /// control - everything interesting happened while you were still reading the panel.
    /// Below 1× the same run just takes longer to watch; the event log is identical
    /// either way (`main::advance_sim`).
    fn clock(&mut self, ui: &mut egui::Ui) {
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
    fn scenario_picker(&mut self, ui: &mut egui::Ui) {
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
    fn controls_help(&mut self, ui: &mut egui::Ui) {
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

    /// What a right-click places. The only remaining mode in the UI.
    fn placement_modes(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.label("Right-click places:");
        let mode = &mut self.ui_state.mode;
        for (value, label) in [
            (ClickMode::Probe, "nothing (move/route)"),
            (ClickMode::PlaceBlueSensor, "Place Blue sensor"),
            (ClickMode::PlaceRedUnit, "Place Red unit"),
            (ClickMode::PlaceRedJammer, "Place Red jammer (EW)"),
            (ClickMode::PlaceRedAir, "Place Red drone"),
            (ClickMode::PlaceBlueAirDefence, "Place Blue air defence"),
            (ClickMode::PlaceBlueC2, "Place Blue C2 post"),
            (ClickMode::AirOrbit, "Drone orbit here (radius below)"),
        ] {
            ui.radio_value(mode, value, label);
        }
    }

    /// What is selected - after dropping anything that died or was cleared under us, so a
    /// stale index can never be commanded.
    fn selection_readout(&mut self, ui: &mut egui::Ui) {
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
    fn type_pickers(&mut self, ui: &mut egui::Ui) {
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

    /// Drone and air-defence types, the flight dials, and applying them to a selection.
    /// `docs/DESIGN.md` §9.
    fn air_section(&mut self, ui: &mut egui::Ui) {
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

    fn air_type_pickers(&mut self, ui: &mut egui::Ui) {
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
    fn air_dials(&mut self, ui: &mut egui::Ui) {
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

    /// The Phase 10 decision layer: how fire is allocated, and whether sensors search.
    /// `docs/DESIGN.md` §10.
    ///
    /// All three are live: switching between `optimal` and `independent` mid-battle is
    /// how the value of coordinating gets *seen* rather than argued about.
    fn decision_section(&mut self, ui: &mut egui::Ui) {
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

    /// The map overlays and the exposure window they are computed over.
    fn overlay_buttons(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.add(
            egui::Slider::new(&mut self.ui_state.coverage_exposure_s, 5.0..=180.0)
                .text("exposure s"),
        );
        let exposure = self.ui_state.coverage_exposure_s;
        if ui.button("Coverage overlay (Pd)").clicked() {
            overlays::rebuild_coverage_overlay(
                self.sim,
                exposure,
                self.overlay,
                self.commands,
                self.images,
            );
        }
        if ui
            .button("Belief snapshot (where Red could hide)")
            .clicked()
        {
            overlays::rebuild_belief_overlay(
                self.sim,
                exposure,
                self.overlay,
                self.commands,
                self.images,
            );
        }
        // The sim's own running filter, as opposed to the snapshot above - this is what
        // the tasking layer actually reads when deciding where to look.
        if ui.button("Belief the sim is flying on (Blue)").clicked() {
            overlays::rebuild_sim_belief_overlay(
                self.sim,
                Side::Blue,
                self.overlay,
                self.commands,
                self.images,
            );
        }
        ui.label("(coverage/snapshot from Blue sensors, vs 'afv')");
    }

    /// The last LOS probe result.
    fn probe_readout(&mut self, ui: &mut egui::Ui) {
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

    /// Who has seen whom, who is left, and who is suppressed.
    fn force_summary(&mut self, ui: &mut egui::Ui) {
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
    fn event_feed(&mut self, ui: &mut egui::Ui) {
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

    /// The counter-air picture: what is flying, what each battery is doing, and the
    /// recent engagements and releases (`docs/DESIGN.md` §9).
    fn air_feed(&mut self, ui: &mut egui::Ui) {
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

    /// What every marker on the map means.
    fn legend(&mut self, ui: &mut egui::Ui) {
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
    // Every reset drops the overlay: it was computed against the old world.
    let drop_overlay = |overlay: &mut Overlay, commands: &mut Commands| {
        if let Some(e) = overlay.0.take() {
            commands.entity(e).despawn();
        }
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
