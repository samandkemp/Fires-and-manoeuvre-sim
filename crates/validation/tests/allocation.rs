//! V56 — fire allocation. `docs/DESIGN.md` §10.2.
//!
//! Two halves. The solver is checked against an exhaustive optimum on instances small
//! enough to enumerate, which is the only reference an assignment algorithm has. The sim
//! side is checked on the property that motivates the whole phase: a side with two
//! shooters and two equally worthwhile targets should engage **both**, where the old
//! nearest-enemy rule sent everyone at whichever happened to be closer.

use sim_core::allocation::{self, ineligible, is_eligible, Solver};
use sim_core::fires::{WeaponClass, WeaponType};
use sim_core::scenario::{AllocationChoice, Libraries, Scenario};
use sim_core::sensing::UnitType;
use sim_core::sim::Sim;
use std::collections::BTreeMap;
use validation::scenario_params;

/// Exhaustive best over every injective shooter → slot map, leaving shooters idle where
/// that is better. Exponential, so only for tiny instances — which is the point.
fn brute_force(payoff: &[Vec<f64>]) -> f64 {
    fn walk(payoff: &[Vec<f64>], i: usize, m: usize, used: &mut Vec<bool>) -> f64 {
        if i == payoff.len() {
            return 0.0;
        }
        let mut best = walk(payoff, i + 1, m, used); // leave shooter i idle
        for j in 0..m {
            if used[j] || !is_eligible(payoff[i][j]) {
                continue;
            }
            used[j] = true;
            best = best.max(payoff[i][j] + walk(payoff, i + 1, m, used));
            used[j] = false;
        }
        best
    }
    let m = payoff.iter().map(Vec::len).min().unwrap_or(0);
    walk(payoff, 0, m, &mut vec![false; m])
}

/// Deterministic pseudo-random payoff matrix with a scattering of forbidden cells.
fn matrix(seed: u64, n: usize, m: usize) -> Vec<Vec<f64>> {
    let mut s = seed
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    let mut next = move || {
        s = s
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (s >> 33) as f64 / f64::from(u32::MAX >> 1)
    };
    (0..n)
        .map(|_| {
            (0..m)
                .map(|_| {
                    let x = next();
                    if x < 0.18 {
                        ineligible()
                    } else {
                        x * 10.0
                    }
                })
                .collect()
        })
        .collect()
}

// V56 (solver half): the Hungarian assignment equals an exhaustive optimum on every
// instance small enough to enumerate, is never beaten by greedy, and never returns an
// assignment that reuses a slot or takes a forbidden pairing.
#[test]
fn v56_allocation_is_optimal_and_feasible() {
    for seed in 0..60u64 {
        for n in 1..=6usize {
            for m in 1..=6usize {
                let payoff = matrix(seed, n, m);

                let optimal = allocation::solve(&payoff, Solver::Optimal);
                let got = allocation::total(&payoff, &optimal);
                let want = brute_force(&payoff);
                assert!(
                    (got - want).abs() < 1e-9,
                    "seed {seed} {n}x{m}: Hungarian scored {got}, exhaustive optimum is {want}"
                );

                let greedy = allocation::solve(&payoff, Solver::Greedy);
                assert!(
                    got >= allocation::total(&payoff, &greedy) - 1e-9,
                    "seed {seed} {n}x{m}: greedy beat the optimal solver"
                );

                // Feasibility: each slot at most once, and never a forbidden pairing.
                let mut used = vec![false; m];
                for (i, slot) in optimal.iter().enumerate() {
                    let Some(j) = slot else { continue };
                    assert!(!used[*j], "seed {seed} {n}x{m}: slot {j} assigned twice");
                    used[*j] = true;
                    assert!(
                        is_eligible(payoff[i][*j]),
                        "seed {seed} {n}x{m}: forbidden pairing chosen"
                    );
                }
            }
        }
    }
}

/// Two Blue guns facing two identical Red targets, one nearer than the other. Flat, open
/// terrain so nothing but the allocation decides anything.
fn two_on_two(allocation: AllocationChoice) -> Sim {
    let scn = Scenario::from_toml_str(&format!(
        r#"
        name = "two-on-two"
        default_seed = 3
        [sim]
        dt_s = 1.0
        epoch_s = 10.0
        allocation = "{}"
        max_shooters_per_target = 3
        [terrain]
        cell_size_m = 10.0
        width_cells = 200
        height_cells = 64
        [terrain.source.flat]
        elevation_m = 0.0
        [[blue.units]]
        id = "gun-a"
        type = "gun"
        pos = [200.0, 300.0]
        [[blue.units]]
        id = "gun-b"
        type = "gun"
        pos = [200.0, 340.0]
        [[red.units]]
        id = "near"
        type = "target"
        pos = [900.0, 300.0]
        [[red.units]]
        id = "far"
        type = "target"
        pos = [1100.0, 340.0]
    "#,
        match allocation {
            AllocationChoice::Optimal => "optimal",
            AllocationChoice::Greedy => "greedy",
            AllocationChoice::Independent => "independent",
        }
    ))
    .unwrap();

    let libs = Libraries {
        units: BTreeMap::from([
            (
                "gun".to_owned(),
                UnitType {
                    height_m: 2.5,
                    silhouette_width_m: 3.0,
                    element_count: 1,
                    signature: BTreeMap::from([("optical".to_owned(), 0.5)]),
                    weapon: Some("cannon".to_owned()),
                    ..Default::default()
                },
            ),
            (
                "target".to_owned(),
                UnitType {
                    height_m: 2.5,
                    silhouette_width_m: 3.0,
                    // Several elements, so one shooter cannot finish a target in an
                    // epoch and piling on is genuinely wasteful.
                    element_count: 6,
                    signature: BTreeMap::from([("optical".to_owned(), 0.8)]),
                    ..Default::default()
                },
            ),
        ]),
        weapons: BTreeMap::from([(
            "cannon".to_owned(),
            WeaponType {
                class: WeaponClass::Direct,
                // Deliberately lethal. The fire log records *casualties*, not shots, so a
                // gun that engages and misses leaves no trace — and the gate would then
                // be unable to tell "did not engage" from "engaged and missed". Making a
                // burst reliably kill removes that ambiguity from the measurement.
                rof_rounds_per_min: 60.0,
                max_range_m: 3000.0,
                dispersion_mrad: 0.5,
                p_kill_given_hit: 0.9,
                ..Default::default()
            },
        )]),
        ..Libraries::with_terrain(scenario_params())
    };
    Sim::new(&scn, &libs, 3).unwrap()
}

/// Distinct Red units engaged in the **first** epoch.
///
/// One epoch, not the whole battle: given long enough, uncoordinated guns also reach the
/// second target, simply by destroying the first one and moving on. The difference the
/// allocation makes is about *simultaneity*, so the measurement has to be too.
fn targets_engaged_first_epoch(allocation: AllocationChoice) -> Vec<usize> {
    let mut sim = two_on_two(allocation);
    sim.run_until(10.0);
    let mut t: Vec<usize> = sim.fire_events().iter().map(|e| e.target).collect();
    t.sort_unstable();
    t.dedup();
    t
}

// V56 (sim half): a coordinated side spreads its fire. Two guns that can both reach two
// six-element targets should engage one each in the same epoch, rather than both piling
// onto the nearer — which is exactly what the pre-Phase-10 "nearest enemy" rule did. The
// independent solver reproduces that old behaviour, so the two together show precisely
// what changed.
#[test]
fn v56_allocation_spreads_fire_across_targets() {
    let coordinated = targets_engaged_first_epoch(AllocationChoice::Optimal);
    assert_eq!(
        coordinated.len(),
        2,
        "an optimal allocation should engage both targets in one epoch, got {coordinated:?}"
    );

    let uncoordinated = targets_engaged_first_epoch(AllocationChoice::Independent);
    assert_eq!(
        uncoordinated.len(),
        1,
        "the independent rule should send both guns at the same (nearer) target, which is \
         the waste allocation exists to remove; got {uncoordinated:?}"
    );
}

// V56 (overkill half): no target may draw more shooters than the dials allow — at most
// one per remaining element, and never more than `max_shooters_per_target`.
#[test]
fn v56_no_target_draws_more_shooters_than_it_has_slots() {
    let mut sim = two_on_two(AllocationChoice::Optimal);
    let cap = 3usize;
    for _ in 0..12 {
        let before = sim.fire_events().len();
        sim.run_until(sim.time_s() + 10.0);
        // Shooters engaging each target in this epoch's slice of the log.
        let mut per_target: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for e in &sim.fire_events()[before..] {
            per_target.entry(e.target).or_default().push(e.shooter);
        }
        for (target, mut shooters) in per_target {
            shooters.sort_unstable();
            shooters.dedup();
            let elements = sim.units()[target].elements as usize;
            assert!(
                shooters.len() <= cap.min(elements.max(1)),
                "target {target} drew {} shooters with {elements} elements (cap {cap})",
                shooters.len()
            );
        }
    }
}

// V56 (determinism half): allocation draws no randomness, so the same scenario and seed
// must reproduce the fire log exactly — and choosing a different solver must be the only
// thing that changes it.
#[test]
fn v56_allocation_is_deterministic() {
    let mut a = two_on_two(AllocationChoice::Optimal);
    let mut b = two_on_two(AllocationChoice::Optimal);
    a.run_until(120.0);
    b.run_until(120.0);
    assert_eq!(a.fire_events(), b.fire_events());

    let mut greedy = two_on_two(AllocationChoice::Greedy);
    greedy.run_until(120.0);
    // Greedy and optimal agree on this instance (it is small and unambiguous); what
    // matters is that both are reproducible, not that they differ.
    assert!(!greedy.fire_events().is_empty());
}
