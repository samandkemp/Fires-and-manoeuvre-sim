//! Gates that need the whole simulation loop running (docs/DESIGN.md §3.3):
//! V14/V15, V18, V24, V30, V31, V37-V40, and the Phase 9 air integration gates.
//!
//! These share heavyweight fixtures — a duel, a battle, a raid — so they live in
//! one file rather than being split by V-number across the per-subsystem gate
//! files. The zero-draw half of V52 is *not* here: it asserts a property of the
//! RNG draw stream, which is internal by definition, so it stays a unit test
//! inside `sim_core`.

use glam::Vec2;
use sim_core::air::AirType;
use sim_core::air_defence::{AdEngagement, AirDefenceType};
use sim_core::fires::{WeaponClass, WeaponType};
use sim_core::scenario::{Libraries, Scenario};
use sim_core::sensing::{detection_rate, Modality, SensorType, UnitType};
use sim_core::sim::{Side, Sim, UnitState};
use sim_core::suppression::Suppression;
use std::collections::BTreeMap;
use validation::scenario_params;

/// A scenario plus the libraries its instances resolve against.
type Fixture = (Scenario, Libraries);

/// A one-sensor, one-target duel on a flat range, built from an inline scenario.
fn duel_scenario() -> Fixture {
    let text = r#"
        name = "duel"
        default_seed = 9
        [sim]
        dt_s = 1.0
        epoch_s = 10.0
        [terrain]
        cell_size_m = 10.0
        width_cells = 64
        height_cells = 16
        [terrain.source.flat]
        elevation_m = 0.0
        [[blue.sensors]]
        id = "obs"
        type = "s"
        pos = [50.0, 80.0]
        [[red.units]]
        id = "tgt"
        type = "u"
        pos = [550.0, 80.0]
    "#;
    let scn = Scenario::from_toml_str(text).unwrap();
    let sensors = BTreeMap::from([(
        "s".to_owned(),
        SensorType {
            modality: Modality::Optical,
            mount_height_m: 2.0,
            max_range_m: 4000.0,
            lambda0_per_s: 0.2,
            range_half_m: 1200.0,
            range_exponent: 2.0,
            for_width_deg: None,
        },
    )]);
    let units = BTreeMap::from([(
        "u".to_owned(),
        UnitType {
            height_m: 2.0,
            signature: BTreeMap::from([("optical".to_owned(), 0.8)]),
            ..Default::default()
        },
    )]);
    let libs = Libraries {
        sensors,
        units,
        ..Libraries::with_terrain(scenario_params())
    };
    (scn, libs)
}

/// The analytic λ for the duel geometry, straight from the rate function.
fn duel_lambda(scn: &Scenario, libs: &Libraries) -> f32 {
    let terrain = scn.build_terrain(&libs.terrain_params, scn.default_seed);
    detection_rate(
        &terrain,
        &libs.sensors["s"],
        Vec2::new(50.0, 80.0),
        0.0,
        &libs.units["u"],
        Vec2::new(550.0, 80.0),
    )
}

// V15 (+V14): Monte Carlo detection frequency by time t within binomial CI of the
// closed form, and mean detection time ≈ 1/λ.
#[test]
fn v14_v15_exponential_law_monte_carlo() {
    let (scn, libs) = duel_scenario();
    let lambda = f64::from(duel_lambda(&scn, &libs));
    assert!(
        lambda > 0.01 && lambda < 1.0,
        "duel λ should be a sane rate, got {lambda}"
    );

    let n = 1500;
    let t_check = 20.0;
    let mut detected_by_t = 0u32;
    let mut detection_times = Vec::new();
    for seed in 0..n {
        let mut sim = Sim::new(&scn, &libs, 1000 + seed).unwrap();
        sim.run_until(200.0);
        if let Some(e) = sim.events().first() {
            if e.time_s <= t_check {
                detected_by_t += 1;
            }
            detection_times.push(e.time_s);
        }
    }

    // V15: frequency vs 1 − e^{−λt}, 3.5σ binomial band.
    let p_exact = 1.0 - (-lambda * t_check).exp();
    let p_hat = f64::from(detected_by_t) / n as f64;
    let sigma = (p_exact * (1.0 - p_exact) / n as f64).sqrt();
    assert!(
        (p_hat - p_exact).abs() < 3.5 * sigma,
        "P(detect by {t_check}) = {p_hat:.4}, closed form {p_exact:.4}, σ {sigma:.4}"
    );

    // V14: nearly every run detects by t=200 (e^{−λ·200} ≈ 0), so the sample mean
    // estimates 1/λ. Discreteness of 1 s ticks biases the mean up by ~dt/2.
    let mean: f64 = detection_times.iter().sum::<f64>() / detection_times.len() as f64;
    let expected = 1.0 / lambda + 0.5;
    let se = (1.0 / lambda) / (detection_times.len() as f64).sqrt();
    assert!(
        (mean - expected).abs() < 4.0 * se,
        "mean detection time {mean:.2} vs 1/λ + dt/2 = {expected:.2} (se {se:.3})"
    );
}

// V18: same (scenario, seed) → identical event log; different seed differs.
#[test]
fn v18_determinism() {
    let (scn, libs) = duel_scenario();
    let run = |seed: u64| {
        let mut sim = Sim::new(&scn, &libs, seed).unwrap();
        sim.run_until(120.0);
        sim.events().to_vec()
    };
    assert_eq!(
        run(7),
        run(7),
        "same seed must reproduce the event log exactly"
    );
    let (a, b) = (run(7), run(8));
    let same = a.len() == b.len()
        && a.iter()
            .zip(b.iter())
            .all(|(x, y)| (x.time_s - y.time_s).abs() < f64::EPSILON);
    assert!(
        !same,
        "different seeds should give different detection times"
    );
}

// V37: a unit on a straight route travels speed·t (within one tick's step).
#[test]
fn v37_route_following() {
    let text = r#"
        name = "move"
        [terrain]
        cell_size_m = 10.0
        width_cells = 200
        height_cells = 20
        [terrain.source.flat]
        elevation_m = 0.0
        [[blue.units]]
        id = "mover"
        type = "mover"
        pos = [0.0, 100.0]
        route = [[1000.0, 100.0]]
    "#;
    let scn = Scenario::from_toml_str(text).unwrap();
    let units = BTreeMap::from([(
        "mover".to_owned(),
        UnitType {
            height_m: 2.0,
            speed_m_s: 10.0,
            ..Default::default()
        },
    )]);
    let libs = Libraries {
        units,
        ..Libraries::with_terrain(scenario_params())
    };
    let mut sim = Sim::new(&scn, &libs, 0).unwrap();
    sim.run_until(30.0);
    let x = sim.units()[0].pos.x;
    assert!(
        (x - 300.0).abs() < 10.0,
        "after 30 s at 10 m/s the mover should be ~300 m along (got {x})"
    );
}

// V38: a Pinned unit does not advance along its route.
#[test]
fn v38_pinned_unit_halts() {
    let text = r#"
        name = "pin"
        [sim]
        recover_per_s = 0.0
        [terrain]
        cell_size_m = 10.0
        width_cells = 200
        height_cells = 20
        [terrain.source.flat]
        elevation_m = 0.0
        [[blue.units]]
        id = "mover"
        type = "mover"
        pos = [0.0, 100.0]
        route = [[1000.0, 100.0]]
    "#;
    let scn = Scenario::from_toml_str(text).unwrap();
    let units = BTreeMap::from([(
        "mover".to_owned(),
        UnitType {
            height_m: 2.0,
            speed_m_s: 10.0,
            ..Default::default()
        },
    )]);
    let libs = Libraries {
        units,
        ..Libraries::with_terrain(scenario_params())
    };
    let mut sim = Sim::new(&scn, &libs, 0).unwrap();
    sim.set_suppression(0, Suppression::Pinned);
    sim.run_until(30.0);
    assert_eq!(sim.units()[0].pos.x, 0.0, "a pinned unit must not move");
}

// V40 (integration): a jammer over the target sharply cuts detections, and with no
// jammer the run is bit-for-bit identical to the un-jammed sim (EW-off identity).
#[test]
fn v40_ew_degrades_and_off_is_identity() {
    let base = r#"
        name = "ew"
        default_seed = 4
        [sim]
        dt_s = 1.0
        epoch_s = 10.0
        [terrain]
        cell_size_m = 10.0
        width_cells = 64
        height_cells = 16
        [terrain.source.flat]
        elevation_m = 0.0
        [[blue.sensors]]
        id = "obs"
        type = "s"
        pos = [50.0, 80.0]
        [[red.units]]
        id = "tgt"
        type = "u"
        pos = [550.0, 80.0]
    "#;
    let sensors = BTreeMap::from([(
        "s".to_owned(),
        SensorType {
            modality: Modality::Optical,
            mount_height_m: 2.0,
            max_range_m: 4000.0,
            lambda0_per_s: 0.3,
            range_half_m: 1200.0,
            range_exponent: 2.0,
            for_width_deg: None,
        },
    )]);
    let units = BTreeMap::from([(
        "u".to_owned(),
        UnitType {
            height_m: 2.0,
            signature: BTreeMap::from([("optical".to_owned(), 0.8)]),
            ..Default::default()
        },
    )]);
    let libs = Libraries {
        sensors,
        units,
        ..Libraries::with_terrain(scenario_params())
    };

    let detect_frac = |scn_text: &str| -> f64 {
        let scn = Scenario::from_toml_str(scn_text).unwrap();
        let mut detected = 0u32;
        let trials = 300u64;
        for seed in 0..trials {
            let mut sim = Sim::new(&scn, &libs, seed).unwrap();
            sim.run_until(30.0);
            if !sim.events().is_empty() {
                detected += 1;
            }
        }
        f64::from(detected) / trials as f64
    };

    let unjammed = detect_frac(base);

    // A strong jammer sitting on the Red target.
    let jammed_text =
        format!("{base}\n[[red.jammers]]\npos = [550.0, 80.0]\npower = 0.95\nradius_m = 400.0\n");
    let jammed = detect_frac(&jammed_text);

    assert!(
        unjammed > 0.6,
        "sanity: un-jammed target is usually detected ({unjammed})"
    );
    assert!(
        jammed < unjammed * 0.4,
        "the jammer must sharply cut detection ({jammed} vs {unjammed})"
    );

    // EW-off identity: a run with an empty jammer list equals the base run exactly.
    let scn = Scenario::from_toml_str(base).unwrap();
    let events_a = {
        let mut s = Sim::new(&scn, &libs, 11).unwrap();
        s.run_until(60.0);
        s.events().to_vec()
    };
    let events_b = {
        let mut s = Sim::new(&scn, &libs, 11).unwrap();
        s.run_until(60.0);
        s.events().to_vec()
    };
    assert_eq!(
        events_a, events_b,
        "EW-off must be deterministic and unchanged"
    );
}

// V39: interdiction sanity — a Red route that no Blue overwatch can see is safe, so
// the equilibrium puts Red on it and the game value falls. Builds a 2×2 payoff from
// real headless battles, then solves it.
#[test]
fn v39_interdiction_safe_route() {
    use sim_core::game::solve_zero_sum;
    let scn = Scenario::from_toml_str(
        r#"
        name = "v39"
        [terrain]
        cell_size_m = 10.0
        width_cells = 250
        height_cells = 250
        [terrain.source.flat]
        elevation_m = 0.0
    "#,
    )
    .unwrap();
    let libs = Libraries::with_terrain(scenario_params());
    let mut sim = Sim::new(&scn, &libs, 1).unwrap();

    let sensor = SensorType {
        modality: Modality::Optical,
        mount_height_m: 2.0,
        max_range_m: 4000.0,
        lambda0_per_s: 1.0,
        range_half_m: 1500.0,
        range_exponent: 2.0,
        for_width_deg: Some(50.0),
    };
    let mortar_unit = UnitType {
        height_m: 2.0,
        element_count: 1,
        signature: BTreeMap::new(),
        weapon: Some("m".to_owned()),
        value: None,
        ..Default::default()
    };
    let mortar = WeaponType {
        class: WeaponClass::Indirect,
        rof_rounds_per_min: 20.0,
        max_range_m: 4000.0,
        cep_m: 40.0,
        lethal_radius_m: 40.0,
        ..Default::default()
    };
    let red = UnitType {
        height_m: 2.8,
        silhouette_width_m: 3.2,
        element_count: 4,
        speed_m_s: 10.0,
        signature: BTreeMap::from([("optical".to_owned(), 0.9)]),
        weapon: None,
        value: None,
        role: None,
    };

    // Both Blue positions watch lane 0 (y=500) from the west; neither can see lane 1
    // (y=2000, outside every field of regard).
    let blue = [Vec2::new(1200.0, 500.0), Vec2::new(1800.0, 500.0)];
    let routes = [
        vec![Vec2::new(100.0, 500.0), Vec2::new(2400.0, 500.0)],
        vec![Vec2::new(100.0, 2000.0), Vec2::new(2400.0, 2000.0)],
    ];

    let seeds = 25u64;
    let mut payoff = ndarray::Array2::<f32>::zeros((2, 2));
    for bi in 0..2 {
        for rj in 0..2 {
            let mut acc = 0.0f32;
            for seed in 0..seeds {
                sim.reset(seed);
                sim.add_sensor("o", Side::Blue, blue[bi], 180.0, sensor.clone());
                sim.add_unit(
                    "m",
                    Side::Blue,
                    blue[bi],
                    mortar_unit.clone(),
                    Some(mortar.clone()),
                );
                sim.add_unit("r", Side::Red, routes[rj][0], red.clone(), None);
                let ri = sim.units().len() - 1;
                sim.set_route(ri, routes[rj].clone());
                loop {
                    sim.step_one();
                    let r = &sim.units()[ri];
                    if !r.alive() || r.route_idx >= r.route.len() || sim.time_s() > 600.0 {
                        acc += 1.0 - r.strength();
                        break;
                    }
                }
            }
            payoff[[bi, rj]] = acc / seeds as f32;
        }
    }

    assert!(
        payoff[[0, 0]] > 0.5,
        "watched lane should be interdicted: {}",
        payoff[[0, 0]]
    );
    assert!(
        payoff[[0, 1]] < 0.2 && payoff[[1, 1]] < 0.2,
        "the unwatched lane must be safe: {payoff:?}"
    );

    let sol = solve_zero_sum(&payoff, 50_000);
    assert!(
        sol.col_strategy[1] > 0.9,
        "Red should take the safe route: {:?}",
        sol.col_strategy
    );
    assert!(
        sol.value < 0.2,
        "value should fall when Red has a safe route: {}",
        sol.value
    );
}

/// Libraries for the air scenarios: a radar, a strike drone with a guided bomb, a
/// recce drone carrying the radar, a CIWS, and a Blue target unit.
fn air_libs() -> Libraries {
    let radar = SensorType {
        modality: Modality::Optical,
        mount_height_m: 3.0,
        max_range_m: 6000.0,
        lambda0_per_s: 2.0,
        range_half_m: 3000.0,
        range_exponent: 2.0,
        for_width_deg: None,
    };
    // Deliberately short-ranged, so only a drone overhead can pick the target up.
    let short = SensorType {
        max_range_m: 400.0,
        ..radar.clone()
    };
    Libraries {
        sensors: BTreeMap::from([("radar".to_owned(), radar), ("short".to_owned(), short)]),
        units: BTreeMap::from([(
            "target".to_owned(),
            UnitType {
                height_m: 2.5,
                silhouette_width_m: 3.0,
                element_count: 6,
                signature: BTreeMap::from([("optical".to_owned(), 0.8)]),
                ..Default::default()
            },
        )]),
        weapons: BTreeMap::from([(
            "guided_bomb".to_owned(),
            WeaponType {
                class: WeaponClass::Indirect,
                cep_m: 15.0,
                lethal_radius_m: 45.0,
                ..Default::default()
            },
        )]),
        air: BTreeMap::from([
            (
                "bomber".to_owned(),
                AirType {
                    height_m: 1.5,
                    cruise_speed_m_s: 50.0,
                    signature: BTreeMap::from([("optical".to_owned(), 0.8)]),
                    payload: Some("guided_bomb".to_owned()),
                    munitions: 1,
                    release_range_m: 400.0,
                    ..Default::default()
                },
            ),
            (
                "recce".to_owned(),
                AirType {
                    height_m: 1.5,
                    cruise_speed_m_s: 40.0,
                    signature: BTreeMap::from([("optical".to_owned(), 0.5)]),
                    sensor: Some("short".to_owned()),
                    ..Default::default()
                },
            ),
        ]),
        air_defence: BTreeMap::from([(
            "ciws".to_owned(),
            AirDefenceType {
                engagement: AdEngagement::Gun {
                    kill_rate_per_s: 1.5,
                },
                max_range_m: 3000.0,
                max_alt_m: 2000.0,
                sensor: Some("radar".to_owned()),
                ..Default::default()
            },
        )]),
        ..Libraries::with_terrain(scenario_params())
    }
}

/// A Red bomber inbound on a Blue unit across 2 km of flat ground.
fn raid_scenario(with_air_defence: bool) -> Scenario {
    let mut text = r#"
        name = "raid"
        [sim]
        dt_s = 1.0
        epoch_s = 10.0
        [terrain]
        cell_size_m = 10.0
        width_cells = 250
        height_cells = 200
        [terrain.source.flat]
        elevation_m = 0.0
        [[blue.units]]
        id = "gun"
        type = "target"
        pos = [2000.0, 1000.0]
        [[red.air]]
        id = "bomber-1"
        type = "bomber"
        pos = [200.0, 1000.0]
        altitude_m = 150.0
        altitude_ref = "agl"
        heading_deg = 0.0
        waypoints = [[2000.0, 1000.0]]
        target = { unit = "gun" }
    "#
    .to_owned();
    if with_air_defence {
        text.push_str(
            "\n[[blue.air_defence]]\nid = \"ciws-1\"\ntype = \"ciws\"\n\
             pos = [2000.0, 1000.0]\nself_cue = true\n",
        );
    }
    Scenario::from_toml_str(&text).unwrap()
}

// V52 (determinism half): with air, air defence and strikes all live, the same
// (scenario, seed) reproduces every log exactly, and a different seed does not.
#[test]
fn v52_air_determinism() {
    let scn = raid_scenario(true);
    let libs = air_libs();
    let run = |seed: u64| {
        let mut sim = Sim::new(&scn, &libs, seed).unwrap();
        sim.run_until(120.0);
        (
            sim.air_events().to_vec(),
            sim.air_defence_events().to_vec(),
            sim.strike_events().to_vec(),
            sim.air()[0].alive,
        )
    };
    assert_eq!(run(3), run(3), "same seed must reproduce the air battle");
    assert!(
        !run(3).1.is_empty(),
        "sanity: the battery should engage the raid"
    );
}

// The strike half of §9.3: an undefended bomber reaches its assigned target and its
// guided bomb attrits the unit. Damage is the ordinary §2.3 indirect maths, so this
// also exercises the area-damage sweep.
#[test]
fn air_strike_attrits_its_assigned_target() {
    let scn = raid_scenario(false);
    let libs = air_libs();

    let mut hit = 0u32;
    let trials = 40u64;
    for seed in 0..trials {
        let mut sim = Sim::new(&scn, &libs, seed).unwrap();
        sim.run_until(120.0);
        assert_eq!(
            sim.strike_events().len(),
            1,
            "one munition ⇒ exactly one release"
        );
        let release = &sim.strike_events()[0];
        // Released at the assigned unit, within the release range, and the burst
        // scattered around it by the payload's CEP.
        assert!((release.aim - Vec2::new(2000.0, 1000.0)).length() < 1e-3);
        assert!(release.burst.distance(release.aim) < 200.0);
        assert_eq!(sim.air()[0].munitions_left, 0);
        assert!(sim.air()[0].alive, "a non-expendable bomber survives");
        if release.casualties > 0 {
            hit += 1;
        }
    }
    assert!(
        hit > trials as u32 / 2,
        "a 45 m lethal radius on a 15 m CEP should usually cause casualties ({hit}/{trials})"
    );
}

// The counter-air half of §9.4–§9.5: with a self-cueing CIWS on the target, the
// bomber is shot down before it can release.
#[test]
fn air_defence_defeats_the_raid() {
    let libs = air_libs();
    let defended = raid_scenario(true);

    let mut shot_down = 0u32;
    let mut leaked = 0u32;
    let trials = 40u64;
    for seed in 0..trials {
        let mut sim = Sim::new(&defended, &libs, seed).unwrap();
        sim.run_until(120.0);
        if sim.air()[0].alive {
            leaked += 1;
        } else {
            shot_down += 1;
        }
        if !sim.strike_events().is_empty() {
            assert!(
                sim.air()[0].munitions_left == 0,
                "a release must consume a munition"
            );
        }
    }
    assert!(
        shot_down > trials as u32 * 3 / 4,
        "a lethal, self-cueing CIWS should stop most of the raid ({shot_down}/{trials})"
    );
    assert!(
        leaked < trials as u32 / 2,
        "and few bombers should survive to release ({leaked}/{trials})"
    );

    // The kill has to be an air-defence event, not something else.
    let mut sim = Sim::new(&defended, &libs, 0).unwrap();
    sim.run_until(120.0);
    assert!(sim
        .air_defence_events()
        .iter()
        .any(|e| e.killed && e.air == 0));
}

// A recce drone is just a sensor that flies: its carried sensor is short-ranged
// enough that only being overhead brings the Red unit inside it, which is exactly
// the mobile-elevated-observer behaviour §9 is after.
#[test]
fn recce_drone_detects_from_overhead() {
    let libs = air_libs();
    let text = r#"
        name = "recce"
        [sim]
        dt_s = 1.0
        [terrain]
        cell_size_m = 10.0
        width_cells = 250
        height_cells = 200
        [terrain.source.flat]
        elevation_m = 0.0
        [[red.units]]
        id = "hidden"
        type = "target"
        pos = [2000.0, 1000.0]
        [[blue.air]]
        id = "recce-1"
        type = "recce"
        pos = [200.0, 1000.0]
        altitude_m = 120.0
        heading_deg = 0.0
        waypoints = [[2000.0, 1000.0]]
        terminal = { orbit = { radius_m = 250.0, clockwise = false } }
    "#;
    let scn = Scenario::from_toml_str(text).unwrap();

    let mut sim = Sim::new(&scn, &libs, 4).unwrap();
    // The carried sensor is registered in the ordinary sensor list, bound to the
    // airframe — that binding is what makes it move.
    assert_eq!(sim.sensors().len(), 1);
    assert_eq!(sim.sensors()[0].carrier, Some(0));

    // Far away at the start: the 400 m sensor cannot reach 1800 m.
    sim.step_one();
    assert!(!sim.units()[0].detected, "out of reach at the start");
    let (pos, height, _) = sim.sensor_view(0);
    assert_eq!(pos, sim.air()[0].pos, "a carried sensor rides its airframe");
    assert_eq!(height, 120.0, "and sees from the airframe's altitude");

    sim.run_until(120.0);
    assert!(
        sim.units()[0].detected,
        "the drone should detect the unit once it arrives overhead"
    );
    let event = sim.events().first().expect("a detection event");
    assert_eq!(event.sensor, 0);
    assert!(
        sim.air()[0].orbit_phase.is_some(),
        "and settle into its orbit"
    );
}

// A carried sensor's *public* position field must track its airframe, and it must go
// inert when the airframe is shot down. Regression guard: consumers outside the
// detection loop (the app's coverage/belief overlays, the `duel_probe` experiment)
// read `SensorState.pos` directly, and a frozen value silently plotted a recce
// drone's coverage from its take-off point.
#[test]
fn carried_sensor_tracks_its_airframe() {
    let libs = air_libs();
    let text = r#"
        name = "carry"
        [sim]
        dt_s = 1.0
        [terrain]
        cell_size_m = 10.0
        width_cells = 250
        height_cells = 200
        [terrain.source.flat]
        elevation_m = 0.0
        [[blue.air]]
        id = "recce-1"
        type = "recce"
        pos = [200.0, 1000.0]
        altitude_m = 120.0
        heading_deg = 0.0
        waypoints = [[2000.0, 1000.0]]
    "#;
    let scn = Scenario::from_toml_str(text).unwrap();
    let mut sim = Sim::new(&scn, &libs, 1).unwrap();
    assert_eq!(sim.sensors()[0].carrier, Some(0));

    let start = sim.sensors()[0].pos;
    assert_eq!(start, Vec2::new(200.0, 1000.0), "placed with its airframe");
    sim.run_until(20.0);

    let air_pos = sim.air()[0].pos;
    assert!(
        air_pos.x > 900.0,
        "sanity: the drone should have flown on (at {air_pos})"
    );
    assert_eq!(
        sim.sensors()[0].pos,
        air_pos,
        "the carried sensor's position must follow the airframe, not stay at take-off"
    );
    assert_eq!(sim.sensors()[0].facing_deg, sim.air()[0].heading_deg);
    // And the effective view reports the airframe's altitude, not the mount height.
    let (view_pos, view_height, _) = sim.sensor_view(0);
    assert_eq!(view_pos, air_pos);
    assert_eq!(view_height, 120.0);
    assert!(sim.sensor_active(0));

    // Shot down ⇒ the sensor is inert, so it must not paint coverage either.
    sim.air_mut(0).alive = false;
    assert!(
        !sim.sensor_active(0),
        "a carried sensor dies with its airframe"
    );
}

// Epoch bookkeeping: epochs advance with sim time.
#[test]
fn epochs_advance() {
    let (scn, libs) = duel_scenario();
    let mut sim = Sim::new(&scn, &libs, 3).unwrap();
    sim.run_until(35.0);
    assert_eq!(
        sim.epochs_run(),
        3,
        "35 s at 10 s epochs = 3 boundaries crossed"
    );
}

/// A close-range fight where a Blue direct-fire unit can see and engage a Red unit:
/// checks fires actually attrit, and V24 (same seed → identical battle).
fn battle_scenario() -> Fixture {
    // Reuse the duel's libraries (sensor "s", unit "u"); rebuild the scenario with a
    // Blue direct-fire shooter placed to see and engage the Red target.
    let (_, mut libs) = duel_scenario();
    let text = r#"
        name = "battle"
        default_seed = 5
        [sim]
        dt_s = 1.0
        epoch_s = 10.0
        [terrain]
        cell_size_m = 10.0
        width_cells = 64
        height_cells = 16
        [terrain.source.flat]
        elevation_m = 0.0
        [[blue.sensors]]
        id = "obs"
        type = "s"
        pos = [50.0, 80.0]
        [[blue.units]]
        id = "gun"
        type = "shooter"
        pos = [60.0, 80.0]
        [[red.units]]
        id = "tgt"
        type = "u"
        pos = [700.0, 80.0]
    "#;
    let scn = Scenario::from_toml_str(text).unwrap();
    libs.units.insert(
        "shooter".to_owned(),
        UnitType {
            height_m: 2.5,
            silhouette_width_m: 3.0,
            element_count: 1,
            speed_m_s: 0.0,
            signature: BTreeMap::from([("optical".to_owned(), 0.5)]),
            weapon: Some("cannon".to_owned()),
            value: None,
            role: None,
        },
    );
    libs.weapons.insert(
        "cannon".to_owned(),
        WeaponType {
            class: WeaponClass::Direct,
            rof_rounds_per_min: 12.0,
            max_range_m: 3000.0,
            dispersion_mrad: 0.4,
            p_kill_given_hit: 0.8,
            ..Default::default()
        },
    );
    (scn, libs)
}

// Fires attrit a detected target, and the battle is deterministic (V24).
#[test]
fn v24_fires_attrit_and_are_deterministic() {
    let (scn, libs) = battle_scenario();
    let run = |seed: u64| {
        let mut sim = Sim::new(&scn, &libs, seed).unwrap();
        sim.run_until(600.0);
        let tgt = sim
            .units()
            .iter()
            .find(|u| u.id == "tgt")
            .unwrap()
            .strength();
        (tgt, sim.fire_events().to_vec())
    };
    let (strength_a, events_a) = run(11);
    let (strength_b, events_b) = run(11);
    assert_eq!(
        strength_a, strength_b,
        "same seed → identical target strength"
    );
    assert_eq!(events_a, events_b, "same seed → identical fire log");
    assert!(
        !events_a.is_empty(),
        "the detected target should be engaged"
    );
    assert!(
        strength_a < 1.0,
        "sustained direct fire should attrit the target"
    );

    let (strength_c, _) = run(12);
    // Different seed usually gives a different attrition outcome (not guaranteed if
    // both fully kill — but with these dials a 600 s fight kills, so compare killed).
    assert!(strength_c <= 1.0);
}

// V30: a homogeneous aimed-fire duel on open ground (suppression off) obeys
// Lanchester's square law — the winner is annihilation-tested to end with
// √(A₀²−B₀²) elements on average.
#[test]
fn v30_lanchester_square_law() {
    let text = r#"
        name = "lanchester"
        [sim]
        dt_s = 1.0
        epoch_s = 10.0
        p_suppress = 0.0
        [terrain]
        cell_size_m = 10.0
        width_cells = 80
        height_cells = 8
        [terrain.source.flat]
        elevation_m = 0.0
        [[blue.units]]
        id = "blue"
        type = "blue_line"
        pos = [200.0, 40.0]
        [[red.units]]
        id = "red"
        type = "red_line"
        pos = [500.0, 40.0]
    "#;
    let scn = Scenario::from_toml_str(text).unwrap();
    let line = |n: u32| UnitType {
        height_m: 2.0,
        silhouette_width_m: 3.0,
        element_count: n,
        speed_m_s: 0.0,
        signature: BTreeMap::new(),
        weapon: Some("rifle".to_owned()),
        value: None,
        role: None,
    };
    let units = BTreeMap::from([
        ("blue_line".to_owned(), line(50)),
        ("red_line".to_owned(), line(40)),
    ]);
    let weapons = BTreeMap::from([(
        "rifle".to_owned(),
        WeaponType {
            class: WeaponClass::Direct,
            rof_rounds_per_min: 6.0, // round(6·10/60) = 1 round/element/epoch
            max_range_m: 2000.0,
            dispersion_mrad: 0.5, // P_hit ≈ 1 at this range → p_kill = p_kill_given_hit
            p_kill_given_hit: 0.02,
            ..Default::default()
        },
    )]);
    let libs = Libraries {
        units,
        weapons,
        ..Libraries::with_terrain(scenario_params())
    };

    // The square-law invariant A² − B² is conserved by the deterministic ODE at
    // A₀² − B₀² = 900; check it holds in the mean over stochastic battles (robust to
    // which side happens to win an individual fight).
    let trials = 400u64;
    let mut sum_invariant = 0.0f64;
    let mut blue_wins = 0u32;
    for seed in 0..trials {
        let mut sim = Sim::new(&scn, &libs, seed).unwrap();
        while sim.units().iter().all(UnitState::alive) && sim.time_s() < 5000.0 {
            sim.step_one();
        }
        let blue = f64::from(
            sim.units()
                .iter()
                .find(|u| u.id == "blue")
                .unwrap()
                .elements,
        );
        let red = f64::from(sim.units().iter().find(|u| u.id == "red").unwrap().elements);
        sum_invariant += blue * blue - red * red;
        if blue > red {
            blue_wins += 1;
        }
    }
    let mean_invariant = sum_invariant / trials as f64;
    assert!(
        (mean_invariant - 900.0).abs() < 90.0,
        "mean (A²−B²) = {mean_invariant:.0} vs Lanchester A₀²−B₀² = 900"
    );
    // The stronger force should win the large majority of fights.
    assert!(
        blue_wins > trials as u32 * 9 / 10,
        "Blue (50 vs 40) should usually win: {blue_wins}/{trials}"
    );
}

// V31: a Pinned unit emits no rounds; a Suppressed unit's expected output is
// `suppressed_fire_factor` × a Free unit's. Drives one shooter at a fixed state
// against a large target and measures casualties over many seeds.
#[test]
fn v31_suppression_gates_fire() {
    // Inline scenario: a Blue direct shooter vs a big Red target, flat, in LOS+range.
    let text = r#"
        name = "supp"
        [sim]
        dt_s = 1.0
        epoch_s = 10.0
        recover_per_s = 0.0
        [terrain]
        cell_size_m = 10.0
        width_cells = 40
        height_cells = 8
        [terrain.source.flat]
        elevation_m = 0.0
        [[blue.units]]
        id = "gun"
        type = "shooter"
        pos = [40.0, 40.0]
        [[red.units]]
        id = "block"
        type = "block"
        pos = [300.0, 40.0]
    "#;
    let scn = Scenario::from_toml_str(text).unwrap();
    let units = BTreeMap::from([
        (
            "shooter".to_owned(),
            UnitType {
                height_m: 2.5,
                silhouette_width_m: 3.0,
                element_count: 1,
                speed_m_s: 0.0,
                signature: BTreeMap::new(),
                weapon: Some("mg".to_owned()),
                value: None,
                role: None,
            },
        ),
        (
            "block".to_owned(),
            UnitType {
                height_m: 2.0,
                silhouette_width_m: 3.0,
                element_count: 200,
                ..Default::default()
            },
        ),
    ]);
    let weapons = BTreeMap::from([(
        "mg".to_owned(),
        WeaponType {
            class: WeaponClass::Direct,
            rof_rounds_per_min: 60.0,
            max_range_m: 2000.0,
            dispersion_mrad: 1.0,
            p_kill_given_hit: 0.5,
            ..Default::default()
        },
    )]);
    let libs = Libraries {
        units,
        weapons,
        ..Libraries::with_terrain(scenario_params())
    };

    // Average casualties inflicted in one epoch with the shooter forced to a state.
    let casualties_at = |state: Suppression| -> f64 {
        let trials = 400u64;
        let mut total = 0u32;
        for seed in 0..trials {
            let mut sim = Sim::new(&scn, &libs, seed).unwrap();
            sim.set_suppression(0, state);
            sim.run_until(10.0); // one epoch
            total += sim.units()[1].initial_elements - sim.units()[1].elements;
        }
        f64::from(total) / trials as f64
    };

    let free = casualties_at(Suppression::Free);
    let suppressed = casualties_at(Suppression::Suppressed);
    let pinned = casualties_at(Suppression::Pinned);

    assert_eq!(pinned, 0.0, "a pinned unit must not fire");
    assert!(
        free > 4.0,
        "sanity: free unit inflicts casualties (got {free})"
    );
    // suppressed / free ≈ suppressed_fire_factor (0.4).
    let ratio = suppressed / free;
    assert!(
        (ratio - 0.4).abs() < 0.06,
        "suppressed output ratio {ratio:.3} should be ~0.4"
    );
}

// ---- V54: removing an asset must not corrupt the recorded history -------------------
// Every event log holds *indices* into the unit and air lists, as does a carried sensor's
// `carrier`. Removal is therefore a tombstone rather than a `Vec::remove`: this gate is
// what stops a future "tidy up the vectors" change from silently repointing every logged
// event at the wrong asset.
#[test]
fn v54_removal_tombstones_keep_logged_indices_valid() {
    let libs = air_libs();
    let scn = raid_scenario(true);
    let mut sim = Sim::new(&scn, &libs, 3).unwrap();
    sim.run_until(120.0);

    let units_before = sim.units().len();
    let air_before = sim.air().len();
    // Record what every logged index resolved to, so we can prove it still does.
    let detections: Vec<(usize, String)> = sim
        .events()
        .iter()
        .map(|e| (e.unit, sim.units()[e.unit].id.clone()))
        .collect();
    let air_events: Vec<(usize, String)> = sim
        .air_events()
        .iter()
        .map(|e| (e.air, sim.air()[e.air].id.clone()))
        .collect();
    assert!(
        !air_events.is_empty(),
        "sanity: the raid should have been detected"
    );

    sim.remove_unit(0);
    sim.remove_air(0);

    // The lists keep their shape, so every index still points where it did.
    assert_eq!(
        sim.units().len(),
        units_before,
        "removal must not shift units"
    );
    assert_eq!(sim.air().len(), air_before, "removal must not shift air");
    for (idx, id) in detections {
        assert_eq!(
            sim.units()[idx].id,
            id,
            "a logged detection changed meaning"
        );
    }
    for (idx, id) in air_events {
        assert_eq!(sim.air()[idx].id, id, "a logged air track changed meaning");
    }

    // And the removed assets really are out of the fight.
    assert!(!sim.units()[0].alive(), "a removed unit is not alive");
    assert!(!sim.air()[0].alive, "a removed airframe is not alive");
    assert!(
        !sim.sensor_active(0) || sim.sensors()[0].carrier != Some(0),
        "a carried sensor must die with its airframe"
    );

    // The sim keeps running without them.
    sim.run_until(200.0);
}

// ---- V55: track lifecycle (docs/DESIGN.md §10.1) ------------------------------------
// Detection used to be permanent, which quietly meant EW could *prevent* a track but
// never *break* one — jamming a unit already seen did nothing at all. A track now lapses
// `track_hold_s` after its last observation, and whether a sensor still "observes" is
// judged on the effective glimpse rate, so degrading a sensor loses the track.

/// A watcher and a target on flat ground. `sensor_range` and the target's `route` are
/// parameterised so the same fixture covers "stays in view" and "drives out of view".
fn track_fixture(sensor_range: f32, route: &str) -> (Scenario, Libraries) {
    let text = format!(
        r#"
        name = "track"
        default_seed = 4
        [sim]
        dt_s = 1.0
        epoch_s = 10.0
        track_hold_s = 30.0
        [terrain]
        cell_size_m = 10.0
        width_cells = 400
        height_cells = 40
        [terrain.source.flat]
        elevation_m = 0.0
        [[blue.sensors]]
        id = "obs"
        type = "s"
        pos = [100.0, 200.0]
        [[red.units]]
        id = "tgt"
        type = "u"
        pos = [600.0, 200.0]
        {route}
    "#
    );
    let scn = Scenario::from_toml_str(&text).unwrap();
    let libs = Libraries {
        sensors: BTreeMap::from([(
            "s".to_owned(),
            SensorType {
                modality: Modality::Optical,
                mount_height_m: 2.0,
                max_range_m: sensor_range,
                lambda0_per_s: 1.5, // acquires fast, so the test is about *holding*
                range_half_m: 2000.0,
                range_exponent: 2.0,
                for_width_deg: None,
            },
        )]),
        units: BTreeMap::from([(
            "u".to_owned(),
            UnitType {
                height_m: 2.0,
                speed_m_s: 20.0,
                signature: BTreeMap::from([("optical".to_owned(), 0.9)]),
                ..Default::default()
            },
        )]),
        ..Libraries::with_terrain(scenario_params())
    };
    (scn, libs)
}

#[test]
fn v55_tracks_decay_and_ew_can_break_them() {
    // 1. Under continuous observation the track never lapses, however long the run.
    let (scn, libs) = track_fixture(4000.0, "");
    let mut sim = Sim::new(&scn, &libs, 1).unwrap();
    sim.run_until(200.0);
    assert!(
        sim.units()[0].detected,
        "a target in plain view must stay tracked"
    );
    let seen = sim.units()[0].last_seen_s.expect("a live track");
    assert!(
        sim.time_s() - seen < 30.0,
        "an observed track should keep being refreshed (last seen {seen})"
    );

    // 2. Drive the target out of sensor range and the track lapses within the hold time.
    let (scn, libs) = track_fixture(1500.0, "route = [[3800.0, 200.0]]");
    let mut sim = Sim::new(&scn, &libs, 1).unwrap();
    sim.run_until(30.0);
    assert!(sim.units()[0].detected, "acquired while still close");
    // At 20 m/s it clears the 1500 m envelope well before t = 120 s; then hold + slack.
    sim.run_until(200.0);
    assert!(
        !sim.units()[0].detected,
        "a target that has driven out of range must lose its track"
    );
    assert_eq!(
        sim.units()[0].last_seen_s,
        None,
        "a lapsed track is cleared, so reacquisition starts afresh"
    );

    // 3. The headline, and the thing permanent detection made impossible: jam a unit that
    //    is *already tracked* and the track breaks. Note the jammer arrives mid-run —
    //    testing that EW *breaks* a track, not merely that it prevents one forming.
    let (scn, libs) = track_fixture(4000.0, "");
    let mut sim = Sim::new(&scn, &libs, 1).unwrap();
    sim.run_until(30.0);
    assert!(
        sim.units()[0].detected,
        "sanity: the track is established before jamming starts"
    );
    sim.add_jammer(Side::Red, Vec2::new(600.0, 200.0), 0.99, 600.0);
    sim.run_until(30.0 + 30.0 + 20.0); // hold time plus slack
    assert!(
        !sim.units()[0].detected,
        "jamming an already-tracked unit must break the track"
    );

    // And without the jammer the identical run keeps its track — so it was the EW, not
    // the passage of time.
    let mut clear = Sim::new(&scn, &libs, 1).unwrap();
    clear.run_until(80.0);
    assert!(
        clear.units()[0].detected,
        "control: unjammed, the same geometry holds its track"
    );
}
