//! V70 — what blocks an indirect shooter is a lapsed track, not terrain.
//! `docs/DESIGN.md` §2, §10.1, §13.4.
//!
//! V66 established that line of sight and range **block** a pairing rather than merely
//! lowering its score, so a shooter whose priority target is masked by a ridge falls
//! through to what it can actually engage, and a lock is released when its target becomes
//! unengageable. Its fixture exercises **direct** fire throughout.
//!
//! Indirect fire is eligible on different terms, and this gate says so in both directions.
//! A shell arcs, so a ridge is irrelevant to it — what it needs is a *track*, held by
//! somebody on its own side. So the asymmetry is:
//!
//! | | blocked by terrain | blocked by a lapsed track |
//! |---|---|---|
//! | direct | yes | no — it needs no track at all |
//! | indirect | **no** | **yes**, `track_hold_s` after the last look |
//!
//! Both halves matter. Without the first, artillery would inherit the sightline
//! restrictions of a tank; without the second, a lock would outlive the information that
//! justified it and guns would keep firing at a position nobody has confirmed for minutes.

use sim_core::scenario::{Libraries, Scenario};
use sim_core::sim::{FireTarget, Side, Sim};
use std::collections::BTreeMap;

use sim_core::fires::{WeaponClass, WeaponType};
use sim_core::sensing::{Modality, SensorType, UnitType};
use validation::scenario_params;

const GUN_X: f32 = 200.0;
const LANE_Y: f32 = 600.0;
/// East of the ridge at the map centre, so the gun has no sightline to it.
const TARGET_X: f32 = 3000.0;
/// Also east of the ridge, close enough to watch the target the whole time.
const OBSERVER_X: f32 = 2600.0;

/// Guns that differ **only** in whether their weapon needs a sightline.
fn libraries() -> Libraries {
    let mut libs = Libraries::with_terrain(scenario_params());
    for (name, class) in [
        ("gun_indirect", WeaponClass::Indirect),
        ("gun_direct", WeaponClass::Direct),
    ] {
        libs.weapons.insert(
            name.to_owned(),
            WeaponType {
                class,
                rof_rounds_per_min: 12.0,
                // Both reach far past the target, so range can never be the reason either
                // one holds its fire — leaving line of sight as the only difference.
                max_range_m: 8000.0,
                dispersion_mrad: 0.5,
                p_kill_given_hit: 0.9,
                // Deliberately almost harmless: expected damage per round is
                // R^2/(sigma^2 + R^2) ~ 0.01 at this CEP, so the target survives long
                // enough for a track to lapse under it. This gate is about *eligibility*,
                // and a target that dies releases the lock for the wrong reason — which is
                // exactly what an earlier cut of this fixture measured.
                cep_m: 400.0,
                lethal_radius_m: 35.0,
                ..Default::default()
            },
        );
    }
    for (unit, weapon) in [("howitzer", "gun_indirect"), ("tank", "gun_direct")] {
        libs.units.insert(
            unit.to_owned(),
            UnitType {
                height_m: 2.5,
                silhouette_width_m: 3.0,
                element_count: 1,
                signature: BTreeMap::from([("optical".to_owned(), 0.5)]),
                weapon: Some(weapon.to_owned()),
                ..Default::default()
            },
        );
    }
    libs.units.insert(
        "target".to_owned(),
        UnitType {
            height_m: 2.5,
            silhouette_width_m: 3.0,
            element_count: 60,
            signature: BTreeMap::from([("optical".to_owned(), 0.9)]),
            ..Default::default()
        },
    );
    libs.sensors.insert(
        "observer".to_owned(),
        SensorType {
            modality: Modality::Optical,
            mount_height_m: 6.0,
            max_range_m: 2000.0,
            lambda0_per_s: 2.0,
            range_half_m: 1500.0,
            range_exponent: 2.0,
            for_width_deg: None,
        },
    );
    libs
}

/// One gun west of a ridge, the target east of it, and an observer east of it too — so the
/// target is *tracked* by the side while being *invisible* to the gun.
fn masked_target(gun_type: &str) -> Sim {
    let scn = Scenario::from_toml_str(&format!(
        r#"
        name = "indirect-eligibility"
        default_seed = 8
        [sim]
        dt_s = 1.0
        epoch_s = 10.0
        [terrain]
        cell_size_m = 10.0
        width_cells = 400
        height_cells = 120
        [terrain.source.layers]
        base = {{ flat = {{ elevation_m = 0.0 }} }}
        # A north-south ridge through the map centre (x = 2000 m).
        [[terrain.source.layers.apply]]
        ridge = {{ bearing_deg = 90.0, crest_m = 90.0, width_m = 300.0, offset_m = 0.0 }}
        [[blue.units]]
        id = "shooter"
        type = "{gun_type}"
        pos = [{GUN_X}, {LANE_Y}]
        [[blue.sensors]]
        id = "obs"
        type = "observer"
        pos = [{OBSERVER_X}, {LANE_Y}]
        [[red.units]]
        id = "tgt"
        type = "target"
        pos = [{TARGET_X}, {LANE_Y}]
    "#
    ))
    .unwrap();
    Sim::new(&scn, &libraries(), 8).expect("fixture builds")
}

/// Run epochs until the shooter holds a lock, or give up.
fn lock_within(sim: &mut Sim, epochs: usize) -> Option<FireTarget> {
    for _ in 0..epochs {
        sim.run_until(sim.time_s() + 10.0);
        if let Some(t) = sim.units()[0].engaging {
            return Some(t);
        }
    }
    None
}

// The fixture only means anything if the ridge really does mask the target from the gun
// while the observer keeps seeing it. Asserted rather than assumed, because a terrain recipe
// that silently did nothing would make every test below pass for the wrong reason.
#[test]
fn v70_the_ridge_masks_the_shooter_but_not_the_observer() {
    let sim = masked_target("howitzer");
    let gun_to_target = sim_core::los::visible(
        sim.terrain(),
        glam::Vec2::new(GUN_X, LANE_Y),
        2.0,
        glam::Vec2::new(TARGET_X, LANE_Y),
        2.5,
    );
    let obs_to_target = sim_core::los::visible(
        sim.terrain(),
        glam::Vec2::new(OBSERVER_X, LANE_Y),
        6.0,
        glam::Vec2::new(TARGET_X, LANE_Y),
        2.5,
    );
    assert!(!gun_to_target, "the ridge must mask the gun's sightline");
    assert!(obs_to_target, "the observer must still see the target");
}

// Half one: a shell arcs. The howitzer engages a target it cannot see, because the side
// holds a track on it — and the direct-fire gun in the identical position does not.
#[test]
fn v70_indirect_fire_is_not_blocked_by_terrain_but_direct_fire_is() {
    let mut indirect = masked_target("howitzer");
    assert_eq!(
        lock_within(&mut indirect, 8),
        Some(FireTarget::Unit(1)),
        "indirect fire needs a track, not a sightline: the ridge is irrelevant to it"
    );

    let mut direct = masked_target("tank");
    assert_eq!(
        lock_within(&mut direct, 8),
        None,
        "direct fire needs the sightline the ridge is blocking, so this gun must hold fire"
    );
}

// Half two, and the asymmetry's other side: what *does* stop the howitzer is losing the
// track. Jamming the observer drives the glimpse rate below the maintenance threshold, the
// track ages out `track_hold_s` after its last good look, and the lock goes with it.
#[test]
fn v70_an_indirect_lock_is_released_when_the_track_lapses() {
    let mut sim = masked_target("howitzer");
    assert_eq!(
        lock_within(&mut sim, 8),
        Some(FireTarget::Unit(1)),
        "the gun should be engaging before anything is taken away"
    );

    // Red jams its own unit: `Sim::jamming_at` folds the *target's* side's jammers, because
    // a jammer protecting Red degrades Blue's sensing of Red (§11.1).
    sim.add_jammer(Side::Red, glam::Vec2::new(TARGET_X, LANE_Y), 1.0, 1500.0);

    // The target must survive this, or the lock would be released because it died rather
    // than because the track lapsed — which is what an earlier cut of this fixture actually
    // measured, and it passed.
    sim.run_until(sim.time_s() + 120.0);
    assert!(
        sim.units()[1].alive(),
        "the target must still be alive, or this gate proves nothing about tracks"
    );
    assert!(
        !sim.units()[1].detected,
        "the jammer should have prevented any refresh, so the track must have lapsed"
    );
    assert_eq!(
        sim.units()[0].engaging,
        None,
        "an indirect lock outlives neither its track nor the information behind it"
    );
}
