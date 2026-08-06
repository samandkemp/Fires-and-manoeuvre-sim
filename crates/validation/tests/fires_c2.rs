//! V63 — ground fires can be made to depend on C2. `docs/DESIGN.md` §11.3.
//!
//! §10.2 let a side coordinate its ground fires for free, while §11 made air defence pay
//! for a C2 post. That asymmetry was deliberate — a battlegroup does share one fire-control
//! net, where point-defence batteries genuinely do not — but it was an argument, not a
//! modelled thing, and so could not be measured.
//!
//! `[sim] fires_need_c2` makes it modelled. **Off by default**, because turning it on
//! unconditionally would silently reduce every existing scenario to `independent` and
//! re-baseline the Phase 10 allocation result, V56 and V39 at once, for a reason invisible
//! in the scenario files. As a dial, the cost of losing the net becomes a number.
//!
//! The fixture is V56's, which is the point: two guns that can both reach two six-element
//! targets. Coordinated, they take one each. Independent, they both pile onto the nearer —
//! the pre-Phase-10 behaviour. So "how many distinct targets took casualties in the first
//! epoch" reads directly as "is this side coordinating".

use sim_core::c2::C2Type;
use sim_core::fires::{WeaponClass, WeaponType};
use sim_core::scenario::{Libraries, Scenario};
use sim_core::sensing::UnitType;
use sim_core::sim::Sim;
use std::collections::BTreeMap;
use validation::scenario_params;

/// Where the guns sit. The post, when there is one, sits on top of them.
const GUN_X: f32 = 200.0;
const GUN_Y: f32 = 300.0;

fn libraries(coordination_range_m: f32) -> Libraries {
    Libraries {
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
                    // Six elements, so one gun cannot finish a target in an epoch and
                    // piling on is genuinely wasteful.
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
                // As V56: the fire log records casualties, not shots, so a gun that engages
                // and misses leaves no trace. Reliable lethality removes the ambiguity
                // between "did not engage" and "engaged and missed".
                rof_rounds_per_min: 60.0,
                max_range_m: 3000.0,
                dispersion_mrad: 0.5,
                p_kill_given_hit: 0.9,
                ..Default::default()
            },
        )]),
        c2: BTreeMap::from([(
            "post".to_owned(),
            C2Type {
                coordination_range_m,
                ..Default::default()
            },
        )]),
        ..Libraries::with_terrain(scenario_params())
    }
}

/// Two guns, two targets, and optionally a post over the guns.
fn two_on_two(fires_need_c2: bool, post: bool, coordination_range_m: f32) -> Sim {
    let mut s = format!(
        r#"
        name = "fires-c2"
        default_seed = 3
        [sim]
        dt_s = 1.0
        epoch_s = 10.0
        allocation = "optimal"
        max_shooters_per_target = 3
        fires_need_c2 = {fires_need_c2}
        [terrain]
        cell_size_m = 10.0
        width_cells = 200
        height_cells = 64
        [terrain.source.flat]
        elevation_m = 0.0
        [[blue.units]]
        id = "gun-a"
        type = "gun"
        pos = [{GUN_X}, {GUN_Y}]
        [[blue.units]]
        id = "gun-b"
        type = "gun"
        pos = [{GUN_X}, 340.0]
        [[red.units]]
        id = "near"
        type = "target"
        pos = [900.0, {GUN_Y}]
        [[red.units]]
        id = "far"
        type = "target"
        pos = [1100.0, 340.0]
    "#
    );
    if post {
        s.push_str(&format!(
            r#"
        [[blue.c2]]
        id = "cp"
        type = "post"
        pos = [{GUN_X}, {GUN_Y}]
        "#
        ));
    }
    let scn = Scenario::from_toml_str(&s).unwrap();
    Sim::new(&scn, &libraries(coordination_range_m), 3).unwrap()
}

/// Distinct Red units that took casualties in the **first** epoch.
///
/// One epoch, not the whole battle: given long enough, uncoordinated guns reach the second
/// target anyway by destroying the first and moving on. What coordination buys is
/// *simultaneity*, so the measurement has to be simultaneous too.
fn targets_hit_first_epoch(sim: &mut Sim) -> usize {
    sim.run_until(10.0);
    let mut t: Vec<usize> = sim
        .fire_events()
        .iter()
        .filter_map(|e| e.target.unit())
        .collect();
    t.sort_unstable();
    t.dedup();
    t.len()
}

// V63 (headline): with the dial on, a side with a post coordinates and a side without one
// does not. Same guns, same targets, same solver — the only difference is whether anything
// is tying the shooters together, exactly as for air defence (V59).
#[test]
fn v63_a_post_is_what_lets_ground_fires_coordinate() {
    let mut with_post = two_on_two(true, true, 2000.0);
    assert_eq!(
        targets_hit_first_epoch(&mut with_post),
        2,
        "under a post, two guns should engage two targets"
    );

    let mut no_post = two_on_two(true, false, 2000.0);
    assert_eq!(
        targets_hit_first_epoch(&mut no_post),
        1,
        "with no post and the dial on, the guns should each take the nearest"
    );
}

// V63 (identity half, §7.4): with the dial off — the default — the C2 lists are never
// consulted, so a scenario coordinates exactly as it did before this existed. Checked with
// a post present *and* absent, since neither may make any difference.
#[test]
fn v63_the_dial_off_is_an_exact_identity() {
    let log = |sim: &mut Sim| -> Vec<(usize, sim_core::sim::FireTarget, u32)> {
        sim.run_until(60.0);
        sim.fire_events()
            .iter()
            .map(|e| (e.shooter, e.target, e.casualties))
            .collect()
    };
    let baseline = log(&mut two_on_two(false, false, 2000.0));
    let with_a_post = log(&mut two_on_two(false, true, 2000.0));
    assert_eq!(
        baseline, with_a_post,
        "with fires_need_c2 off, a C2 post must not touch ground fires at all"
    );
    assert!(!baseline.is_empty(), "the fixture must actually shoot");
}

// V63 (partial-net half): the net is per **shooter**, not per side. Three guns and three
// targets, with a post whose radius reaches two of them:
//
//   post covers all three  ->  3 targets engaged (fully coordinated)
//   post covers two        ->  2               (the loose gun duplicates)
//   no post                ->  1               (everyone takes the nearest)
//
// The middle row is the claim. It also shows what being outside the net costs: the loose
// gun still *fires* — it is not silenced, only uninformed — but it fires at a target one of
// the netted guns has already destroyed, so its rounds leave no trace at all. That wasted
// volley is exactly what coordination buys back.
#[test]
fn v63_the_net_is_per_shooter_not_per_side() {
    /// Guns at y = 300/340/380; the post sits at 320, so a small radius reaches the first
    /// two and not the third.
    fn three_guns(post_radius_m: Option<f32>) -> Sim {
        let post = post_radius_m.map_or_else(String::new, |_| {
            format!(
                r#"
        [[blue.c2]]
        id = "cp"
        type = "post"
        pos = [{GUN_X}, 320.0]
        "#
            )
        });
        let scn = Scenario::from_toml_str(&format!(
            r#"
        name = "fires-c2-partial"
        default_seed = 3
        [sim]
        dt_s = 1.0
        epoch_s = 10.0
        allocation = "optimal"
        max_shooters_per_target = 3
        fires_need_c2 = true
        [terrain]
        cell_size_m = 10.0
        width_cells = 200
        height_cells = 64
        [terrain.source.flat]
        elevation_m = 0.0
        [[blue.units]]
        id = "gun-a"
        type = "gun"
        pos = [{GUN_X}, 300.0]
        [[blue.units]]
        id = "gun-b"
        type = "gun"
        pos = [{GUN_X}, 340.0]
        [[blue.units]]
        id = "gun-c"
        type = "gun"
        pos = [{GUN_X}, 380.0]
        [[red.units]]
        id = "near"
        type = "target"
        pos = [900.0, 340.0]
        [[red.units]]
        id = "mid"
        type = "target"
        pos = [1150.0, 340.0]
        [[red.units]]
        id = "far"
        type = "target"
        pos = [1400.0, 340.0]
        {post}
    "#
        ))
        .unwrap();
        Sim::new(&scn, &libraries(post_radius_m.unwrap_or(0.0)), 3).unwrap()
    }

    // 30 m from the post at y = 320 reaches gun-a and gun-b (20 m each), not gun-c (60 m).
    assert_eq!(
        targets_hit_first_epoch(&mut three_guns(Some(3000.0))),
        3,
        "all three netted: one gun per target"
    );
    assert_eq!(
        targets_hit_first_epoch(&mut three_guns(Some(30.0))),
        2,
        "two netted, one loose: the loose gun duplicates onto the nearest"
    );
    assert_eq!(
        targets_hit_first_epoch(&mut three_guns(None)),
        1,
        "nobody netted: everyone takes the nearest, the pre-Phase-10 rule"
    );
}

// V63 (jamming half): the ground net uses the same degraded radius air defence does
// (§11.2), so jamming the post breaks ground coordination too — one mechanism, not two.
//
// Tested at two powers, because the claim is that jamming *scales* the radius rather than
// switching the link off. A jammer strong enough to pull 2000 m down to 1000 m still
// leaves both guns inside; one that pulls it to 20 m does not.
#[test]
fn v63_jamming_the_post_breaks_ground_coordination() {
    let scenario = |power: f32| -> Sim {
        let jammer = if power > 0.0 {
            format!(
                r#"
        [[red.jammers]]
        pos = [{GUN_X}, {GUN_Y}]
        power = {power}
        radius_m = 1500.0
        "#
            )
        } else {
            String::new()
        };
        let scn = Scenario::from_toml_str(&format!(
            r#"
        name = "fires-c2-jam"
        default_seed = 3
        [sim]
        dt_s = 1.0
        epoch_s = 10.0
        allocation = "optimal"
        max_shooters_per_target = 3
        fires_need_c2 = true
        [terrain]
        cell_size_m = 10.0
        width_cells = 200
        height_cells = 64
        [terrain.source.flat]
        elevation_m = 0.0
        [[blue.units]]
        id = "gun-a"
        type = "gun"
        pos = [{GUN_X}, {GUN_Y}]
        [[blue.units]]
        id = "gun-b"
        type = "gun"
        pos = [{GUN_X}, 340.0]
        [[blue.c2]]
        id = "cp"
        type = "post"
        pos = [{GUN_X}, {GUN_Y}]
        [[red.units]]
        id = "near"
        type = "target"
        pos = [900.0, {GUN_Y}]
        [[red.units]]
        id = "far"
        type = "target"
        pos = [1100.0, 340.0]
        {jammer}
    "#
        ))
        .unwrap();
        Sim::new(&scn, &libraries(2000.0), 3).unwrap()
    };

    let mut clear = scenario(0.0);
    assert_eq!(targets_hit_first_epoch(&mut clear), 2, "clear air: netted");

    // 0.5 leaves half of 2000 m. Both guns are within 40 m of the post, so the net holds —
    // the link degraded and the defence did not care, which is the behaviour that stops
    // "jammed" being read as "off".
    let mut light = scenario(0.5);
    assert_eq!(
        targets_hit_first_epoch(&mut light),
        2,
        "a 1000 m radius still covers both guns"
    );

    // 0.99 leaves 20 m. gun-a sits on the post and stays in; gun-b at 40 m falls out, and
    // the side stops coordinating.
    let mut heavy = scenario(0.99);
    assert_eq!(
        targets_hit_first_epoch(&mut heavy),
        1,
        "a 20 m radius drops gun-b out of the net"
    );
}
