//! V66 — the kill chain: what a side has been *told* to shoot first.
//! `docs/DESIGN.md` §13.
//!
//! §10.2 allocates fire by maximising `P(kill) × value`, which is what an omniscient
//! optimiser would do. Real crews are not omniscient optimisers: they do not hold a
//! kill-probability table, they hold orders, and they follow them whether or not the shot
//! is a good one.
//!
//! So the fixture is built to make the two rules disagree as loudly as possible. One gun,
//! two targets:
//!
//! - a **tank** close in, which it hits about half the time and which scores highly on the
//!   derived threat value — the payoff-optimal choice by a wide margin;
//! - a **SAM** near the edge of range, which it hits about three times in a hundred.
//!
//! Undirected, the gun takes the tank every time. Told `priority = ["air_defence"]`, it
//! must take the SAM — a fifteen-fold worse shot — because that is what it was told to do.
//! Nothing in between would prove the ordering is actually strict.

use sim_core::air::AirType;
use sim_core::air_defence::{AdEngagement, AirDefenceType};
use sim_core::c2::C2Type;
use sim_core::fires::{WeaponClass, WeaponType};
use sim_core::scenario::{Libraries, Scenario};
use sim_core::sensing::{Modality, SensorType, UnitType};
use sim_core::sim::{FireTarget, Sim};
use std::collections::BTreeMap;
use validation::scenario_params;

const GUN_X: f32 = 200.0;
const LANE_Y: f32 = 500.0;
/// Close: about a 46% hit. Far: about 3%. The gap is the whole point.
const TANK_X: f32 = 700.0;
const SAM_X: f32 = 2800.0;

fn libraries() -> Libraries {
    Libraries {
        sensors: BTreeMap::from([(
            "radar".to_owned(),
            SensorType {
                modality: Modality::Optical,
                mount_height_m: 4.0,
                max_range_m: 9000.0,
                lambda0_per_s: 4.0,
                range_half_m: 9000.0,
                range_exponent: 2.0,
                for_width_deg: None,
            },
        )]),
        units: BTreeMap::from([
            (
                "gun".to_owned(),
                UnitType {
                    height_m: 2.5,
                    silhouette_width_m: 3.0,
                    element_count: 1,
                    signature: BTreeMap::from([("optical".to_owned(), 0.5)]),
                    weapon: Some("cannon".to_owned()),
                    role: Some("artillery".to_owned()),
                    ..Default::default()
                },
            ),
            (
                "tank".to_owned(),
                UnitType {
                    height_m: 2.8,
                    silhouette_width_m: 3.2,
                    element_count: 4,
                    signature: BTreeMap::from([("optical".to_owned(), 0.8)]),
                    // Armed, so it earns a high derived threat value and is what the
                    // payoff would pick unprompted.
                    weapon: Some("cannon".to_owned()),
                    role: Some("armour".to_owned()),
                    ..Default::default()
                },
            ),
        ]),
        weapons: BTreeMap::from([(
            "cannon".to_owned(),
            WeaponType {
                class: WeaponClass::Direct,
                rof_rounds_per_min: 30.0,
                max_range_m: 3000.0,
                // Coarse on purpose: it is what opens the gap between the near and far
                // shot far enough that no tie-break could explain the result.
                dispersion_mrad: 3.0,
                p_kill_given_hit: 0.9,
                ..Default::default()
            },
        )]),
        air_defence: BTreeMap::from([(
            "sam".to_owned(),
            AirDefenceType {
                max_range_m: 8000.0,
                max_alt_m: 4000.0,
                element_count: 4,
                height_m: 3.0,
                silhouette_width_m: 4.0,
                sensor: Some("radar".to_owned()),
                role: Some("sam".to_owned()),
                engagement: AdEngagement::Gun {
                    kill_rate_per_s: 0.0,
                },
                ..Default::default()
            },
        )]),
        air: BTreeMap::from([
            (
                "recce".to_owned(),
                AirType {
                    height_m: 2.0,
                    cruise_speed_m_s: 0.0,
                    signature: BTreeMap::from([("optical".to_owned(), 0.9)]),
                    role: Some("recce".to_owned()),
                    ..Default::default()
                },
            ),
            (
                "striker".to_owned(),
                AirType {
                    height_m: 2.0,
                    cruise_speed_m_s: 0.0,
                    signature: BTreeMap::from([("optical".to_owned(), 0.9)]),
                    role: Some("strike".to_owned()),
                    ..Default::default()
                },
            ),
        ]),
        c2: BTreeMap::from([("post".to_owned(), C2Type::default())]),
        ..Libraries::with_terrain(scenario_params())
    }
}

/// One Blue gun; a near Red tank and a far Red SAM. `doctrine` is inserted verbatim.
fn duel(doctrine: &str) -> Result<Sim, sim_core::scenario::ScenarioError> {
    let scn = Scenario::from_toml_str(&format!(
        r#"
        name = "doctrine"
        default_seed = 6
        [sim]
        dt_s = 1.0
        epoch_s = 10.0
        [terrain]
        cell_size_m = 10.0
        width_cells = 400
        height_cells = 120
        [terrain.source.flat]
        elevation_m = 0.0
        [[blue.units]]
        id = "gun-a"
        type = "gun"
        pos = [{GUN_X}, {LANE_Y}]
        [[red.units]]
        id = "tank-1"
        type = "tank"
        pos = [{TANK_X}, {LANE_Y}]
        [[red.air_defence]]
        id = "sam-1"
        type = "sam"
        pos = [{SAM_X}, {LANE_Y}]
        {doctrine}
    "#
    ))?;
    Sim::new(&scn, &libraries(), 6)
}

/// What **the Blue gun** engaged in the first epoch.
///
/// Filtered to shooter 0 deliberately: the Red tank is armed — it has to be, or it would
/// not earn the high derived value that makes it the payoff-optimal choice — so it shoots
/// back, and its return fire would otherwise appear in the same log. Blue's gun is unit 0,
/// the tank unit 1, so an unfiltered read of "who was hit" mixes the two sides' decisions.
///
/// The first epoch only: both sides allocate against the same board and fire in shooter
/// index order, so Blue's volley always happens even if the tank then destroys it.
fn engaged(sim: &mut Sim) -> Vec<FireTarget> {
    sim.run_until(10.0);
    let mut t: Vec<FireTarget> = sim
        .fire_events()
        .iter()
        .filter(|e| e.shooter == 0)
        .map(|e| e.target)
        .collect();
    t.sort_unstable();
    t.dedup();
    t
}

// V66 (headline): a declared priority is *followed*, not weighed. The gun abandons a 46%
// shot at the tank for a 3% shot at the SAM, because that is the order it has.
#[test]
fn v66_strict_doctrine_overrides_a_better_shot() {
    let mut undirected = duel("").expect("valid");
    assert_eq!(
        engaged(&mut undirected),
        vec![FireTarget::Unit(1)],
        "unprompted, the payoff must prefer the near, high-value tank — if it does not, \
         the fixture is not posing the question"
    );

    let mut directed = duel(
        r#"
        [blue.doctrine]
        priority = ["air_defence"]
    "#,
    )
    .expect("valid");
    assert_eq!(
        engaged(&mut directed),
        vec![FireTarget::AirDefence(0)],
        "told to engage air defence first, the gun must do so despite the worse shot"
    );
}

// V66 (weighted half): the other mode is a thumb on the scale, not an instruction. With the
// same priority weighted rather than strict, a shot fifteen times better still wins — which
// is exactly the difference the two modes exist to express.
#[test]
fn v66_weighted_doctrine_only_biases() {
    let mut weighted = duel(
        r#"
        [blue.doctrine]
        priority = ["air_defence"]
        mode = "weighted"
    "#,
    )
    .expect("valid");
    assert_eq!(
        engaged(&mut weighted),
        vec![FireTarget::Unit(1)],
        "weighted doctrine must not overturn a fifteen-fold better shot"
    );
}

// V66 (identity half, §7.4): a side with no doctrine and no orders allocates exactly as it
// did before any of this existed. Nothing in the fire log may move.
#[test]
fn v66_no_doctrine_is_an_exact_identity() {
    let log = || -> Vec<(usize, FireTarget, u32)> {
        let mut sim = duel("").expect("valid");
        sim.run_until(120.0);
        sim.fire_events()
            .iter()
            .map(|e| (e.shooter, e.target, e.casualties))
            .collect()
    };
    let a = log();
    assert_eq!(a, log(), "must reproduce");
    assert!(!a.is_empty(), "the fixture must actually shoot");
}

// V66 (orders half): a directly ordered engagement bypasses the assignment entirely, so a
// gate can state a pairing as a fact about the run rather than a likely outcome of it.
#[test]
fn v66_an_order_pins_a_pairing() {
    let mut ordered = duel(
        r#"
        [[blue.orders]]
        shooter = "gun-a"
        target = "sam-1"
    "#,
    )
    .expect("valid");
    assert_eq!(
        engaged(&mut ordered),
        vec![FireTarget::AirDefence(0)],
        "an ordered shooter is not choosing"
    );

    // An order against a destroyed target lapses and the shooter rejoins the problem — a
    // standing order does not make a crew fire at a wreck.
    let mut lapsed = duel(
        r#"
        [[blue.orders]]
        shooter = "gun-a"
        target = "sam-1"
    "#,
    )
    .expect("valid");
    lapsed.remove_air_defence(0);
    assert_eq!(
        engaged(&mut lapsed),
        vec![FireTarget::Unit(1)],
        "with its ordered target gone, the gun must fall back to the assignment"
    );
}

// V66 (vocabulary half): a priority entry that names nothing is a load error, not an empty
// tier. Same reasoning as the schema's `deny_unknown_fields` — a tier matching nothing
// fails silently, and the run then answers a different question than the one asked.
#[test]
fn v66_a_priority_naming_nothing_is_a_load_error() {
    // `Sim` is not Debug, so unwrap the Result by hand rather than via `expect_err`.
    let Err(err) = duel(
        r#"
        [blue.doctrine]
        priority = ["artilery"]
    "#,
    ) else {
        panic!("a typo must not load");
    };
    let msg = format!("{err}");
    assert!(
        msg.contains("artilery"),
        "must name the offending entry: {msg}"
    );
    assert!(
        msg.contains("air_defence") || msg.contains("armour"),
        "and say what would have worked: {msg}"
    );

    // An id, a role and a class must all be accepted.
    for good in ["sam-1", "sam", "air_defence", "armour", "unit"] {
        duel(&format!(
            r#"
            [blue.doctrine]
            priority = ["{good}"]
        "#
        ))
        .map(|_| ())
        .unwrap_or_else(|e| panic!("'{good}' should be a valid priority entry: {e}"));
    }

    // And an order naming a shooter that is not there.
    let Err(err) = duel(
        r#"
        [[blue.orders]]
        shooter = "gun-z"
        target = "sam-1"
    "#,
    ) else {
        panic!("an unknown shooter must not load");
    };
    assert!(format!("{err}").contains("gun-z"));
}

// V66 (air-defence half): the same doctrine drives counter-air. One battery, a near recce
// drone and a far strike drone — nearest-first takes the recce, doctrine takes the striker.
#[test]
fn v66_air_defence_follows_the_same_doctrine() {
    let raid = |doctrine: &str| -> Sim {
        let scn = Scenario::from_toml_str(&format!(
            r#"
            name = "doctrine-air"
            default_seed = 6
            [sim]
            dt_s = 1.0
            epoch_s = 10.0
            [terrain]
            cell_size_m = 10.0
            width_cells = 400
            height_cells = 120
            [terrain.source.flat]
            elevation_m = 0.0
            [[blue.air_defence]]
            id = "sam-1"
            type = "sam"
            pos = [{GUN_X}, {LANE_Y}]
            [[red.air]]
            id = "uas-recce"
            type = "recce"
            pos = [900.0, {LANE_Y}]
            altitude_m = 300.0
            heading_deg = 180.0
            [[red.air]]
            id = "uas-strike"
            type = "striker"
            pos = [2200.0, {LANE_Y}]
            altitude_m = 300.0
            heading_deg = 180.0
            {doctrine}
        "#
        ))
        .unwrap();
        Sim::new(&scn, &libraries(), 6).unwrap()
    };

    let engaging = |sim: &mut Sim| -> Vec<usize> {
        sim.run_until(30.0);
        sim.air_defence()[0]
            .engagements
            .iter()
            .map(|e| e.target)
            .collect()
    };

    let mut nearest = raid("");
    assert_eq!(
        engaging(&mut nearest),
        vec![0],
        "undirected, the battery takes the nearer drone (index 0, the recce)"
    );

    let mut directed = raid(
        r#"
        [blue.doctrine]
        priority = ["strike"]
    "#,
    );
    assert_eq!(
        engaging(&mut directed),
        vec![1],
        "told to engage strike first, it must take the further striker"
    );
}

// ---------------------------------------------------------------------------------------
// V66 (eligibility and locks). LOS and range **block** a pairing rather than merely
// lowering its score, and a shooter that takes a target holds it. `docs/DESIGN.md` Â§13.4.
// ---------------------------------------------------------------------------------------

/// Indirect-fire fixture for the lock tests: expected damage per round does not vary with
/// range, so the payoff is proportional to value and value to remaining elements â€” which is
/// what makes a fresh solve want to switch part-way through.
fn lock_libraries() -> Libraries {
    let mut libs = libraries();
    libs.weapons.insert(
        "gun_indirect".to_owned(),
        WeaponType {
            class: WeaponClass::Indirect,
            rof_rounds_per_min: 12.0,
            max_range_m: 8000.0,
            cep_m: 30.0,
            lethal_radius_m: 35.0,
            ..Default::default()
        },
    );
    libs.units.insert(
        "howitzer".to_owned(),
        UnitType {
            height_m: 2.5,
            element_count: 1,
            signature: BTreeMap::from([("optical".to_owned(), 0.5)]),
            weapon: Some("gun_indirect".to_owned()),
            ..Default::default()
        },
    );
    for (name, elements) in [("small_target", 4u32), ("big_target", 8)] {
        libs.units.insert(
            name.to_owned(),
            UnitType {
                height_m: 2.5,
                element_count: elements,
                signature: BTreeMap::from([("optical".to_owned(), 0.9)]),
                ..Default::default()
            },
        );
    }
    libs
}

/// A ridge between the gun and the far target, so direct fire is blocked to one and clear
/// to the other.
fn masked_duel(doctrine: &str) -> Sim {
    let scn = Scenario::from_toml_str(&format!(
        r#"
        name = "doctrine-mask"
        default_seed = 6
        [sim]
        dt_s = 1.0
        epoch_s = 10.0
        [terrain]
        cell_size_m = 10.0
        width_cells = 400
        height_cells = 120
        [terrain.source.layers]
        base = {{ flat = {{ elevation_m = 0.0 }} }}
        # A north-south ridge through the map centre (x = 2000 m): the gun and the armour
        # are west of it, the SAM east, so one is in view and the other is not.
        [[terrain.source.layers.apply]]
        ridge = {{ bearing_deg = 90.0, crest_m = 90.0, width_m = 300.0, offset_m = 0.0 }}
        [[blue.units]]
        id = "gun-a"
        type = "gun"
        pos = [{GUN_X}, {LANE_Y}]
        [[red.units]]
        id = "tank-1"
        type = "tank"
        pos = [{TANK_X}, {LANE_Y}]
        [[red.air_defence]]
        id = "sam-1"
        type = "sam"
        pos = [{SAM_X}, {LANE_Y}]
        {doctrine}
    "#
    ))
    .unwrap();
    Sim::new(&scn, &libraries(), 6).unwrap()
}

// The property that matters most: an unreachable priority target does not hold a shooter
// hostage. A ridge hides the SAM, so a gun ordered to engage air defence first finds
// nothing it can shoot in that tier and falls through â€” rather than idling, or worse,
// spending the epoch on something it cannot touch while a live threat goes unengaged.
#[test]
fn v66_an_unreachable_priority_falls_through_to_the_next_tier() {
    let mut masked = masked_duel(
        r#"
        [blue.doctrine]
        priority = ["air_defence", "armour"]
    "#,
    );
    // The fixture only means anything if the ridge really does block the SAM.
    assert!(
        !sim_core::los::visible(
            masked.terrain(),
            glam::Vec2::new(GUN_X, LANE_Y),
            2.0,
            glam::Vec2::new(SAM_X, LANE_Y),
            3.0
        ),
        "the ridge must actually mask the SAM, or this tests nothing"
    );
    assert_eq!(
        engaged(&mut masked),
        vec![FireTarget::Unit(1)],
        "with its priority target masked, the gun must engage the armour it can see"
    );
}

// An ordered engagement obeys the same rule: it stands while reachable and lapses while
// not, so an order can never leave a gun facing a hill.
#[test]
fn v66_an_unreachable_order_falls_back_to_the_assignment() {
    let mut masked = masked_duel(
        r#"
        [[blue.orders]]
        shooter = "gun-a"
        target = "sam-1"
    "#,
    );
    assert_eq!(
        engaged(&mut masked),
        vec![FireTarget::Unit(1)],
        "an order against a masked target must lapse, not waste the epoch"
    );
}

// A lock is held until the target is dead or unengageable, not re-decided every epoch.
//
// The fixture makes a fresh solve *want* to switch: indirect fire's expected damage per
// round does not vary with range, so the payoff is proportional to remaining elements. The
// gun opens on the larger unit; once it has whittled that below the other, an unlocked
// shooter would move on. A locked one finishes the job.
#[test]
fn v66_a_lock_is_held_until_the_target_is_finished() {
    let scn = Scenario::from_toml_str(&format!(
        r#"
        name = "doctrine-lock"
        default_seed = 6
        [sim]
        dt_s = 1.0
        epoch_s = 10.0
        max_shooters_per_target = 1
        [terrain]
        cell_size_m = 10.0
        width_cells = 400
        height_cells = 120
        [terrain.source.flat]
        elevation_m = 0.0
        [[blue.units]]
        id = "how-a"
        type = "howitzer"
        pos = [{GUN_X}, {LANE_Y}]
        [[blue.sensors]]
        id = "obs"
        type = "radar"
        pos = [{GUN_X}, {LANE_Y}]
        [[red.units]]
        id = "small"
        type = "small_target"
        pos = [3000.0, {LANE_Y}]
        [[red.units]]
        id = "big"
        type = "big_target"
        pos = [3200.0, {LANE_Y}]
    "#
    ))
    .unwrap();
    let mut sim = Sim::new(&scn, &lock_libraries(), 6).unwrap();

    sim.run_until(10.0);
    let locked = sim.units()[0]
        .engaging
        .expect("the gun must open on something");
    assert_eq!(
        locked,
        FireTarget::Unit(2),
        "the bigger unit is worth more, so the payoff opens there"
    );

    let mut switched_early = false;
    for _ in 0..40 {
        sim.run_until(sim.time_s() + 10.0);
        let big_alive = sim.units()[2].elements > 0;
        if big_alive && sim.units()[0].engaging.is_some_and(|t| t != locked) {
            switched_early = true;
            break;
        }
        if !big_alive {
            break;
        }
    }
    assert!(
        !switched_early,
        "a lock must hold while its target is alive and engageable"
    );
    assert_eq!(
        sim.units()[2].elements,
        0,
        "the fixture must actually finish the target within the window"
    );
    assert!(
        sim.units()[1].elements > 0,
        "and must not have finished the other one too, or the lock proved nothing"
    );
}

// The other half of a lock: it is *lost* the moment the target cannot be engaged, so a gun
// is never left committed to something it can no longer touch.
#[test]
fn v66_a_lock_is_lost_when_the_target_becomes_unengageable() {
    let mut sim = masked_duel("");
    sim.run_until(10.0);
    assert_eq!(
        sim.units()[0].engaging,
        Some(FireTarget::Unit(1)),
        "the gun should be locked onto the armour"
    );
    sim.remove_unit(1);
    sim.run_until(30.0);
    assert_eq!(
        sim.units()[0].engaging,
        None,
        "a lock on a destroyed target must be released"
    );
}

// A lock still occupies a slot. Without that, `max_shooters_per_target` would apply only to
// shooters that happened to be re-deciding, and a target could accumulate any number of
// locked guns â€” the cap V56 exists to enforce, quietly bypassed.
#[test]
fn v66_a_lock_counts_against_the_overkill_cap() {
    let scn = Scenario::from_toml_str(&format!(
        r#"
        name = "doctrine-cap"
        default_seed = 6
        [sim]
        dt_s = 1.0
        epoch_s = 10.0
        max_shooters_per_target = 1
        [terrain]
        cell_size_m = 10.0
        width_cells = 400
        height_cells = 120
        [terrain.source.flat]
        elevation_m = 0.0
        [[blue.units]]
        id = "how-a"
        type = "howitzer"
        pos = [{GUN_X}, 400.0]
        [[blue.units]]
        id = "how-b"
        type = "howitzer"
        pos = [{GUN_X}, 600.0]
        [[blue.sensors]]
        id = "obs"
        type = "radar"
        pos = [{GUN_X}, {LANE_Y}]
        [[red.units]]
        id = "big"
        type = "big_target"
        pos = [3200.0, {LANE_Y}]
        [[red.units]]
        id = "small"
        type = "small_target"
        pos = [3000.0, {LANE_Y}]
    "#
    ))
    .unwrap();
    let mut sim = Sim::new(&scn, &lock_libraries(), 6).unwrap();

    for _ in 0..8 {
        sim.run_until(sim.time_s() + 10.0);
        for t in [FireTarget::Unit(2), FireTarget::Unit(3)] {
            let on_it = sim.units().iter().filter(|u| u.engaging == Some(t)).count();
            assert!(
                on_it <= 1,
                "cap is 1, but {on_it} shooters are locked onto {t:?} at t = {}",
                sim.time_s()
            );
        }
    }
}
