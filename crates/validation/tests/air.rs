//! V44, V46-V47 - drone altitude/masking and flight kinematics (docs/DESIGN.md §9.1-§9.2).

use glam::Vec2;
use sim_core::air::*;
use sim_core::los;
use sim_core::sim::Side;
use validation::ridge;

fn drone(pos: Vec2, altitude_m: f32, altitude_ref: AltitudeRef, heading_deg: f32) -> AirState {
    AirState::new(
        "uas",
        Side::Red,
        pos,
        altitude_m,
        altitude_ref,
        heading_deg,
        AirType {
            cruise_speed_m_s: 50.0,
            ..Default::default()
        },
        None,
        None,
    )
}

// V44: altitude & masking. A ridge of crest height C masks an AMSL drone flying
// below C from a ground observer on the far side, while the same drone at AGL rides
// over the ridge and stays visible. Visibility is monotone in altitude (extends V8).
#[test]
fn v44_altitude_and_masking() {
    // Ridge crest 200 m across columns 40..44 (x = 400..440 m); observer west of it,
    // drone east of it.
    let g = ridge(96, 16, 40, 44, 200.0);
    let observer = Vec2::new(100.0, 80.0);
    let drone_pos = Vec2::new(800.0, 80.0);

    // AMSL 120 m: the airframe is *below* the 200 m crest, so the ridge masks it.
    let low = drone(drone_pos, 120.0, AltitudeRef::Amsl, 180.0);
    assert_eq!(
        low.actor_height(&g),
        120.0,
        "flat ground east of the ridge ⇒ AMSL altitude is also the actor height"
    );
    assert!(
        !los::visible(&g, observer, 2.0, drone_pos, low.actor_height(&g)),
        "an AMSL drone below the crest must be masked by the ridge"
    );

    // AGL 120 m over the same spot: identical here (ground is at 0), so the
    // distinction only bites where the airframe is *over* high ground — check that.
    let over_ridge = Vec2::new(420.0, 80.0);
    let agl = drone(over_ridge, 120.0, AltitudeRef::Agl, 180.0);
    let amsl = drone(over_ridge, 120.0, AltitudeRef::Amsl, 180.0);
    assert_eq!(
        agl.actor_height(&g),
        120.0,
        "AGL carries its height over the crest"
    );
    assert_eq!(
        amsl.actor_height(&g),
        0.0,
        "AMSL 120 m over a 200 m crest is on the deck (clamped, never negative)"
    );
    assert!(
        agl.absolute_height(&g) > amsl.absolute_height(&g),
        "the AGL airframe is absolutely higher over high ground"
    );

    // Monotone in altitude: raising an AMSL drone above the crest reveals it, and
    // once visible it stays visible as it climbs further (V8's monotonicity).
    let mut previously_visible = false;
    for alt in [50.0f32, 120.0, 190.0, 210.0, 400.0, 900.0] {
        let d = drone(drone_pos, alt, AltitudeRef::Amsl, 180.0);
        let vis = los::visible(&g, observer, 2.0, drone_pos, d.actor_height(&g));
        assert!(
            !previously_visible || vis,
            "visibility must not be lost by climbing (alt {alt})"
        );
        previously_visible = vis;
    }
    assert!(
        previously_visible,
        "a drone high above the crest must be visible"
    );

    // And the slant range to an overhead drone is dominated by its altitude.
    let r = los::slant_range(&g, observer, 2.0, observer, 900.0);
    assert!((r - 898.0).abs() < 1e-2);
}

// V46: orbit kinematics — the radius holds and a lap takes exactly 2πR/v.
#[test]
fn v46_orbit_kinematics() {
    let centre = Vec2::new(3000.0, 3000.0);
    let (radius, speed, dt) = (600.0f32, 50.0f32, 1.0f32);
    let mut d = drone(Vec2::new(1000.0, 3000.0), 300.0, AltitudeRef::Agl, 0.0);
    d.speed_m_s = speed;
    d.set_plan(FlightPlan::orbit(centre, radius, false));

    // Transit until the orbit is established.
    let mut t = 0.0f32;
    while d.orbit_phase.is_none() && t < 200.0 {
        d.advance(dt);
        t += dt;
    }
    assert!(d.orbit_phase.is_some(), "the orbit should be captured");

    // Radius holds through a full lap, and heading stays tangential.
    let lap_s = std::f32::consts::TAU * radius / speed; // 2πR/v ≈ 75.4 s
    let start_phase = d.orbit_phase.unwrap();
    let laps = (lap_s / dt).round() as u32;
    for _ in 0..laps {
        d.advance(dt);
        let r = d.pos.distance(centre);
        assert!(
            (r - radius).abs() < 1e-2,
            "orbit radius must hold exactly, got {r}"
        );
        // Tangency: the heading is perpendicular to the radius vector.
        let radial = bearing_deg(d.pos - centre);
        let off = (wrap180(d.heading_deg - radial)).abs();
        assert!(
            (off - 90.0).abs() < 1e-2,
            "heading must be tangential ({off})"
        );
    }
    // A lap of 2πR/v seconds returns to the starting phase.
    let end_phase = d.orbit_phase.unwrap();
    let closed = wrap180((end_phase - start_phase).to_degrees()).abs();
    assert!(
        closed < 2.0,
        "a lap of {lap_s:.1} s should close the circle, off by {closed:.2}°"
    );
}

// V47: transit & turn rate. A straight leg covers speed·t; a turn-rate-limited
// airframe traces a circle of radius v/ω_max.
#[test]
fn v47_transit_and_turn_rate() {
    // Straight leg: 50 m/s for 30 s is 1500 m along, exactly (mirrors V37).
    let mut d = drone(Vec2::new(0.0, 500.0), 200.0, AltitudeRef::Agl, 0.0);
    d.set_plan(FlightPlan::route(vec![Vec2::new(100_000.0, 500.0)]));
    for _ in 0..30 {
        d.advance(1.0);
    }
    assert!(
        (d.pos.x - 1500.0).abs() < 1e-1 && (d.pos.y - 500.0).abs() < 1e-3,
        "30 s at 50 m/s due east should be x = 1500, got {:?}",
        d.pos
    );

    // Turn: heading east, waypoint far to the north, turn capped at 10°/s.
    // Minimum turn radius r = v/ω = 50 / (10·π/180) ≈ 286.5 m; after turning
    // through 90° the airframe has traced a quarter circle, displacing r east and
    // r north of where the turn began.
    let (speed, omega) = (50.0f32, 10.0f32);
    let mut d = AirState::new(
        "turner",
        Side::Red,
        Vec2::ZERO,
        200.0,
        AltitudeRef::Agl,
        0.0,
        AirType {
            cruise_speed_m_s: speed,
            max_turn_rate_deg_s: omega,
            ..Default::default()
        },
        None,
        None,
    );
    d.set_plan(FlightPlan::route(vec![Vec2::new(0.0, 100_000.0)]));
    let r_min = d.min_turn_radius();
    assert!(
        (r_min - speed / omega.to_radians()).abs() < 1e-3,
        "minimum turn radius is v/ω"
    );

    let start = d.pos;
    // Turn through 90° at 10°/s = 9 ticks of 1 s.
    for _ in 0..9 {
        d.advance(1.0);
    }
    assert!(
        (d.heading_deg - 90.0).abs() < 1e-3,
        "after 9 s at 10°/s the heading should be due north, got {}",
        d.heading_deg
    );

    // The invariant to check is the *chord*: a turn through Φ at radius r displaces
    // the airframe by `2r·sin(Φ/2)`, here `r√2`. (The east/north split is *not* a
    // clean (r, r): the integrator turns then flies, so the polygon it traces sits
    // half a turn-step — 5° — ahead in phase. That biases the components while
    // leaving the chord length correct to ~0.1%, which is why the chord is the gate.)
    let delta = d.pos - start;
    let chord = 2.0 * r_min * (std::f32::consts::FRAC_PI_4).sin();
    assert!(
        (delta.length() - chord).abs() < 0.01 * chord,
        "a 90° turn at radius {r_min:.1} m should displace by the chord {chord:.1} m, \
         got {:.1} m ({delta:?})",
        delta.length()
    );
    // And the achieved turn was no tighter than the rate limit allows.
    let achieved = delta.length() / std::f32::consts::SQRT_2;
    assert!(
        achieved >= r_min * 0.98,
        "achieved turn radius {achieved:.1} must not beat the {r_min:.1} m limit"
    );
}

// Endurance removes the airframe, and a flight plan is otherwise deterministic.
#[test]
fn endurance_and_determinism() {
    let mut d = drone(Vec2::ZERO, 100.0, AltitudeRef::Agl, 0.0);
    d.stats.endurance_s = 10.0;
    d.set_plan(FlightPlan::route(vec![Vec2::new(10_000.0, 0.0)]));
    for _ in 0..12 {
        d.advance(1.0);
    }
    assert!(!d.alive, "the airframe should time out at its endurance");

    let fly = || {
        let mut d = drone(Vec2::new(10.0, 20.0), 150.0, AltitudeRef::Amsl, 33.0);
        d.set_plan(FlightPlan::orbit(Vec2::new(2000.0, 1500.0), 400.0, true));
        for _ in 0..500 {
            d.advance(0.5);
        }
        (d.pos, d.heading_deg, d.orbit_phase)
    };
    assert_eq!(fly(), fly(), "flight is pure — identical every time");
}
