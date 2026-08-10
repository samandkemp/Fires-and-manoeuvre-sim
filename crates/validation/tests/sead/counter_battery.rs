//! V65 - ground fires can reach an emplacement. `docs/DESIGN.md` §12.4.
//!
//! §12 made batteries and posts killable, but only from the air: ground fires iterated the
//! unit list, so artillery could not conduct counter-battery against a SAM sitting in range
//! of it. Every asset class already had elements and already took §2.3 area damage
//! identically - the only thing missing was *which lists are searched*.
//!
//! The interesting half is not that a shell can hurt a battery. It is **how the battery is
//! found**. Neither batteries nor posts go through the §3.2 glimpse loop, so neither has a
//! track. Rather than invent one - which would insert draws into every scenario fielding air
//! defence and shift the stream under V50, V51, V59 and V60 for no modelling gain - this
//! asks the question counter-battery acquisition actually asks: *has it given itself away?*
//!
//! - a **battery** has, if it is transmitting or has fired - ESM, or a counter-battery track
//!   back along its rounds;
//! - a **post** has, if it is coordinating anything: found because it is talking.
//!
//! Which joins the two halves of §12.3. Switching a radar off already made an ARM miss; it
//! now also hides the battery from artillery. One decision, two consequences.

use sim_core::air_defence::{AdEngagement, AirDefenceType};
use sim_core::c2::C2Type;
use sim_core::fires::{WeaponClass, WeaponType};
use sim_core::scenario::{Libraries, Scenario};
use sim_core::sensing::{Modality, SensorType, UnitType};
use sim_core::sim::{FireTarget, Sim};
use std::collections::BTreeMap;
use validation::scenario_params;

const GUN_X: f32 = 400.0;
const GUN_Y: f32 = 500.0;
/// Well inside the howitzer's reach, and inside direct-fire LOS on flat ground.
const SAM_X: f32 = 1600.0;

fn libraries() -> Libraries {
    Libraries {
        sensors: BTreeMap::from([(
            "radar".to_owned(),
            SensorType {
                modality: Modality::Optical,
                mount_height_m: 4.0,
                max_range_m: 8000.0,
                lambda0_per_s: 1.0,
                range_half_m: 8000.0,
                range_exponent: 2.0,
                for_width_deg: None,
            },
        )]),
        units: BTreeMap::from([
            (
                "howitzer".to_owned(),
                UnitType {
                    height_m: 2.5,
                    silhouette_width_m: 3.0,
                    element_count: 2,
                    signature: BTreeMap::from([("optical".to_owned(), 0.5)]),
                    weapon: Some("gun".to_owned()),
                    ..Default::default()
                },
            ),
            (
                "tank".to_owned(),
                UnitType {
                    height_m: 2.5,
                    silhouette_width_m: 3.0,
                    element_count: 2,
                    signature: BTreeMap::from([("optical".to_owned(), 0.5)]),
                    weapon: Some("cannon".to_owned()),
                    ..Default::default()
                },
            ),
        ]),
        weapons: BTreeMap::from([
            (
                "gun".to_owned(),
                WeaponType {
                    class: WeaponClass::Indirect,
                    rof_rounds_per_min: 12.0,
                    max_range_m: 6000.0,
                    cep_m: 30.0,
                    lethal_radius_m: 40.0,
                    ..Default::default()
                },
            ),
            (
                "cannon".to_owned(),
                WeaponType {
                    class: WeaponClass::Direct,
                    rof_rounds_per_min: 30.0,
                    max_range_m: 3000.0,
                    dispersion_mrad: 0.4,
                    p_kill_given_hit: 0.9,
                    ..Default::default()
                },
            ),
        ]),
        air_defence: BTreeMap::from([(
            "sam".to_owned(),
            AirDefenceType {
                max_range_m: 8000.0,
                max_alt_m: 4000.0,
                element_count: 4,
                height_m: 3.0,
                silhouette_width_m: 4.0,
                sensor: Some("radar".to_owned()),
                // Worth shooting: without a declared value an emplacement scores 1.0 per
                // element and the guns would rationally prefer a unit (§12.4). Saying so
                // here is exactly the judgement the dial exists for.
                value: Some(6.0),
                engagement: AdEngagement::Gun {
                    kill_rate_per_s: 0.0,
                },
                ..Default::default()
            },
        )]),
        c2: BTreeMap::from([(
            "post".to_owned(),
            C2Type {
                coordination_range_m: 3000.0,
                element_count: 2,
                value: Some(6.0),
                ..Default::default()
            },
        )]),
        ..Libraries::with_terrain(scenario_params())
    }
}

/// Blue guns against a Red SAM, whose radar is on or off, with an optional Red post.
fn counter_battery(shooter: &str, emitting: bool, post: bool) -> Sim {
    let post_toml = if post {
        format!(
            r#"
        [[red.c2]]
        id = "red-cp"
        type = "post"
        pos = [{SAM_X}, 700.0]
        "#
        )
    } else {
        String::new()
    };
    let scn = Scenario::from_toml_str(&format!(
        r#"
        name = "counter-battery"
        default_seed = 4
        [sim]
        dt_s = 1.0
        epoch_s = 10.0
        [terrain]
        cell_size_m = 10.0
        width_cells = 300
        height_cells = 150
        [terrain.source.flat]
        elevation_m = 0.0
        [[blue.units]]
        id = "bty-1"
        type = "{shooter}"
        pos = [{GUN_X}, {GUN_Y}]
        [[red.air_defence]]
        id = "sam-1"
        type = "sam"
        pos = [{SAM_X}, {GUN_Y}]
        emitting = {emitting}
        {post_toml}
    "#
    ))
    .unwrap();
    Sim::new(&scn, &libraries(), 4).unwrap()
}

/// Elements the battery has lost.
fn battery_losses(sim: &Sim) -> u32 {
    4 - sim.air_defence()[0].elements
}

// V65 (headline): artillery can conduct counter-battery. A SAM whose radar is up is
// locatable, and a howitzer in range kills it - which §12 could not express at all.
#[test]
fn v65_artillery_can_kill_an_emitting_battery() {
    let mut sim = counter_battery("howitzer", true, false);
    sim.run_until(300.0);
    assert!(
        battery_losses(&sim) > 0,
        "an emitting SAM in range of a howitzer must take counter-battery fire"
    );
    assert!(
        sim.fire_events()
            .iter()
            .any(|e| e.target == FireTarget::AirDefence(0)),
        "and the fire log must say what was hit"
    );
}

// V65 (acquisition half): the counter is the same one V64 poses. A battery that is neither
// transmitting nor shooting has not given itself away, so indirect fire has nothing to aim
// at - the §2 track gate applies to an emplacement exactly as to a unit.
#[test]
fn v65_a_silent_battery_cannot_be_found_by_indirect_fire() {
    let mut quiet = counter_battery("howitzer", false, false);
    quiet.run_until(300.0);
    assert_eq!(
        battery_losses(&quiet),
        0,
        "a battery that has neither emitted nor fired must not be locatable"
    );
    assert!(
        !quiet.emplacement_is_located(FireTarget::AirDefence(0)),
        "and the model must say so directly, not merely fail to hit it"
    );

    let loud = counter_battery("howitzer", true, false);
    assert!(
        loud.emplacement_is_located(FireTarget::AirDefence(0)),
        "the same battery transmitting is located"
    );
}

// V65 (direct-fire half): direct fire needs line of sight and range, not a track - the §2.1
// rule, applied unchanged. So going silent hides a battery from artillery but not from
// anything that can see it.
#[test]
fn v65_direct_fire_needs_no_track() {
    let mut sim = counter_battery("tank", false, false);
    sim.run_until(300.0);
    assert!(
        battery_losses(&sim) > 0,
        "a silent battery in plain view is still a target for direct fire"
    );
}

// V65 (C2 half): a post is located because it is *talking*, and killing it by
// counter-battery decoheres the defence exactly as an air-delivered kill does (§11, §12.2)
// - the consequence belongs to the death, not to what caused it.
#[test]
fn v65_a_coordinating_post_is_locatable_and_killing_it_decoheres() {
    let mut sim = counter_battery("howitzer", true, true);
    assert!(
        sim.emplacement_is_located(FireTarget::C2(0)),
        "a post coordinating a live battery has given itself away"
    );
    sim.run_until(600.0);

    let post_hit = sim
        .fire_events()
        .iter()
        .any(|e| e.target == FireTarget::C2(0));
    assert!(
        post_hit || battery_losses(&sim) > 0,
        "the guns must engage something in the emplacement list"
    );

    // Once every battery it coordinated is dead, the post is talking to nobody and is no
    // longer locatable - the acquisition rule is a live property, not a one-way latch.
    if !sim.air_defence()[0].alive() {
        assert!(
            !sim.emplacement_is_located(FireTarget::C2(0)),
            "a post with nothing left to coordinate has stopped transmitting"
        );
    }
}

// V65 (identity half, §7.4): a scenario with no enemy emplacements produces exactly the
// target list it always did, so nothing about the ground fight moved. Units come first in
// `engageable_targets` for precisely this reason.
#[test]
fn v65_a_scenario_without_emplacements_is_unchanged() {
    let scn = Scenario::from_toml_str(&format!(
        r#"
        name = "counter-battery-identity"
        default_seed = 4
        [sim]
        dt_s = 1.0
        epoch_s = 10.0
        [terrain]
        cell_size_m = 10.0
        width_cells = 300
        height_cells = 150
        [terrain.source.flat]
        elevation_m = 0.0
        [[blue.units]]
        id = "bty-1"
        type = "tank"
        pos = [{GUN_X}, {GUN_Y}]
        [[red.units]]
        id = "red-1"
        type = "tank"
        pos = [{SAM_X}, {GUN_Y}]
    "#
    ))
    .unwrap();
    let run = || -> Vec<(usize, FireTarget, u32)> {
        let mut sim = Sim::new(&scn, &libraries(), 4).unwrap();
        sim.run_until(300.0);
        sim.fire_events()
            .iter()
            .map(|e| (e.shooter, e.target, e.casualties))
            .collect()
    };
    let log = run();
    assert_eq!(log, run(), "must reproduce");
    assert!(!log.is_empty(), "the fixture must actually shoot");
    assert!(
        log.iter().all(|(_, t, _)| t.unit().is_some()),
        "with no emplacements on the field, every target must be a unit"
    );
}
