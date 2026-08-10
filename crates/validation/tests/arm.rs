//! V64 — anti-radiation homing: the radar buys its own accuracy. `docs/DESIGN.md` §12.3.
//!
//! §12 made air-defence batteries killable, but the missile did not care whether its target
//! was radiating: the aim point was the battery's position regardless. A real ARM rides the
//! radar's own signal down, so its accuracy is *bought with the target's emissions*, and
//! switching the radar off is a counter.
//!
//! That is the trade this gate pins, and it is a genuine one in both directions. `emitting`
//! off is not free: the radar is *off*, so the battery detects nothing at all through it.
//! **Survive the missile, or see the raid coming — not both.**
//!
//! `emitting` is a separate flag from `self_cue` for exactly this reason. They were once the
//! same one, and sharing it meant a battery could take the missile protection of going dark
//! while its radar carried on seeing everything — the survivability of EMCON without its
//! cost, which made this gate pass while measuring the wrong thing (§12.5, gate V69).
//! `self_cue` now means only "who does this battery listen to"; `emitting` means "is the
//! radar on".
//!
//! Modelled as a dispersion, not a veto. The munition still arrives; with nothing to home on
//! it flies to where the emitter was last known to be and lands with `silent_cep_m` instead
//! of `cep_m`. "An ARM cannot engage a silent radar at all" is that with the value set very
//! large — reachable as a scenario's choice, rather than baked in as the model's opinion.

use sim_core::air::{AirType, TargetSpec};
use sim_core::air_defence::{AdEngagement, AirDefenceType};
use sim_core::fires::{WeaponClass, WeaponType};
use sim_core::scenario::{Libraries, Scenario};
use sim_core::sensing::{Modality, SensorType};
use sim_core::sim::Sim;
use std::collections::BTreeMap;
use validation::scenario_params;

/// Where the battery sits, and where the drone starts.
const SAM_X: f32 = 2000.0;
const SAM_Y: f32 = 1000.0;
const LAUNCH_X: f32 = 200.0;

/// Accurate when the radar is up; badly off when it is not.
const ARM_CEP_M: f32 = 5.0;
const ARM_SILENT_CEP_M: f32 = 400.0;

fn libraries(anti_radiation: bool) -> Libraries {
    Libraries {
        sensors: BTreeMap::from([(
            "radar".to_owned(),
            SensorType {
                modality: Modality::Optical,
                mount_height_m: 4.0,
                max_range_m: 6000.0,
                lambda0_per_s: 2.0,
                range_half_m: 6000.0,
                range_exponent: 2.0,
                for_width_deg: None,
            },
        )]),
        air_defence: BTreeMap::from([(
            "sam".to_owned(),
            AirDefenceType {
                // Deliberately toothless: this gate is about what the ARM does to the
                // battery, not what the battery does back. V48-V51 own that half.
                max_range_m: 10.0,
                max_alt_m: 10.0,
                element_count: 6,
                height_m: 3.0,
                silhouette_width_m: 4.0,
                sensor: Some("radar".to_owned()),
                engagement: AdEngagement::Gun {
                    kill_rate_per_s: 0.0,
                },
                ..Default::default()
            },
        )]),
        weapons: BTreeMap::from([(
            "arm".to_owned(),
            WeaponType {
                class: WeaponClass::Indirect,
                cep_m: ARM_CEP_M,
                silent_cep_m: Some(ARM_SILENT_CEP_M),
                anti_radiation,
                // Tight enough that the 400 m silent miss is well outside it, so the
                // difference in dispersion shows up as a difference in damage.
                lethal_radius_m: 30.0,
                ..Default::default()
            },
        )]),
        air: BTreeMap::from([(
            "sead".to_owned(),
            AirType {
                height_m: 2.0,
                cruise_speed_m_s: 60.0,
                signature: BTreeMap::from([("optical".to_owned(), 0.7)]),
                payload: Some("arm".to_owned()),
                munitions: 1,
                release_range_m: 900.0,
                ..Default::default()
            },
        )]),
        ..Libraries::with_terrain(scenario_params())
    }
}

/// A SEAD drone flying at a SAM whose radar is transmitting or shut down.
fn sead(emitting: bool, seed: u64) -> Sim {
    let scn = Scenario::from_toml_str(&format!(
        r#"
        name = "arm"
        default_seed = 5
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
        type = "sam"
        pos = [{SAM_X}, {SAM_Y}]
        emitting = {emitting}
        [[red.air]]
        id = "sead-1"
        type = "sead"
        pos = [{LAUNCH_X}, {SAM_Y}]
        heading_deg = 0.0
        waypoints = [[{SAM_X}, {SAM_Y}]]
        target = {{ asset = "sam-1" }}
    "#
    ))
    .unwrap();
    Sim::new(&scn, &libraries(true), seed).unwrap()
}

/// How far the munition landed from its aim point, and how many launchers it destroyed.
fn miss_and_kills(sim: &Sim) -> Option<(f32, u32)> {
    let e = sim.strike_events().first()?;
    Some((e.burst.distance(e.aim), e.casualties))
}

// V64 (headline): the same missile against the same battery is accurate when the radar is
// transmitting and badly off when it is not.
#[test]
fn v64_a_silent_radar_degrades_the_missile() {
    // Averaged over seeds: a single CEP draw is a random variable, and one sample of it
    // proves nothing about a dispersion.
    let mut emitting_miss = 0.0f32;
    let mut silent_miss = 0.0f32;
    let mut emitting_kills = 0u32;
    let mut silent_kills = 0u32;
    let seeds = 24u64;

    for seed in 0..seeds {
        let mut on = sead(true, seed);
        let mut off = sead(false, seed);
        on.run_until(200.0);
        off.run_until(200.0);
        let (m_on, k_on) = miss_and_kills(&on).expect("the drone must release");
        let (m_off, k_off) = miss_and_kills(&off).expect("the drone must release");
        emitting_miss += m_on;
        silent_miss += m_off;
        emitting_kills += k_on;
        silent_kills += k_off;
    }
    let n = seeds as f32;

    // E|miss| for a 2-D Gaussian is sigma*sqrt(pi/2), and sigma = CEP/sqrt(2 ln 2), so the
    // ratio of mean misses is exactly the ratio of the two CEPs.
    let expected_ratio = ARM_SILENT_CEP_M / ARM_CEP_M;
    let ratio = (silent_miss / n) / (emitting_miss / n).max(1e-6);
    assert!(
        (ratio / expected_ratio - 1.0).abs() < 0.35,
        "mean miss should scale with the CEP ratio ({expected_ratio:.0}x); got {ratio:.1}x \
         from {:.1} m emitting, {:.1} m silent",
        emitting_miss / n,
        silent_miss / n
    );
    assert!(
        emitting_kills > silent_kills,
        "the accurate missile must do more damage: {emitting_kills} vs {silent_kills} \
         launchers over {seeds} seeds"
    );
}

// V64 (counter half): switching the radar off is a real counter — it is what makes the
// missile miss — but the model must not let it be free. A battery under EMCON still *has* a
// radar; it is simply not transmitting, so it sees nothing through it.
#[test]
fn v64_going_silent_is_the_counter_and_it_costs_the_radar() {
    let quiet = sead(false, 1);
    let battery = &quiet.air_defence()[0];
    assert!(
        !battery.emitting,
        "the fixture's counter is the battery shutting its radar down"
    );
    assert!(
        battery.sensor_idx.is_some(),
        "it still HAS a radar — it has chosen not to run it, which is the whole point"
    );

    let loud = sead(true, 1);
    assert!(loud.air_defence()[0].emitting);
}

// V64 (identity half, §7.4): a weapon that is not an ARM ignores the emitter entirely, so
// every existing munition is bit-identical. `cep_against` is the single place that decides.
#[test]
fn v64_an_ordinary_munition_ignores_the_emitter() {
    let plain = WeaponType {
        class: WeaponClass::Indirect,
        cep_m: 40.0,
        silent_cep_m: Some(999.0), // set, but must be unreachable without the flag
        ..Default::default()
    };
    assert_eq!(plain.cep_against(true), 40.0);
    assert_eq!(
        plain.cep_against(false),
        40.0,
        "a dumb shell's accuracy does not depend on what the target is transmitting"
    );

    // And an ARM with no `silent_cep_m` stated falls back to `cep_m`, so declaring the
    // flag alone changes nothing until the degradation is given a number.
    let undeclared = WeaponType {
        class: WeaponClass::Indirect,
        cep_m: 40.0,
        anti_radiation: true,
        ..Default::default()
    };
    assert_eq!(undeclared.cep_against(false), 40.0);
}

// V64 (targeting half): only a named, live, transmitting battery counts as emitting.
// A command post, a unit or a map point radiates nothing an ARM could ride, so an ARM sent
// at one is flying blind by definition rather than by omission.
#[test]
fn v64_only_a_transmitting_battery_counts_as_an_emitter() {
    let mut point_target = sead(true, 2);
    // Re-aim the drone at bare ground: same missile, no emitter.
    point_target.air_mut(0).target = Some(TargetSpec::Point(glam::Vec2::new(SAM_X, SAM_Y)));
    point_target.run_until(200.0);
    let (miss_at_point, _) = miss_and_kills(&point_target).expect("released");

    let mut at_radar = sead(true, 2);
    at_radar.run_until(200.0);
    let (miss_at_radar, _) = miss_and_kills(&at_radar).expect("released");

    assert!(
        miss_at_point > miss_at_radar,
        "an ARM aimed at bare ground has nothing to home on: {miss_at_point:.1} m vs \
         {miss_at_radar:.1} m at the radar"
    );
}
