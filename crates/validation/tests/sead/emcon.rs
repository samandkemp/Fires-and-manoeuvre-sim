//! V69 — `emitting` and `self_cue` are two different decisions. `docs/DESIGN.md` §12.5.
//!
//! §9.5 introduced `self_cue` as the **cueing timeline**: does this battery act on its own
//! radar, or wait for a track over the net and pay `cue_latency_s`? §12.3 then reused the
//! same flag as the **emission** test for anti-radiation homing.
//!
//! One flag, two meanings, and they disagree. A battery with `self_cue = false` counted as
//! silent to a missile while its radar kept detecting perfectly well — measured on
//! `scenarios/sead_arm.toml` at 1.000 detections with first contact at 9.3 s, statistically
//! indistinguishable from the emitting arm. It bought the survivability of EMCON without
//! the blindness that is supposed to pay for it.
//!
//! Split, the two now fail differently, and this gate holds them apart:
//!
//! * `emitting = false` — the radar is **off**. No detections through it, no self-cue, and
//!   nothing for an ARM to home on.
//! * `self_cue = false` — the radar **runs**. It detects, an ARM can see it, but the
//!   battery takes its cue from the net and pays the latency.

use sim_core::scenario::{Libraries, Scenario};
use sim_core::sim::Sim;

/// A battery with an organic radar watching a drone fly past, with the two flags set
/// independently. Nothing else can see: the battery's own radar is the side's only sensor,
/// so every detection recorded is one it made itself.
fn battery_watching(emitting: bool, self_cue: bool) -> Sim {
    let scn = Scenario::from_toml_str(&format!(
        r#"
        name = "emcon"
        default_seed = 4
        [sim]
        dt_s = 1.0
        epoch_s = 10.0
        [terrain]
        cell_size_m = 10.0
        width_cells = 300
        height_cells = 200
        [terrain.source.flat]
        elevation_m = 0.0
        [[blue.air_defence]]
        id = "sam-1"
        type = "ciws"
        pos = [1000.0, 1000.0]
        emitting = {emitting}
        self_cue = {self_cue}
        [[red.air]]
        id = "uas-1"
        type = "recce_uas"
        pos = [2600.0, 1000.0]
        heading_deg = 180.0
        waypoints = [[400.0, 1000.0]]
    "#
    ))
    .unwrap();
    // The shipped stat blocks, so `ciws` and `recce_uas` mean here what they mean in a
    // scenario — this gate is about a flag, not about invented numbers.
    let libs = Libraries::load_dir(&validation::scenarios_dir()).expect("stat blocks load");
    Sim::new(&scn, &libs, 4).expect("fixture builds")
}

/// Air detections made over a run. The battery's own radar is the only sensor on the
/// field, so every event here is one it made itself.
fn detections(mut sim: Sim) -> usize {
    sim.run_until(120.0);
    sim.air_events().len()
}

// The half that was broken. A battery under EMCON has its radar off, so it sees nothing
// through it — where before it went on detecting exactly as if the radar were running.
#[test]
fn v69_a_silent_battery_detects_nothing() {
    assert_eq!(
        detections(battery_watching(false, true)),
        0,
        "a battery with its radar off must not detect through it"
    );
}

// The other half, and the reason the flags are separate: a battery that is *listening to
// the net* still has its radar on. It detects normally; what it gives up is immediacy.
#[test]
fn v69_a_net_cued_battery_still_detects() {
    assert!(
        detections(battery_watching(true, false)) > 0,
        "`self_cue = false` is about who the battery listens to, not whether its radar runs"
    );
}

// Both defaults are `true`, so an ordinary battery is unaffected by the split: it detects,
// and it detects the same amount whichever way the *other* flag is set.
#[test]
fn v69_the_defaults_are_an_exact_identity() {
    let both_on = detections(battery_watching(true, true));
    assert!(both_on > 0, "a transmitting, self-cueing battery detects");
    assert_eq!(
        both_on,
        detections(battery_watching(true, false)),
        "cueing route must not change what the radar SEES — only when the battery may act \
         on it. If these differ, the flags are entangled again"
    );
}
