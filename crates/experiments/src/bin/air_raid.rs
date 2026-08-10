//! How much of a drone raid leaks through an air defence, and what decides it.
//! Model: `docs/DESIGN.md` §9.5.
//!
//! The cueing clock starts at *detection*, not envelope entry. For a drone detected `D`
//! seconds before it enters and spending `W` seconds inside,
//! `W_eff = max(0, W − max(0, L + R − D))`, so the critical latency is `L* = W + D − R`
//! and early warning trades one-for-one against comms delay. Sweep 1a pins `D = 0`
//! (radar range = gun range) to isolate the simple form; 1b sweeps `D`. Both print the
//! closed form next to the measurement.
//!
//! Sweeps 2 and 3 cover the saturation levers: magazine depth and engagement channels
//! against raid size.
//!
//! Run: `cargo run -p experiments --release --bin air_raid`

use glam::Vec2;
use sim_core::air::{AirType, AltitudeRef, FlightPlan};
use sim_core::air_defence::{
    critical_latency_s, effective_window_s, p_leak_gun, AdEngagement, AirDefenceType, RadarPosture,
};
use sim_core::scenario::{Libraries, Scenario};
use sim_core::sensing::{Modality, SensorType, UnitType};
use sim_core::sim::{Side, Sim};
use std::collections::BTreeMap;

const SEEDS: u64 = 400;
/// Drones start here and fly due west at the target.
const START_X: f32 = 6000.0;
const TARGET: Vec2 = Vec2::new(1000.0, 2500.0);
const DRONE_SPEED: f32 = 50.0;
const DRONE_ALT: f32 = 400.0;
/// Gun envelope, metres - also what sets the time a drone spends inside it.
const AD_RANGE: f32 = 2500.0;
const KILL_RATE: f32 = 0.35;
const REACTION_S: f32 = 2.0;
/// Slant range at which the drone releases. Note it must exceed `DRONE_ALT`: release
/// range is a *slant* range (§9.1), so a drone at 400 m that only releases inside 300 m
/// can never release at all - directly overhead it is still 400 m from the aim point.
const RELEASE_RANGE: f32 = 900.0;

/// Horizontal distance at which a level drone at `DRONE_ALT` is `slant_m` from a ground
/// point - the geometry that turns a slant envelope into a time in that envelope.
fn horizontal_at_slant(slant_m: f32) -> f32 {
    (slant_m * slant_m - DRONE_ALT * DRONE_ALT).max(0.0).sqrt()
}

/// Seconds of warning a radar of `radar_range_m` gives before the drone reaches the gun's
/// envelope - the warning lead `D` of the §9.5 timeline.
fn warning_lead_s(radar_range_m: f32) -> f32 {
    ((horizontal_at_slant(radar_range_m) - horizontal_at_slant(AD_RANGE)) / DRONE_SPEED).max(0.0)
}

fn main() {
    let scn = Scenario::from_toml_str(
        r#"
        name = "air_raid_sweep"
        [sim]
        dt_s = 1.0
        epoch_s = 10.0
        [terrain]
        cell_size_m = 10.0
        width_cells = 700
        height_cells = 500
        [terrain.source.flat]
        elevation_m = 0.0
    "#,
    )
    .unwrap();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scenarios");
    let libs = Libraries::load_dir(&dir).unwrap();
    // Flat ground and a fixed run-in, so the geometry is exactly analysable: the drone is
    // inside the envelope from AD_RANGE out to its release point.
    let mut sim = Sim::new(&scn, &libs, 1).unwrap();

    // A radar that acquires almost instantly, so the sweep isolates *cue* delay rather
    // than acquisition delay. Its **range** is a swept parameter: early warning is what
    // decides whether a slow cueing chain has time to work (see sweep 1b).
    let radar = |max_range_m: f32| SensorType {
        modality: Modality::Optical,
        mount_height_m: 4.0,
        max_range_m,
        lambda0_per_s: 5.0,
        range_half_m: 6000.0,
        range_exponent: 2.0,
        for_width_deg: None,
    };
    let drone = AirType {
        height_m: 1.5,
        cruise_speed_m_s: DRONE_SPEED,
        signature: BTreeMap::from([("optical".to_owned(), 0.8)]),
        payload: Some("guided_bomb".to_owned()),
        munitions: 1,
        release_range_m: RELEASE_RANGE,
        ..Default::default()
    };
    let target_unit = UnitType {
        height_m: 2.5,
        element_count: 8,
        signature: BTreeMap::new(),
        ..Default::default()
    };
    let gun = |magazine: u32, channels: u32, cue_latency_s: f32| AirDefenceType {
        engagement: AdEngagement::Gun {
            kill_rate_per_s: KILL_RATE,
        },
        max_range_m: AD_RANGE,
        max_alt_m: 3000.0,
        requires_los: true,
        reaction_time_s: REACTION_S,
        cue_latency_s,
        magazine,
        channels,
        ..Default::default()
    };

    // One raid: `raid_size` drones abreast, all inbound on the target. Returns the number
    // that survived to release a munition (the leakers).
    let mut run_raid = |raid_size: u32,
                        magazine: u32,
                        channels: u32,
                        cue_latency_s: f32,
                        self_cue: bool,
                        radar_range_m: f32,
                        seed: u64|
     -> u32 {
        sim.reset(seed);
        sim.add_unit("target", Side::Blue, TARGET, target_unit.clone(), None);
        sim.add_air_defence(
            "gun",
            Side::Blue,
            TARGET,
            gun(magazine, channels, cue_latency_s),
            // The radar transmits either way here: this probe varies the *cueing* route
            // (§9.5), not emission control (§12.3). They are separate flags now.
            RadarPosture {
                self_cue,
                emitting: true,
            },
            Some(radar(radar_range_m)),
        );
        for i in 0..raid_size {
            // Spread the raid across a 600 m front so the drones are distinguishable
            // targets rather than a single point.
            let y = TARGET.y + (i as f32 - (raid_size as f32 - 1.0) / 2.0) * 200.0;
            let idx = sim.add_air(
                &format!("d{i}"),
                Side::Red,
                Vec2::new(START_X, y),
                DRONE_ALT,
                AltitudeRef::Agl,
                180.0,
                drone.clone(),
                None,
                libs.weapons.get("guided_bomb").cloned(),
            );
            sim.set_flight_plan(idx, FlightPlan::route(vec![TARGET]));
        }
        // Long enough for the whole raid to cross the envelope or die trying.
        sim.run_until(300.0);
        sim.strike_events().len() as u32
    };

    // The time a single drone spends inside the envelope before releasing, from the
    // slant geometry: it enters where slant = AD_RANGE and releases where slant =
    // RELEASE_RANGE, both converted to horizontal distance at its cruising altitude.
    let window_s =
        (horizontal_at_slant(AD_RANGE) - horizontal_at_slant(RELEASE_RANGE)) / DRONE_SPEED;
    let l_star = critical_latency_s(window_s, 0.0, REACTION_S);

    println!("=== Counter-air sweep (docs/DESIGN.md §9.4-§9.5) ===");
    println!(
        "gun kill rate {KILL_RATE}/s, envelope {AD_RANGE:.0} m slant, release at \
         {RELEASE_RANGE:.0} m slant, drone {DRONE_SPEED:.0} m/s at {DRONE_ALT:.0} m"
    );
    println!(
        "time in envelope W = {window_s:.1} s, reaction R = {REACTION_S:.1} s \
         => critical latency L* = W - R = {l_star:.1} s\n"
    );

    // --- 1a. Cue latency vs leakage, against the closed form -------------------------
    // The radar reaches exactly as far as the gun, so a drone is acquired essentially as
    // it enters the envelope. That is the geometry the §9.5 closed form assumes, and it
    // isolates the cue delay from the early-warning lead time swept in 1b.
    println!("--- 1a. cue latency vs leakage (no early warning: radar range = gun range) ---");
    println!(
        "{:>9}  {:>9}  {:>10}  {:>12}",
        "latency", "W_eff", "leak(sim)", "leak(theory)"
    );
    for latency in [0.0f32, 10.0, 20.0, 25.0, 28.0, 30.0, l_star, l_star + 10.0] {
        let leaked: u32 = (0..SEEDS)
            .map(|seed| run_raid(1, 0, 1, latency, false, AD_RANGE, seed))
            .sum();
        let observed = f64::from(leaked) / SEEDS as f64;
        let w_eff = effective_window_s(window_s, 0.0, latency, REACTION_S);
        let theory = f64::from(p_leak_gun(KILL_RATE, w_eff));
        let bar = "#".repeat((observed * 40.0).round() as usize);
        println!("{latency:>9.1}  {w_eff:>9.1}  {observed:>10.3}  {theory:>12.3}  {bar}");
    }

    // Self-cueing is the same battery with the latency term switched off - the cleanest
    // statement of what an organic sensor is worth.
    let delay = l_star * 0.9; // deep enough into the curve to bite
    let self_cued: u32 = (0..SEEDS)
        .map(|seed| run_raid(1, 0, 1, delay, true, AD_RANGE, seed))
        .sum();
    let net_cued: u32 = (0..SEEDS)
        .map(|seed| run_raid(1, 0, 1, delay, false, AD_RANGE, seed))
        .sum();
    println!(
        "\nsame battery, {delay:.0} s comms delay: self-cued leaks {:.3}, \
         net-cued leaks {:.3}",
        f64::from(self_cued) / SEEDS as f64,
        f64::from(net_cued) / SEEDS as f64
    );
    println!("(an organic sensor is worth exactly the latency term it removes)\n");

    // --- 1b. Early warning buys the latency back -------------------------------------
    // The §9.5 clock starts at *detection*, not at envelope entry. A radar that reaches
    // further starts it earlier, so a cue can be ageing through the comms chain while the
    // drone is still inbound. Early-warning range and comms latency trade directly
    // against one another - the practical form of the sensor-to-shooter timeline, and
    // the reason 1a has to suppress early warning to see the closed form at all.
    let late = l_star + 20.0; // a latency that is fatal without early warning
    println!("--- 1b. early-warning range vs a fixed {late:.0} s comms latency ---");
    println!(
        "{:>12}  {:>9}  {:>9}  {:>10}  {:>12}",
        "radar range", "lead(s)", "W_eff", "leak(sim)", "leak(theory)"
    );
    for radar_range in [AD_RANGE, 3000.0, 3500.0, 4000.0, 5000.0, 6500.0] {
        let lead = warning_lead_s(radar_range);
        let leaked: u32 = (0..SEEDS)
            .map(|seed| run_raid(1, 0, 1, late, false, radar_range, seed))
            .sum();
        let observed = f64::from(leaked) / SEEDS as f64;
        // The *general* §9.5 form, with the warning lead carried explicitly.
        let w_eff = effective_window_s(window_s, lead, late, REACTION_S);
        let theory = f64::from(p_leak_gun(KILL_RATE, w_eff));
        let bar = "#".repeat((observed * 40.0).round() as usize);
        println!(
            "{radar_range:>12.0}  {lead:>9.1}  {w_eff:>9.1}  {observed:>10.3}  {theory:>12.3}  {bar}"
        );
    }
    println!(
        "(the chain only bites when the drone arrives before its own cue does: once the\n \
         warning lead exceeds the latency, a slow network costs nothing)\n"
    );

    // --- 2. Raid size vs magazine depth ----------------------------------------------
    println!("--- 2. raid size vs magazine depth (self-cued, 1 channel) ---");
    print!("{:>10}", "raid \\ mag");
    for magazine in [1u32, 2, 4, 8] {
        print!("{magazine:>8}");
    }
    println!("{:>10}", "unlimited");
    for raid in [1u32, 2, 4, 6] {
        print!("{raid:>10}");
        for magazine in [1u32, 2, 4, 8, 0] {
            let leaked: u32 = (0..SEEDS / 4)
                .map(|seed| run_raid(raid, magazine, 1, 0.0, true, AD_RANGE, seed))
                .sum();
            let mean = f64::from(leaked) / (SEEDS / 4) as f64;
            print!("{mean:>8.2}");
        }
        println!();
    }
    println!("(cells are mean leakers per raid - drones that survived to release)\n");

    // --- 3. Channels: what saturation actually costs ---------------------------------
    println!("--- 3. engagement channels vs raid size (self-cued, unlimited magazine) ---");
    print!("{:>10}", "raid \\ ch");
    for channels in [1u32, 2, 4] {
        print!("{channels:>8}");
    }
    println!();
    for raid in [2u32, 4, 6, 8] {
        print!("{raid:>10}");
        for channels in [1u32, 2, 4] {
            let leaked: u32 = (0..SEEDS / 4)
                .map(|seed| run_raid(raid, 0, channels, 0.0, true, AD_RANGE, seed))
                .sum();
            let mean = f64::from(leaked) / (SEEDS / 4) as f64;
            print!("{mean:>8.2}");
        }
        println!();
    }
    println!(
        "\nA single-channel battery engages one drone at a time, so a raid saturates it \
         however deep its magazine is."
    );
}
