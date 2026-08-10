//! V62 - the C2 link can be degraded, not only destroyed. `docs/DESIGN.md` §11.2.
//!
//! §11 made coordination an asset you can lose by having it killed. That left the link
//! itself binary and instant: inside the radius or not, from the first tick. Two things
//! now bear on it.
//!
//! **Jamming pulls the radius in.** An enemy jammer near the post scales its effective
//! coordination range by the [`sim_core::ew`] factor, so the batteries on the flanks fall
//! out of the net first while the one sitting on top of the post keeps talking. That is the
//! right shape - a link degrades with range against a noise floor, and raising the floor is
//! what a jammer does - and it gives the raid a *soft* counter beside SEAD's hard one.
//!
//! **Joining costs time.** `link_latency_s` is how long a battery must have been inside the
//! radius before it is actually in the net. Zero by default, so the pre-latency behaviour
//! is exactly recovered and a sweep can isolate either effect from the other.
//!
//! The fixture is V59's, unchanged: three single-channel batteries in a line, three drones
//! placed so one of them is nearest to all three. Nearest-first sends every battery at that
//! same drone; a working C2 post makes them cover one each. So "how many distinct drones
//! are under engagement" reads directly as "is the net working".

use sim_core::air::AirType;
use sim_core::air_defence::{AdEngagement, AirDefenceType};
use sim_core::c2::C2Type;
use sim_core::scenario::{Libraries, Scenario};
use sim_core::sensing::{Modality, SensorType};
use sim_core::sim::Sim;
use std::collections::BTreeMap;
use validation::scenario_params;

const BASE_X: f32 = 1000.0;
const BASE_Y: f32 = 1500.0;
const STANDOFF: f32 = 1200.0;
/// How far the flanking batteries sit from the post. The link has to shrink past this for
/// them to drop out, and the centre battery (at zero) must survive any amount of jamming.
const FLANK_M: f32 = 500.0;
/// The post's clear-air reach. Comfortably over the line, so nothing is marginal.
const POST_RANGE_M: f32 = 3000.0;

fn libraries(link_latency_s: f32) -> Libraries {
    Libraries {
        sensors: BTreeMap::from([(
            "radar".to_owned(),
            SensorType {
                modality: Modality::Optical,
                mount_height_m: 4.0,
                max_range_m: 8000.0,
                lambda0_per_s: 8.0,
                range_half_m: 8000.0,
                range_exponent: 2.0,
                for_width_deg: None,
            },
        )]),
        air: BTreeMap::from([(
            "drone".to_owned(),
            AirType {
                height_m: 2.0,
                cruise_speed_m_s: 0.0,
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
                // As V59: long enough that every battery has acquired every drone before
                // any may open, so the gate reads allocation and not acquisition luck.
                reaction_time_s: 25.0,
                magazine: 0,
                channels: 1,
                sensor: Some("radar".to_owned()),
                engagement: AdEngagement::Gun {
                    kill_rate_per_s: 5.0e-4,
                },
                ..Default::default()
            },
        )]),
        c2: BTreeMap::from([(
            "post".to_owned(),
            C2Type {
                coordination_range_m: POST_RANGE_M,
                link_latency_s,
                ..Default::default()
            },
        )]),
        ..Libraries::with_terrain(scenario_params())
    }
}

/// What sits on the map besides the three batteries, the post and the three drones.
#[derive(Clone, Copy, Default)]
struct Extras {
    /// A jammer at the post, and whose it is. `None` for clear air.
    jammer: Option<Jammer>,
    link_latency_s: f32,
}

#[derive(Clone, Copy)]
struct Jammer {
    /// `true` for a Red (enemy) jammer, `false` for a Blue (friendly) one.
    hostile: bool,
    power: f32,
}

fn raid(extras: Extras) -> Sim {
    let mut s = format!(
        r#"
        name = "c2-link"
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
        [[blue.c2]]
        id = "cp"
        type = "post"
        pos = [{BASE_X}, {BASE_Y}]
    "#,
        BASE_Y - FLANK_M,
        BASE_Y + FLANK_M
    );
    if let Some(j) = extras.jammer {
        // Sitting exactly on the post, so the degradation is the jammer's full power and
        // the geometry is not doing any of the work.
        let side = if j.hostile { "red" } else { "blue" };
        s.push_str(&format!(
            r#"
        [[{side}.jammers]]
        pos = [{BASE_X}, {BASE_Y}]
        power = {}
        radius_m = 1500.0
        "#,
            j.power
        ));
    }
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
    Sim::new(&scn, &libraries(extras.link_latency_s), 3).unwrap()
}

/// How many *distinct* airframes the defence has under engagement - the direct reading of
/// whether the net is working.
fn distinct(sim: &Sim) -> usize {
    let mut t: Vec<usize> = sim
        .air_defence()
        .iter()
        .filter_map(|ad| ad.engagements.first().map(|e| e.target))
        .collect();
    t.sort_unstable();
    t.dedup();
    t.len()
}

// V62 (headline): jamming the post's link decoheres the defence without touching a single
// battery, a post, a magazine or an envelope - the soft twin of killing the post (§12.2).
#[test]
fn v62_jamming_the_link_decoheres_the_defence() {
    let mut clear = raid(Extras::default());
    clear.run_until(40.0);
    assert_eq!(
        distinct(&clear),
        3,
        "a clear link must still split the raid three ways"
    );

    // 0.9 at the centre leaves a tenth of the radius - 300 m - so the flanking batteries
    // at 500 m drop out and only the one sitting on the post stays in the net.
    let mut jammed = raid(Extras {
        jammer: Some(Jammer {
            hostile: true,
            power: 0.9,
        }),
        ..Default::default()
    });
    jammed.run_until(40.0);
    assert!(
        distinct(&jammed) < 3,
        "jamming the post must break the net; still covering {} drones",
        distinct(&jammed)
    );

    // Nothing was destroyed. The whole point is that this costs the attacker no ordnance.
    for (a, b) in clear.air_defence().iter().zip(jammed.air_defence()) {
        assert_eq!(a.elements, b.elements, "jamming must not kill a battery");
    }
    assert!(jammed.c2()[0].alive(), "jamming must not kill the post");
    assert_eq!(
        clear.air().iter().filter(|a| a.alive).count(),
        3,
        "the fixture must not be resolving by attrition"
    );
}

// V62 (side half): a jammer degrades the *enemy's* link, not its own side's. Same asset,
// same dials, opposite side of the argument from the sensing case (§8.1) - and getting
// this backwards would silently make EW self-defeating.
#[test]
fn v62_a_friendly_jammer_does_not_cut_its_own_net() {
    let mut friendly = raid(Extras {
        jammer: Some(Jammer {
            hostile: false,
            power: 0.9,
        }),
        ..Default::default()
    });
    friendly.run_until(40.0);
    assert_eq!(
        distinct(&friendly),
        3,
        "Blue's own jammer must not cut Blue's C2 net"
    );
}

// V62 (identity half, §7.4): a jammer of zero power runs the whole new arithmetic - the
// link-quality fold, the scaled radius, the latency gate - and must change nothing at all.
// Stronger than simply omitting the jammer, which would skip the code under test.
#[test]
fn v62_a_powerless_jammer_is_an_exact_identity() {
    let engagements = |sim: &Sim| -> Vec<Option<usize>> {
        sim.air_defence()
            .iter()
            .map(|ad| ad.engagements.first().map(|e| e.target))
            .collect()
    };
    let mut clear = raid(Extras::default());
    let mut inert = raid(Extras {
        jammer: Some(Jammer {
            hostile: true,
            power: 0.0,
        }),
        ..Default::default()
    });
    clear.run_until(40.0);
    inert.run_until(40.0);
    assert_eq!(
        engagements(&clear),
        engagements(&inert),
        "a zero-power jammer must leave the allocation untouched"
    );

    // And the allocation is a proper cover: three batteries, three different drones, one
    // each. The *permutation* is not asserted - which battery takes which drone is a
    // solver detail, and pinning it would make an unrelated tie-break look like a defect.
    let taken = engagements(&clear);
    assert!(taken.iter().all(Option::is_some), "every battery engaged");
    assert_eq!(distinct(&clear), 3, "and all three drones are covered");
}

// V62 (latency half): joining the net costs time, and a battery that is not yet in it
// falls back to nearest-first - so a late link cannot retrospectively undo the duplicated
// engagements a defence has already committed to. That consequence is the interesting one.
#[test]
fn v62_joining_the_net_costs_time() {
    // Longer than the observation window, so the batteries never get into the net.
    let mut slow = raid(Extras {
        link_latency_s: 300.0,
        ..Default::default()
    });
    slow.run_until(40.0);
    assert_eq!(
        distinct(&slow),
        1,
        "before the link comes up, batteries must act on their own: {:?}",
        slow.air_defence()
            .iter()
            .map(|ad| ad.engagements.first().map(|e| e.target))
            .collect::<Vec<_>>()
    );

    // The ready time is set once, on coming under coverage, and not restarted every tick -
    // which would make any latency permanent.
    for ad in slow.air_defence() {
        let ready = ad.net_ready_at_s.expect("under a live post");
        assert!(
            (ready - 301.0).abs() < 1e-9,
            "net_ready_at_s should be the first covered tick plus the latency, got {ready}"
        );
    }

    // Zero latency, same fixture: the net is up from the start.
    let mut instant = raid(Extras::default());
    instant.run_until(40.0);
    assert_eq!(distinct(&instant), 3);
}
