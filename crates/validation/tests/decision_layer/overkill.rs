//! V68 — the overkill discount replaces the overkill cap. `docs/DESIGN.md` §11.4.
//!
//! §10.2 prices the *k*-th shooter on a target at `(1 - q̄)^k`: the extra shooter only helps
//! if every one before it failed. A hard cap on top of that — `max_shooters_per_target`,
//! now removed — did not *discourage* piling on, it **truncated** the option, and a shooter
//! with nothing else to engage was assigned nothing at all.
//!
//! That is the wrong trade whenever targets are scarcer than shooters, which for indirect
//! fire is most of the opening: a target has to be tracked before it can be shot at. It was
//! visible as an inversion — on `fires_c2.toml`, splitting a side in two with
//! `fires_need_c2` made it fight *better*, because the cap applied once per fire-control
//! problem and a split side therefore got two of them.
//!
//! Two properties pin the replacement, and they pull in opposite directions, which is why
//! both are needed: fire must not idle when there is only one thing to shoot, and it must
//! still spread when there is more than one. The first is what the cap got wrong; the
//! second is what the cap was *for*, and the discount has to keep delivering it alone.

use sim_core::fires::{WeaponClass, WeaponType};
use sim_core::scenario::{Libraries, Scenario};
use sim_core::sensing::UnitType;
use sim_core::sim::Sim;
use std::collections::{BTreeMap, BTreeSet};
use validation::scenario_params;

/// `n_guns` Blue guns against `n_targets` identical Red units, flat and open, every gun in
/// range and line of sight of every target.
///
/// Direct fire deliberately: it needs no track, so the measurement is not hostage to when
/// acquisition happens to succeed. Targets carry enough elements to survive several epochs,
/// because the fire log records **casualties** — a gun that engages and kills nothing
/// leaves no trace, and the gate could not then tell "did not engage" from "engaged and
/// missed".
fn guns_against(n_guns: usize, n_targets: usize) -> Sim {
    let mut s = String::from(
        r#"
        name = "overkill"
        default_seed = 5
        [sim]
        dt_s = 1.0
        epoch_s = 10.0
        [terrain]
        cell_size_m = 10.0
        width_cells = 200
        height_cells = 64
        [terrain.source.flat]
        elevation_m = 0.0
        "#,
    );
    for i in 0..n_guns {
        let y = 300.0 + 40.0 * i as f32;
        s.push_str(&format!(
            "[[blue.units]]\nid = \"gun-{i}\"\ntype = \"gun\"\npos = [200.0, {y}]\n"
        ));
    }
    for j in 0..n_targets {
        let y = 300.0 + 40.0 * j as f32;
        s.push_str(&format!(
            "[[red.units]]\nid = \"tgt-{j}\"\ntype = \"target\"\npos = [1000.0, {y}]\n"
        ));
    }
    let scn = Scenario::from_toml_str(&s).expect("fixture parses");

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
                    element_count: 20,
                    signature: BTreeMap::from([("optical".to_owned(), 0.8)]),
                    ..Default::default()
                },
            ),
        ]),
        weapons: BTreeMap::from([(
            "cannon".to_owned(),
            WeaponType {
                class: WeaponClass::Direct,
                // A couple of rounds an epoch, reliably lethal: enough that every engaging
                // gun leaves a mark in the log, few enough that a target lasts several
                // epochs.
                rof_rounds_per_min: 12.0,
                max_range_m: 3000.0,
                dispersion_mrad: 0.5,
                p_kill_given_hit: 0.9,
                ..Default::default()
            },
        )]),
        ..Libraries::with_terrain(scenario_params())
    };
    Sim::new(&scn, &libs, 5).expect("fixture builds")
}

/// Distinct shooters, and distinct targets, in the epoch just run.
fn epoch_slice(sim: &Sim, from: usize) -> (BTreeSet<usize>, BTreeSet<sim_core::sim::FireTarget>) {
    let slice = &sim.fire_events()[from..];
    (
        slice.iter().map(|e| e.shooter).collect(),
        slice.iter().map(|e| e.target).collect(),
    )
}

// The property the cap was getting wrong. Three guns, one target: all three should engage.
// The third is worth little — the discount says so — but little is more than the nothing a
// cap of one or two delivered.
#[test]
fn v68_no_gun_idles_when_there_is_only_one_target() {
    let mut sim = guns_against(3, 1);
    for _ in 0..8 {
        let before = sim.fire_events().len();
        sim.run_until(sim.time_s() + 10.0);
        let (shooters, _) = epoch_slice(&sim, before);
        if shooters.len() == 3 {
            return;
        }
    }
    panic!("no epoch had all three guns engaging the single target; a cap would idle two");
}

// The other direction, and the reason the discount is not simply dropped as well: with a
// target each, the guns must still take one each rather than stacking. If the geometric
// term stopped doing its job, this is the test that would catch it.
#[test]
fn v68_fire_still_spreads_when_there_is_a_target_for_everyone() {
    let mut sim = guns_against(3, 3);
    for _ in 0..8 {
        let before = sim.fire_events().len();
        sim.run_until(sim.time_s() + 10.0);
        let (shooters, targets) = epoch_slice(&sim, before);
        // Only judge an epoch in which all three guns fired and all three targets are still
        // alive — once one dies the survivors are correctly free to double up.
        let all_alive = sim.units()[3..6].iter().all(|u| u.elements > 0);
        if shooters.len() == 3 && all_alive {
            assert_eq!(
                targets.len(),
                3,
                "three guns with three live targets should cover three, not stack; \
                 got {targets:?}"
            );
            return;
        }
    }
    panic!("no epoch had all three guns firing with three live targets");
}
