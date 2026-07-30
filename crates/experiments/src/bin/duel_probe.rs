//! Diagnostic: load the default scenario, report the geometry of every
//! (blue sensor → red unit) pair — range, LOS, τ, λ — then run the clock and report
//! detections. Answers "why so few detections?" without opening the window.
//!
//! Run: `cargo run -p experiments --bin duel_probe`

use glam::Vec2;
use sim_core::scenario::{Libraries, Scenario};
use sim_core::sensing::detection_rate;
use sim_core::sim::{Side, Sim};
use std::path::Path;

fn main() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scenarios");
    let scenario = Scenario::load(&dir.join("default.toml")).expect("load default");
    let libs = Libraries::load_dir(&dir).unwrap();

    let sim = Sim::new(&scenario, &libs, scenario.default_seed).expect("resolve scenario");
    let terrain = sim.terrain();

    println!("=== pairwise geometry (blue sensors → red units) ===");
    for s in sim.sensors().iter().filter(|s| s.side == Side::Blue) {
        for u in sim.units().iter().filter(|u| u.side == Side::Red) {
            let r = s.pos.distance(u.pos);
            let l = sim_core::los::line_of_sight(
                terrain,
                s.pos,
                s.stats.mount_height_m,
                u.pos,
                u.stats.height_m,
            );
            let lambda = detection_rate(terrain, &s.stats, s.pos, s.facing_deg, &u.stats, u.pos);
            println!(
                "{:>10} -> {:<10} r={:>6.0}m  LOS={:<7} tau={:.2}  lambda={:.5}/s  {}",
                s.id,
                u.id,
                r,
                if l.clear { "clear" } else { "BLOCKED" },
                l.transmittance,
                lambda,
                if r > s.stats.max_range_m {
                    "(out of range)"
                } else {
                    ""
                },
            );
        }
    }

    println!("\n=== blue shooters → red units (range / LOS / in weapon band) ===");
    for s in sim
        .units()
        .iter()
        .filter(|u| u.side == Side::Blue && u.weapon.is_some())
    {
        let w = s.weapon.as_ref().unwrap();
        for u in sim.units().iter().filter(|u| u.side == Side::Red) {
            let r = s.pos.distance(u.pos);
            let clear = sim_core::los::visible(terrain, s.pos, 2.0, u.pos, u.stats.height_m);
            let in_band = r <= w.max_range_m && r >= w.min_range_m;
            println!(
                "{:>10} -> {:<10} r={:>6.0}m  LOS={:<7} band={}",
                s.id,
                u.id,
                r,
                if clear { "clear" } else { "BLOCKED" },
                in_band
            );
        }
    }

    // Run 10 minutes of sim and report detections.
    let mut sim = Sim::new(&scenario, &libs, scenario.default_seed).expect("resolve scenario");
    sim.run_until(600.0);
    println!("\n=== detections after {:.0} s ===", sim.time_s());
    if sim.events().is_empty() {
        println!("(none — every pair is out of range, hard-blocked, or too attenuated)");
    }
    for e in sim.events() {
        let s = &sim.sensors()[e.sensor];
        let u = &sim.units()[e.unit];
        println!(
            "t={:>4.0}s  {} spotted {} at {:?}",
            e.time_s,
            s.id,
            u.id,
            Vec2::new(e.unit_pos.x, e.unit_pos.y)
        );
    }

    println!("\n=== fires ===");
    if sim.fire_events().is_empty() {
        println!("(no shooter engaged a target in weapon range)");
    }
    for e in sim.fire_events() {
        let sh = &sim.units()[e.shooter];
        let tg = &sim.units()[e.target];
        println!(
            "t={:>4.0}s  {} hit {} for {} casualties{}",
            e.time_s,
            sh.id,
            tg.id,
            e.casualties,
            if e.killed { "  [DESTROYED]" } else { "" }
        );
    }
    println!("\n=== final unit states ===");
    for u in sim.units() {
        println!(
            "  {:<10} {:?}  {}/{} elements  {:?}",
            u.id, u.side, u.elements, u.initial_elements, u.suppression
        );
    }
}
