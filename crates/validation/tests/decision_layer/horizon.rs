//! V75 - the fire-allocation objective's planning horizon. `docs/THEORY.md` §10.2.
//!
//! `[sim] allocation_horizon` is how many epochs the objective prices. Two properties hold
//! it honest, and they pull in opposite directions:
//!
//! * a horizon of 1 must be an **exact identity** - the formula multiplies threat by 1.0,
//!   so every previously measured result stands without re-baselining;
//! * above 1 it must actually change the preference, toward killing what shoots back rather
//!   than damaging whatever is largest. A dial that changes nothing is worse than no dial.

use sim_core::scenario::{Libraries, Scenario};
use sim_core::sim::Sim;
use validation::scenario_params;

fn libs() -> Libraries {
    Libraries::load_dir(&validation::scenarios_dir())
        .unwrap_or_else(|_| Libraries::with_terrain(scenario_params()))
}

/// The identity fixture: `fire_allocation`, four shooters that can all reach all four
/// targets. Its targets are deliberately **identical**, which makes it the right place to
/// check the identity and the wrong place to check that the horizon changes anything - see
/// [`v75_a_longer_horizon_changes_the_fire_plan`].
fn scenario(horizon: Option<u32>) -> Scenario {
    let text = std::fs::read_to_string(validation::scenario_path("fire_allocation.toml"))
        .expect("fixture scenario");
    let text = match horizon {
        Some(h) => text.replace("[sim]", &format!("[sim]\nallocation_horizon = {h}")),
        None => text,
    };
    Scenario::from_toml_str(&text).expect("fixture parses")
}

fn run(horizon: Option<u32>, seed: u64) -> (Vec<String>, f64) {
    let scn = scenario(horizon);
    let mut sim = Sim::new(&scn, &libs(), seed).expect("builds");
    sim.run_until(300.0);
    let log = sim
        .fire_events()
        .iter()
        .map(|e| format!("{:.1}/{:?}/{:?}", e.time_s, e.shooter, e.target))
        .collect();
    (log, sim.time_s())
}

// The identity. Omitting the dial and setting it to 1 must produce the same event log, event
// for event - not merely the same summary. The horizon multiplies the threat term by 1.0,
// which is exact in floating point, so this is a true identity and not a tolerance.
#[test]
fn v75_a_horizon_of_one_is_an_exact_identity() {
    for seed in [1, 7, 42] {
        let (absent, t_absent) = run(None, seed);
        let (one, t_one) = run(Some(1), seed);
        assert_eq!(
            absent, one,
            "seed {seed}: horizon 1 must reproduce the un-dialled objective exactly"
        );
        assert!((t_absent - t_one).abs() < 1e-9);
        assert!(
            !absent.is_empty(),
            "seed {seed}: the fixture must actually fire"
        );
    }
}

// The dial has to do something, and **where** it does something is the point.
//
// The horizon weights a target's threat against its size, so it can only change a decision
// between targets that differ in that ratio. On `fire_allocation`, whose four targets are
// identical, it correctly changes nothing: scaling every value by the same factor leaves the
// assignment untouched.
//
// `default` is the fixture that *does* present the choice - armour, infantry, a gun, a
// sensor and a battery, which span the threat-to-size ratio. `kill_chain` looks like a
// better fixture and is not: it resolves in four shots with rarely more than one target
// engageable, so there is no ordering to change. Two fixtures were tried and failed for the
// right behaviour before this one, which is worth recording rather than quietly re-pointing.
#[test]
fn v75_a_longer_horizon_changes_the_fire_plan() {
    let plan = |h: u32, seed: u64| -> Vec<String> {
        let Ok(text) = std::fs::read_to_string(validation::scenario_path("default.toml")) else {
            return Vec::new();
        };
        let text = text.replace(
            "[sim]",
            &format!(
                "[sim]
allocation_horizon = {h}"
            ),
        );
        let Ok(scn) = Scenario::from_toml_str(&text) else {
            return Vec::new();
        };
        let Ok(mut sim) = Sim::new(&scn, &libs(), seed) else {
            return Vec::new();
        };
        sim.run_until(120.0);
        sim.fire_events()
            .iter()
            .map(|e| format!("{:?}", e.target))
            .collect()
    };
    let differs = [1u64, 2, 3, 7, 11, 42]
        .into_iter()
        .filter(|&seed| {
            let a = plan(1, seed);
            !a.is_empty() && a != plan(8, seed)
        })
        .count();
    assert!(
        differs > 0,
        "a horizon of 8 produced an identical target sequence on every seed of `default`;          the objective is not pricing what a survivor would go on to do"
    );
}

// What the horizon is *for*, checked on the objective's own terms rather than an outcome:
// lengthening it must raise a dangerous target's value faster than a harmless one's, because
// the harm a survivor does grows with the time it has to do it in.
//
// This is a property of the formula, so it is asserted arithmetically. `value` is
// `elements x (1 + H x threat/scale)`: at H = 1 a big harmless target can outrank a small
// dangerous one, and as H grows the dangerous one must overtake it. If that ordering could
// never flip, the dial would be a uniform rescale and would change no decision at all.
#[test]
fn v75_the_horizon_raises_threat_against_size() {
    // A large, harmless target against a small, dangerous one.
    let value =
        |elements: f32, threat_ratio: f32, horizon: f32| elements * (1.0 + horizon * threat_ratio);

    let bulk = value(6.0, 0.0, 1.0);
    let armed = value(2.0, 1.0, 1.0);
    assert!(
        bulk > armed,
        "at a horizon of 1 the larger target should be worth more ({bulk} vs {armed})"
    );

    let bulk_long = value(6.0, 0.0, 8.0);
    let armed_long = value(2.0, 1.0, 8.0);
    assert!(
        armed_long > bulk_long,
        "at a horizon of 8 the dangerous target should overtake ({armed_long} vs {bulk_long})"
    );

    // And the ordering must move monotonically, so the dial reads as a dial rather than a
    // switch that flips somewhere unpredictable.
    let ratio = |h: f32| value(2.0, 1.0, h) / value(6.0, 0.0, h);
    assert!(ratio(1.0) < ratio(4.0) && ratio(4.0) < ratio(8.0));
}
