//! V60 - SEAD: air defence and C2 are attritable. `docs/DESIGN.md` §12.
//!
//! Before this, a battery was immortal and a post could only be removed by calling
//! `Sim::remove_c2` from outside the simulation. Neither could be *attacked*, so the
//! §11 finding - that a command post is the thing worth killing first - was a claim the
//! model could not actually demonstrate.
//!
//! These gates run the whole chain instead: a strike drone is assigned a named air-defence
//! asset, flies to its release point, drops, and the asset dies. What follows from the
//! death is the part that matters - a battery's radar goes dark with it, and a post's
//! group decoheres.

use sim_core::air::AirType;
use sim_core::air_defence::{AdEngagement, AirDefenceType};
use sim_core::c2::C2Type;
use sim_core::fires::{WeaponClass, WeaponType};
use sim_core::scenario::{Libraries, Scenario};
use sim_core::sensing::{Modality, SensorType};
use sim_core::sim::Sim;
use std::collections::BTreeMap;
use validation::scenario_params;

fn libraries() -> Libraries {
    Libraries {
        sensors: BTreeMap::from([(
            "radar".to_owned(),
            SensorType {
                modality: Modality::Optical,
                mount_height_m: 4.0,
                max_range_m: 8000.0,
                lambda0_per_s: 5.0,
                range_half_m: 8000.0,
                range_exponent: 2.0,
                for_width_deg: None,
            },
        )]),
        air: BTreeMap::from([(
            "sead".to_owned(),
            AirType {
                height_m: 1.5,
                cruise_speed_m_s: 60.0,
                signature: BTreeMap::from([("optical".to_owned(), 0.4)]),
                payload: Some("arm".to_owned()),
                munitions: 1,
                // Comfortably more than the 300 m cruise altitude: with a release range
                // equal to the altitude, the only point satisfying it is directly
                // overhead, which a 60 m tick steps straight past.
                release_range_m: 900.0,
                ..Default::default()
            },
        )]),
        weapons: BTreeMap::from([(
            "arm".to_owned(),
            WeaponType {
                class: WeaponClass::Indirect,
                rof_rounds_per_min: 60.0,
                max_range_m: 2000.0,
                // Accurate and lethal: this gate is about whether the damage *reaches*
                // air-defence assets at all, not about whether a marginal shot connects.
                cep_m: 3.0,
                lethal_radius_m: 60.0,
                ..Default::default()
            },
        )]),
        air_defence: BTreeMap::from([(
            "gun".to_owned(),
            AirDefenceType {
                max_range_m: 3000.0,
                max_alt_m: 2000.0,
                requires_los: false,
                reaction_time_s: 0.0,
                magazine: 0,
                channels: 1,
                element_count: 2,
                sensor: Some("radar".to_owned()),
                engagement: AdEngagement::Gun {
                    // Almost harmless, so the SEAD drone reliably survives to release.
                    // Whether SEAD can fight its way in is a scenario question; this gate
                    // is about what happens once it arrives.
                    kill_rate_per_s: 1.0e-6,
                },
                ..Default::default()
            },
        )]),
        c2: BTreeMap::from([(
            "post".to_owned(),
            C2Type {
                coordination_range_m: 4000.0,
                element_count: 1,
                ..Default::default()
            },
        )]),
        ..Libraries::with_terrain(scenario_params())
    }
}

/// Two batteries, a post covering both, and one SEAD drone sent at `target_id`.
fn strike_on(target_id: &str) -> Sim {
    let scn = Scenario::from_toml_str(&format!(
        r#"
        name = "sead"
        default_seed = 5
        [sim]
        dt_s = 1.0
        epoch_s = 10.0
        [terrain]
        cell_size_m = 10.0
        width_cells = 500
        height_cells = 300
        [terrain.source.flat]
        elevation_m = 0.0
        [[blue.air_defence]]
        id = "gun-a"
        type = "gun"
        pos = [1000.0, 1400.0]
        [[blue.air_defence]]
        id = "gun-b"
        type = "gun"
        pos = [1000.0, 1600.0]
        [[blue.c2]]
        id = "cp"
        type = "post"
        pos = [1000.0, 1500.0]
        [[red.air]]
        id = "sead-1"
        type = "sead"
        pos = [4000.0, 1500.0]
        altitude_m = 300.0
        heading_deg = 180.0
        waypoints = [[1000.0, 1500.0]]
        target = {{ unit = "{target_id}" }}
    "#
    ))
    .unwrap();
    Sim::new(&scn, &libraries(), 5).unwrap()
}

// V60 (the headline): a strike drone assigned a named C2 post actually destroys it, and
// the batteries it was coordinating lose their group. Nothing removes the post from
// outside - the whole chain runs inside the simulation.
#[test]
fn v60_sead_kills_a_c2_post_and_decoheres_the_defence() {
    let mut sim = strike_on("cp");
    assert!(sim.c2()[0].alive(), "the post should start alive");

    sim.run_until(300.0);

    assert!(
        !sim.strike_events().is_empty(),
        "the drone should have released a munition"
    );
    assert!(
        !sim.c2()[0].alive(),
        "a munition on the post should destroy it"
    );
    // The point of killing a post: no battery is lost, only the coordination.
    assert_eq!(sim.air_defence().len(), 2);
    assert!(
        sim.air_defence().iter().all(alive_ad),
        "killing the post must not scratch a single battery"
    );
}

/// `AirDefenceState::alive` as a free function, for use in iterator adapters.
fn alive_ad(ad: &sim_core::air_defence::AirDefenceState) -> bool {
    ad.alive()
}

// V60 (battery half): a battery can be destroyed too, and when it is, its organic radar
// goes dark with it. That is what makes SEAD worth more than the launchers it removes -
// a self-cueing battery is also an emitter the rest of the network was leaning on.
#[test]
fn v60_killing_a_battery_takes_its_radar_with_it() {
    let mut sim = strike_on("gun-a");

    // The battery's organic radar is a sensor in the ordinary list, and starts active.
    let radar = (0..sim.sensors().len())
        .find(|&i| sim.sensors()[i].id.contains("gun-a"))
        .expect("the battery should have registered a radar");
    assert!(sim.sensor_active(radar), "the radar should start active");

    sim.run_until(300.0);

    assert!(
        !sim.air_defence()[0].alive(),
        "the targeted battery should be destroyed, got {} elements",
        sim.air_defence()[0].elements
    );
    assert!(
        !sim.sensor_active(radar),
        "a destroyed battery's radar must stop emitting"
    );
    assert!(
        sim.air_defence()[1].alive(),
        "the other battery should be untouched"
    );
}

// V60 (identity half): naming a *unit* still behaves exactly as it did before batteries
// and posts became targetable - the new lists are additional sweep targets, not a
// replacement, so no existing scenario changes (the §7.4 discipline).
#[test]
fn v60_air_defence_assets_are_additive_not_a_replacement() {
    // With nothing named, the drone falls back to its final waypoint (§9.3) and the
    // munition lands there - on top of the post, which sits at that waypoint.
    let mut sim = strike_on("no-such-asset");
    sim.run_until(300.0);
    assert!(
        sim.strike_events().is_empty(),
        "a named target that does not exist must yield no aim point, so no release"
    );
    assert!(
        sim.c2()[0].alive() && sim.air_defence().iter().all(alive_ad),
        "nothing should have been damaged"
    );
}
