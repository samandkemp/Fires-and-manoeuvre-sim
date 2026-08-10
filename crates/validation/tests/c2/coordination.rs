//! V59 — C2-coordinated air defence. `docs/DESIGN.md` §11.
//!
//! The fixture reproduces the failure mode deterministically. Three single-channel
//! batteries sit in a line; the three drones are placed so that **one of them is the
//! nearest to all three**. Nearest-first therefore sends every battery at that same drone
//! while the other two fly on untouched — the classic point-defence failure, and entirely
//! a coordination problem: same batteries, same envelopes, same magazines.
//!
//! A C2 post covering the line is the only thing that changes.

use sim_core::air_defence::AirDefenceType;
use sim_core::c2::C2Type;
use sim_core::scenario::{Libraries, Scenario};
use sim_core::sensing::{Modality, SensorType};
use sim_core::sim::Sim;
use std::collections::BTreeMap;
use validation::scenario_params;

/// The battery line, metres.
const BASE_X: f32 = 1000.0;
const BASE_Y: f32 = 1500.0;
/// How far east the drones sit.
const STANDOFF: f32 = 1200.0;

fn libraries() -> Libraries {
    Libraries {
        sensors: BTreeMap::from([(
            "radar".to_owned(),
            SensorType {
                modality: Modality::Optical,
                mount_height_m: 4.0,
                max_range_m: 8000.0,
                // Acquire almost immediately: this gate is about *allocation*, not about
                // the §9.5 cueing timeline, which V50 already covers.
                lambda0_per_s: 8.0,
                range_half_m: 8000.0,
                range_exponent: 2.0,
                for_width_deg: None,
            },
        )]),
        air: BTreeMap::from([(
            "drone".to_owned(),
            sim_core::air::AirType {
                height_m: 2.0,
                cruise_speed_m_s: 0.0, // stationary: the geometry must not drift
                signature: BTreeMap::from([("optical".to_owned(), 0.9)]),
                ..Default::default()
            },
        )]),
        air_defence: BTreeMap::from([(
            "gun".to_owned(),
            AirDefenceType {
                max_range_m: 4000.0,
                max_alt_m: 3000.0,
                requires_los: false,
                // Long enough that every battery has acquired every drone before any of
                // them may open. Without it, acquisition *order* decides who engages
                // what and the gate measures detection luck instead of allocation.
                reaction_time_s: 25.0,
                cue_latency_s: 0.0,
                magazine: 0,
                channels: 1, // one engagement each: the resource being contested
                sensor: Some("radar".to_owned()),
                engagement: sim_core::air_defence::AdEngagement::Gun {
                    // Chosen so the *payoff* is meaningful (P(kill) over the engagement
                    // window is ~0.5) while almost nothing actually dies in the 40 s
                    // observed — expected attrition here is under a tenth of an airframe.
                    // Both halves matter: too lethal and the batteries re-open on
                    // survivors, so the reading is of attrition rather than allocation;
                    // too feeble and every pairing scores alike, so the diminishing-return
                    // discount stops separating "cover another drone" from "pile on".
                    kill_rate_per_s: 5.0e-4,
                },
                ..Default::default()
            },
        )]),
        c2: BTreeMap::from([(
            "post".to_owned(),
            C2Type {
                coordination_range_m: 3000.0,
                ..Default::default()
            },
        )]),
        ..Libraries::with_terrain(scenario_params())
    }
}

/// Three batteries, three drones at equal range, and optionally a C2 post over them.
fn raid(with_c2: bool) -> Sim {
    let mut s = format!(
        r#"
        name = "ad-c2"
        default_seed = 3
        [sim]
        dt_s = 1.0
        epoch_s = 10.0
        [terrain]
        cell_size_m = 10.0
        width_cells = 400
        height_cells = 300
        [terrain.source.flat]
        elevation_m = 0.0
        [[blue.air_defence]]
        id = "gun-a"
        type = "gun"
        pos = [{BASE_X}, {}]
        [[blue.air_defence]]
        id = "gun-b"
        type = "gun"
        pos = [{BASE_X}, {BASE_Y}]
        [[blue.air_defence]]
        id = "gun-c"
        type = "gun"
        pos = [{BASE_X}, {}]
    "#,
        BASE_Y - 500.0,
        BASE_Y + 500.0
    );
    if with_c2 {
        s.push_str(&format!(
            r#"
        [[blue.c2]]
        id = "cp"
        type = "post"
        pos = [{BASE_X}, {BASE_Y}]
        "#
        ));
    }
    // Drone 0 sits level with the centre battery, so it is the nearest to ALL three;
    // the flankers are far enough out that no battery prefers them.
    for (i, dy) in [0.0f32, 1500.0, -1500.0].iter().enumerate() {
        s.push_str(&format!(
            r#"
        [[red.air]]
        id = "uas-{i}"
        type = "drone"
        pos = [{}, {}]
        altitude_m = 300.0
        heading_deg = 180.0
        "#,
            BASE_X + STANDOFF,
            BASE_Y + dy
        ));
    }
    let scn = Scenario::from_toml_str(&s).unwrap();
    Sim::new(&scn, &libraries(), 3).unwrap()
}

/// Which airframe each battery is currently engaging, in battery order.
fn engaged(sim: &Sim) -> Vec<Option<usize>> {
    sim.air_defence()
        .iter()
        .map(|ad| ad.engagements.first().map(|e| e.target))
        .collect()
}

/// How many *distinct* airframes the defence has under engagement.
fn distinct(sim: &Sim) -> usize {
    let mut t: Vec<usize> = engaged(sim).into_iter().flatten().collect();
    t.sort_unstable();
    t.dedup();
    t.len()
}

// V59 (the headline): uncoordinated batteries all pile onto one drone; a C2 post makes
// them split the raid. Same batteries, same envelopes, same magazines — the only
// difference is whether anything is tying them together.
#[test]
fn v59_c2_makes_air_defence_split_the_raid() {
    let mut alone = raid(false);
    alone.run_until(40.0);
    assert_eq!(
        distinct(&alone),
        1,
        "without C2, nearest-first should send every battery at the same drone; got {:?}",
        engaged(&alone)
    );

    let mut coordinated = raid(true);
    coordinated.run_until(40.0);
    assert_eq!(
        distinct(&coordinated),
        3,
        "with C2, three batteries should cover three drones; got {:?}",
        engaged(&coordinated)
    );
}

// V59 (identity half): a scenario with no C2 post must behave exactly as it did before
// C2 existed. This is the §7.4 discipline — the coordinated path is *added*, never
// substituted, so V50/V51/V52 cannot move.
#[test]
fn v59_without_a_post_nothing_changes() {
    let mut a = raid(false);
    let mut b = raid(false);
    a.run_until(40.0);
    b.run_until(40.0);
    assert_eq!(
        a.air_defence_events(),
        b.air_defence_events(),
        "the uncoordinated path must stay deterministic"
    );
    // Every battery still on the lowest-index drone, which is the pre-C2 rule exactly.
    assert!(
        engaged(&a).iter().flatten().all(|&t| t == 0),
        "nearest-first with tied ranges must break on index: {:?}",
        engaged(&a)
    );
}

// V59 (decoherence): a *dead* post coordinates nothing. Destroying it costs no battery,
// no magazine and no envelope — the defence simply stops splitting the raid. That is the
// property which makes a command post worth attacking, and the hook SEAD will pull.
#[test]
fn v59_killing_the_post_decoheres_the_defence() {
    let mut sim = raid(true);
    // Killed before anything opens, so what is measured is the allocation decision itself
    // rather than how long stale engagements happen to persist.
    sim.remove_c2(0);
    sim.run_until(40.0);

    assert_eq!(
        distinct(&sim),
        1,
        "a dead post must coordinate nothing; got {:?}",
        engaged(&sim)
    );
    assert_eq!(
        sim.air_defence().len(),
        3,
        "killing the post must not remove a single battery — only the coordination"
    );
    // Tombstoned, not removed — the same discipline as every other asset (V54), so any
    // index already recorded against it still resolves.
    assert_eq!(
        sim.c2().len(),
        1,
        "the post should be tombstoned, not deleted"
    );
    assert!(!sim.c2()[0].alive(), "the post should be marked dead");
}

// --- Two sides, two posts -------------------------------------------------------------
//
// Coordination is a relationship between a post and *its own* batteries. Nothing about the
// enemy having a post of its own should reach into my fire plan — but pooling every
// coordinated battery into one assignment made it do exactly that, scoring the whole field
// under whichever side happened to be first in the list.
//
// The fixture separates doctrine from payoff: each side faces a `juicy` drone (value 3.0)
// and a `cheap` one (0.5), and each side's doctrine names the **cheap** one. Following
// orders is therefore unambiguously different from taking the better shot, so a battery
// scored under the enemy's doctrine — which names neither of its targets — falls through to
// the payoff and takes the juicy drone instead.

/// Where each side's battery and post sit, metres.
const BLUE_AD: (f32, f32) = (1000.0, 1500.0);
const RED_AD: (f32, f32) = (3400.0, 1500.0);

/// Libraries for the two-sided fixture: the V59 gun and radar, plus two drone types that
/// differ **only** in what they are worth, so the doctrine test cannot be passed by luck.
fn valued_libraries() -> Libraries {
    let drone = |value: f32| sim_core::air::AirType {
        height_m: 2.0,
        cruise_speed_m_s: 0.0,
        signature: BTreeMap::from([("optical".to_owned(), 0.9)]),
        value: Some(value),
        ..Default::default()
    };
    let base = libraries();
    Libraries {
        air: BTreeMap::from([
            ("juicy".to_owned(), drone(3.0)),
            ("cheap".to_owned(), drone(0.5)),
        ]),
        ..base
    }
}

/// A mirrored engagement: one single-channel battery per side, each with two enemy drones
/// in reach, each side's doctrine naming the enemy's *cheap* drone. `blue_post` / `red_post`
/// select which sides are coordinated, which is what the independence check varies.
fn two_sided(blue_post: bool, red_post: bool) -> Sim {
    let mut s = format!(
        r#"
        name = "two-sided-c2"
        default_seed = 3
        [sim]
        dt_s = 1.0
        epoch_s = 10.0
        [terrain]
        cell_size_m = 10.0
        width_cells = 500
        height_cells = 300
        [terrain.source.flat]
        elevation_m = 0.0

        [blue.doctrine]
        priority = ["cheap-r"]

        [[blue.air_defence]]
        id = "blue-gun"
        type = "gun"
        pos = [{}, {}]

        [red.doctrine]
        priority = ["cheap-b"]

        [[red.air_defence]]
        id = "red-gun"
        type = "gun"
        pos = [{}, {}]
    "#,
        BLUE_AD.0, BLUE_AD.1, RED_AD.0, RED_AD.1
    );
    for (side, id, post) in [("blue", "blue-cp", blue_post), ("red", "red-cp", red_post)] {
        if !post {
            continue;
        }
        let at = if side == "blue" { BLUE_AD } else { RED_AD };
        s.push_str(&format!(
            r#"
        [[{side}.c2]]
        id = "{id}"
        type = "post"
        pos = [{}, {}]
        "#,
            at.0, at.1
        ));
    }
    // Each side's drones sit between the two batteries, in reach of the enemy's gun only.
    // Blue's are placed first, so they take the lower air indices — which is what put the
    // Blue battery first in the pooled list and made Blue's doctrine the one that won.
    for (side, suffix, y) in [("blue", 'b', 1000.0f32), ("red", 'r', 2000.0)] {
        for kind in ["juicy", "cheap"] {
            s.push_str(&format!(
                r#"
        [[{side}.air]]
        id = "{kind}-{suffix}"
        type = "{kind}"
        pos = [2200.0, {y}]
        altitude_m = 300.0
        heading_deg = 180.0
        "#
            ));
        }
    }
    let scn = Scenario::from_toml_str(&s).unwrap();
    Sim::new(&scn, &valued_libraries(), 3).unwrap()
}

/// The id of the airframe battery `id` is engaging, if any.
fn engaging_id(sim: &Sim, id: &str) -> Option<String> {
    let ad = sim.air_defence().iter().find(|d| d.id == id)?;
    let target = ad.engagements.first()?.target;
    Some(sim.air()[target].id.clone())
}

// V59 (two sides): a post coordinates its own side and nobody else's.
//
// Both halves matter, and the second is the general property. The first names the symptom:
// each battery must follow the fire plan it was given. The second says *why* that has to
// hold however the doctrines are set — my decision cannot depend on whether the enemy
// happens to have a command post, because his post is not talking to my battery.
#[test]
fn v59_each_side_coordinates_only_its_own_batteries() {
    let mut both = two_sided(true, true);
    both.run_until(40.0);

    // 1. The symptom: strict doctrine is followed, against the better shot, on both sides.
    assert_eq!(
        engaging_id(&both, "blue-gun").as_deref(),
        Some("cheap-r"),
        "Blue's battery must follow Blue's fire plan"
    );
    assert_eq!(
        engaging_id(&both, "red-gun").as_deref(),
        Some("cheap-b"),
        "Red's battery must follow RED's fire plan, not whatever Blue was told"
    );

    // 2. The property: the enemy's coordination does not reach into mine. Each side's
    // choice must be the one it would make with the other side's post removed entirely.
    let mut blue_alone = two_sided(true, false);
    let mut red_alone = two_sided(false, true);
    blue_alone.run_until(40.0);
    red_alone.run_until(40.0);

    assert_eq!(
        engaging_id(&both, "blue-gun"),
        engaging_id(&blue_alone, "blue-gun"),
        "Red gaining a post changed Blue's decision"
    );
    assert_eq!(
        engaging_id(&both, "red-gun"),
        engaging_id(&red_alone, "red-gun"),
        "Blue gaining a post changed Red's decision"
    );
}
