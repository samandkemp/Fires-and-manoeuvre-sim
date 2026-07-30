//! Air assets: drones as a first-class class alongside units and sensors.
//! Specified in `docs/DESIGN.md` §9; validated by V44, V46, V47.
//!
//! An airframe flies at a chosen altitude (AGL or AMSL), heading and speed, following a
//! flight plan that is either a path or a transit-then-orbit. It may carry a sensor (a
//! recce drone — a mobile elevated observer) and/or a payload (a strike drone). Flight
//! itself is **pure and RNG-free**: all air stochasticity lives in detection (§3) and
//! air-defence engagement (§9.4).

use crate::fires::WeaponType;
use crate::sensing::{Modality, SensorType};
use crate::sim::Side;
use crate::terrain::TerrainGrid;
use glam::Vec2;
use std::collections::BTreeMap;

/// What `altitude_m` is measured from (`docs/DESIGN.md` §9.1). This is the dial that
/// decides whether terrain can mask the airframe.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AltitudeRef {
    /// Above ground level: the airframe follows the terrain and is never masked by the
    /// hill it overflies.
    #[default]
    Agl,
    /// Above mean sea level: the airframe cruises level, so higher ground masks it and
    /// it can sit below a ridgeline.
    Amsl,
}

/// How a flight plan ends. The orbit centre is always the final waypoint, so "fly this
/// path" is `Hold` and "go here and orbit at radius R" is one waypoint plus `Orbit` —
/// both requested behaviours from one structure (`docs/DESIGN.md` §9.2).
#[derive(Clone, Copy, PartialEq, Debug, Default, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Terminal {
    /// Station-keep at the final waypoint.
    #[default]
    Hold,
    /// Orbit the final waypoint at a fixed radius.
    Orbit {
        /// Orbit radius, metres.
        radius_m: f32,
        /// Turn direction: `true` = clockwise (decreasing phase angle).
        clockwise: bool,
    },
}

/// A flight plan: waypoints to fly, then a terminal behaviour.
#[derive(Clone, Debug, Default)]
pub struct FlightPlan {
    /// Waypoints in world metres, flown in order.
    pub waypoints: Vec<Vec2>,
    /// What to do on arrival at the last waypoint.
    pub terminal: Terminal,
}

impl FlightPlan {
    /// A plan that flies `waypoints` and then holds.
    #[must_use]
    pub fn route(waypoints: Vec<Vec2>) -> Self {
        Self {
            waypoints,
            terminal: Terminal::Hold,
        }
    }

    /// A plan that transits to `centre` and orbits it at `radius_m`.
    #[must_use]
    pub fn orbit(centre: Vec2, radius_m: f32, clockwise: bool) -> Self {
        Self {
            waypoints: vec![centre],
            terminal: Terminal::Orbit {
                radius_m,
                clockwise,
            },
        }
    }

    /// The final waypoint, if the plan has one.
    #[must_use]
    pub fn destination(&self) -> Option<Vec2> {
        self.waypoints.last().copied()
    }
}

/// What a strike drone is sent to attack (`docs/DESIGN.md` §9.3). Assigned only —
/// autonomous target selection is deliberately deferred as its own kill-chain piece.
#[derive(Clone, Debug, PartialEq)]
pub enum TargetSpec {
    /// A named unit (matched against [`crate::sim::UnitState::id`]); the aim point
    /// tracks the unit as it moves.
    Unit(String),
    /// A fixed ground point.
    Point(Vec2),
}

/// An air type's stat block (`scenarios/air.toml`) — all placeholder dials.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct AirType {
    /// Airframe size as a LOS and air-defence target, metres.
    pub height_m: f32,
    /// Silhouette width for direct engagement, metres.
    #[serde(default = "default_silhouette_width")]
    pub silhouette_width_m: f32,
    /// Default cruise speed, metres/second (an instance may override it).
    pub cruise_speed_m_s: f32,
    /// Maximum rate of turn, degrees/second. Implies a minimum turn radius `v / ω`; the
    /// large default makes turns effectively instant, so a plan flown by a type that
    /// doesn't care tracks its waypoints exactly.
    #[serde(default = "default_turn_rate")]
    pub max_turn_rate_deg_s: f32,
    /// Time aloft before the airframe is removed, seconds (`0` = unlimited).
    #[serde(default)]
    pub endurance_s: f32,
    /// Per-modality signature, as [`crate::sensing::UnitType::signature`].
    #[serde(default)]
    pub signature: BTreeMap<String, f32>,
    /// Sensor type id this airframe carries (key into the sensor library), if any.
    #[serde(default)]
    pub sensor: Option<String>,
    /// Weapon type id carried as a strike payload (an `indirect` weapon), if any.
    #[serde(default)]
    pub payload: Option<String>,
    /// Number of munitions carried.
    #[serde(default)]
    pub munitions: u32,
    /// Is the airframe consumed by its own attack (a one-way attack munition)?
    #[serde(default)]
    pub expendable: bool,
    /// Slant range to the aim point at which a munition is released, metres.
    #[serde(default = "default_release_range")]
    pub release_range_m: f32,
}

fn default_silhouette_width() -> f32 {
    2.0
}

fn default_turn_rate() -> f32 {
    180.0
}

fn default_release_range() -> f32 {
    200.0
}

// Manual `Default` for the same reason `UnitType` and `WeaponType` have one: the derive
// would zero the silhouette width, the turn rate (freezing the heading forever) and the
// release range, all of which fail silently rather than loudly.
impl Default for AirType {
    fn default() -> Self {
        Self {
            height_m: 1.0,
            silhouette_width_m: default_silhouette_width(),
            cruise_speed_m_s: 0.0,
            max_turn_rate_deg_s: default_turn_rate(),
            endurance_s: 0.0,
            signature: BTreeMap::new(),
            sensor: None,
            payload: None,
            munitions: 0,
            expendable: false,
            release_range_m: default_release_range(),
        }
    }
}

impl AirType {
    /// The airframe's signature in a modality (0 if the table has no entry).
    #[must_use]
    pub fn signature_in(&self, modality: Modality) -> f32 {
        crate::sensing::signature_in(&self.signature, modality)
    }
}

/// A placed airframe with its resolved stat blocks and live flight state.
#[derive(Clone, Debug)]
pub struct AirState {
    /// Scenario id.
    pub id: String,
    /// Owning side.
    pub side: Side,
    /// World position, metres.
    pub pos: Vec2,
    /// Altitude, metres, in the frame given by [`AirState::altitude_ref`].
    pub altitude_m: f32,
    /// Whether `altitude_m` is above ground or above mean sea level.
    pub altitude_ref: AltitudeRef,
    /// Current heading, degrees (maths convention: 0° = +X/east, CCW).
    pub heading_deg: f32,
    /// Current speed, metres/second.
    pub speed_m_s: f32,
    /// The flight plan being flown.
    pub plan: FlightPlan,
    /// Index of the next waypoint to fly to.
    pub route_idx: usize,
    /// Orbit phase angle in radians once the orbit is established; `None` while
    /// transiting. Integrating this phase (rather than steering at the circle) is what
    /// keeps the radius exact.
    pub orbit_phase: Option<f32>,
    /// Assigned strike target; `None` falls back to the final waypoint (§9.3).
    pub target: Option<TargetSpec>,
    /// Still flying (endurance not expired, not shot down, not expended).
    pub alive: bool,
    /// Time aloft, seconds.
    pub time_alive_s: f32,
    /// Munitions remaining.
    pub munitions_left: u32,
    /// Whether the *opposing* side has detected this airframe.
    pub detected: bool,
    /// Sim time of the *first* detection by any sensor — the moment a track enters the
    /// cueing network (§9.5).
    pub detected_at_s: Option<f64>,
    /// Index into [`crate::sim::Sim::sensors`] of the sensor that first detected it.
    pub detected_by: Option<usize>,
    /// When **each** sensor first saw this airframe, keyed by sensor index. An
    /// air-defence battery reads its own entry to know whether its organic radar has
    /// closed the loop, rather than inferring it from who happened to detect first —
    /// which is what makes the §9.5 timeline exact for a self-cueing battery.
    /// `BTreeMap` (not `HashMap`) so iteration order is deterministic.
    pub seen_by: BTreeMap<usize, f64>,
    /// Resolved stat block.
    pub stats: AirType,
    /// Resolved carried sensor, if any (recce payload).
    pub sensor: Option<SensorType>,
    /// Resolved strike payload, if any.
    pub payload: Option<WeaponType>,
}

impl AirState {
    /// Build a placed airframe at `pos` with its resolved stat blocks.
    // Nine arguments, but they are the placement itself — where, how high, in which
    // frame, on what heading, and with which payloads. A builder would add ceremony
    // without removing a single decision the caller has to make.
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn new(
        id: &str,
        side: Side,
        pos: Vec2,
        altitude_m: f32,
        altitude_ref: AltitudeRef,
        heading_deg: f32,
        stats: AirType,
        sensor: Option<SensorType>,
        payload: Option<WeaponType>,
    ) -> Self {
        let speed_m_s = stats.cruise_speed_m_s;
        let munitions_left = stats.munitions;
        Self {
            id: id.to_owned(),
            side,
            pos,
            altitude_m,
            altitude_ref,
            heading_deg: wrap360(heading_deg),
            speed_m_s,
            plan: FlightPlan::default(),
            route_idx: 0,
            orbit_phase: None,
            target: None,
            alive: true,
            time_alive_s: 0.0,
            munitions_left,
            detected: false,
            detected_at_s: None,
            detected_by: None,
            seen_by: BTreeMap::new(),
            stats,
            sensor,
            payload,
        }
    }

    /// Height above the ground directly beneath — the actor height `h` of
    /// `docs/DESIGN.md` §1.2 that LOS, viewshed and sensing all take.
    ///
    /// This one function is the whole of the AGL/AMSL distinction: an AGL airframe
    /// carries its height with it, an AMSL one has the terrain eat into it. The clamp
    /// means an AMSL altitude below local ground is an airframe on the deck — degenerate
    /// but well-defined, never negative.
    #[must_use]
    pub fn actor_height(&self, terrain: &TerrainGrid) -> f32 {
        match self.altitude_ref {
            AltitudeRef::Agl => self.altitude_m.max(0.0),
            AltitudeRef::Amsl => (self.altitude_m - terrain.sample_elevation(self.pos)).max(0.0),
        }
    }

    /// Absolute height above datum (`z + h`) — what the slant-range and LOS maths use.
    #[must_use]
    pub fn absolute_height(&self, terrain: &TerrainGrid) -> f32 {
        terrain.sample_elevation(self.pos) + self.actor_height(terrain)
    }

    /// Can this airframe still deliver a munition?
    #[must_use]
    pub fn can_strike(&self) -> bool {
        self.alive && self.payload.is_some() && self.munitions_left > 0
    }

    /// Has the airframe finished its plan (holding at the end, not orbiting)?
    #[must_use]
    pub fn plan_complete(&self) -> bool {
        self.orbit_phase.is_none() && self.route_idx >= self.plan.waypoints.len()
    }

    /// Assign a flight plan, restarting it from the first waypoint.
    pub fn set_plan(&mut self, plan: FlightPlan) {
        self.plan = plan;
        self.route_idx = 0;
        self.orbit_phase = None;
    }

    /// Advance flight by `dt_s` seconds. Pure: no RNG, no terrain interaction — position
    /// integrates from the heading, and the heading steers toward the current steering
    /// point at up to `max_turn_rate_deg_s`. `docs/DESIGN.md` §9.2.
    pub fn advance(&mut self, dt_s: f32) {
        if !self.alive || dt_s <= 0.0 {
            return;
        }
        self.time_alive_s += dt_s;
        if self.stats.endurance_s > 0.0 && self.time_alive_s > self.stats.endurance_s {
            self.alive = false;
            return;
        }
        if self.speed_m_s <= 0.0 {
            return;
        }

        // Established orbit: integrate the phase directly, so the radius stays exact and
        // the lap time is exactly 2πR/v (V46) rather than accumulating steering error.
        if let (
            Some(phase),
            Terminal::Orbit {
                radius_m,
                clockwise,
            },
            Some(centre),
        ) = (
            self.orbit_phase,
            self.plan.terminal,
            self.plan.destination(),
        ) {
            if radius_m > 0.0 {
                let sign = if clockwise { -1.0 } else { 1.0 };
                let phase = wrap_tau(phase + sign * (self.speed_m_s / radius_m) * dt_s);
                self.orbit_phase = Some(phase);
                self.pos = centre + Vec2::from_angle(phase) * radius_m;
                // Heading is the tangent: 90° ahead of the radius vector, or behind it
                // going clockwise.
                self.heading_deg = wrap360(phase.to_degrees() + sign * 90.0);
                return;
            }
        }

        let Some(steer_to) = self.steering_point() else {
            return; // plan complete and holding
        };

        // Turn toward the steering point, then fly the (new) heading for one tick.
        let desired = bearing_deg(steer_to - self.pos);
        self.heading_deg = turn_toward(
            self.heading_deg,
            desired,
            self.stats.max_turn_rate_deg_s * dt_s,
        );
        let step = self.speed_m_s * dt_s;
        self.pos += Vec2::from_angle(self.heading_deg.to_radians()) * step;

        // Waypoint capture. The radius is the larger of one tick's travel and the
        // airframe's minimum turn radius: with an unlimited turn rate that is just the
        // step (so the plan is tracked exactly), and with a turn limit it is wide enough
        // that a tight corner cannot trap the airframe circling outside it forever.
        let capture = step.max(self.min_turn_radius());
        if self.pos.distance(steer_to) <= capture {
            self.capture_steering_point();
        }
    }

    /// Minimum turn radius `v / ω` (ω in radians/second) — the geometric consequence of
    /// the turn-rate limit, and the gate V47 checks.
    #[must_use]
    pub fn min_turn_radius(&self) -> f32 {
        let omega = self.stats.max_turn_rate_deg_s.to_radians();
        if omega <= 0.0 {
            f32::INFINITY
        } else {
            self.speed_m_s / omega
        }
    }

    /// The point the airframe is currently steering at: the next waypoint, except that
    /// on the final leg of an orbit plan it steers at the nearest point on the orbit
    /// circle rather than the centre — so establishing the orbit is a smooth capture
    /// rather than a teleport out to the radius.
    fn steering_point(&self) -> Option<Vec2> {
        let target = *self.plan.waypoints.get(self.route_idx)?;
        let last_leg = self.route_idx + 1 == self.plan.waypoints.len();
        if let (true, Terminal::Orbit { radius_m, .. }) = (last_leg, self.plan.terminal) {
            if radius_m > 0.0 {
                let out = self.pos - target;
                // Directly over the centre: fly out along the nose to pick an entry point.
                let dir = out
                    .try_normalize()
                    .unwrap_or_else(|| Vec2::from_angle(self.heading_deg.to_radians()));
                return Some(target + dir * radius_m);
            }
        }
        Some(target)
    }

    /// Arrive at the current steering point: either establish the orbit or move on to
    /// the next waypoint.
    fn capture_steering_point(&mut self) {
        let last_leg = self.route_idx + 1 == self.plan.waypoints.len();
        if let (true, Terminal::Orbit { radius_m, .. }, Some(centre)) =
            (last_leg, self.plan.terminal, self.plan.destination())
        {
            if radius_m > 0.0 {
                // Phase from where we actually are, then snap exactly onto the circle;
                // the correction is at most one capture radius.
                let phase = bearing_rad(self.pos - centre);
                self.orbit_phase = Some(phase);
                self.pos = centre + Vec2::from_angle(phase) * radius_m;
                return;
            }
        }
        self.route_idx += 1;
    }
}

/// Bearing of a vector in degrees (0° = +X/east, CCW), the sim's angle convention.
#[must_use]
pub fn bearing_deg(v: Vec2) -> f32 {
    wrap360(v.y.atan2(v.x).to_degrees())
}

fn bearing_rad(v: Vec2) -> f32 {
    v.y.atan2(v.x)
}

/// Turn from `from_deg` toward `to_deg` by at most `max_step_deg`, the short way round.
#[must_use]
pub fn turn_toward(from_deg: f32, to_deg: f32, max_step_deg: f32) -> f32 {
    let delta = wrap180(to_deg - from_deg);
    wrap360(from_deg + delta.clamp(-max_step_deg.abs(), max_step_deg.abs()))
}

/// Wrap an angle to `[0, 360)`.
#[must_use]
pub fn wrap360(deg: f32) -> f32 {
    deg.rem_euclid(360.0)
}

/// Wrap an angle difference to `(-180, 180]` — the signed short way round, which is what
/// makes "how far off heading am I?" answerable without a sign convention argument.
#[must_use]
pub fn wrap180(deg: f32) -> f32 {
    let d = deg.rem_euclid(360.0);
    if d > 180.0 {
        d - 360.0
    } else {
        d
    }
}

/// Wrap a radian angle to `[0, 2π)`.
fn wrap_tau(rad: f32) -> f32 {
    rad.rem_euclid(std::f32::consts::TAU)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::los;
    use crate::terrain::{TerrainGrid, TerrainParams, TerrainParamsTable, TerrainType};
    use ndarray::Array2;

    fn params() -> TerrainParamsTable {
        let mk = |fh, k, cov, con, mob| TerrainParams {
            feature_height_m: fh,
            extinction_per_m: k,
            cover: cov,
            concealment: con,
            mobility_cost: mob,
        };
        TerrainParamsTable {
            open: mk(0.0, 0.0, 0.0, 0.0, 1.0),
            trees: mk(12.0, 0.08, 0.3, 0.6, 1.8),
            urban: mk(8.0, 0.0, 0.7, 0.5, 1.5),
        }
    }

    /// A north–south ridge of height `crest` occupying the cell columns `[x0, x1)`.
    fn ridge(w: usize, h: usize, x0: usize, x1: usize, crest: f32) -> TerrainGrid {
        let elev = Array2::from_shape_fn(
            (h, w),
            |(_, ix)| {
                if ix >= x0 && ix < x1 {
                    crest
                } else {
                    0.0
                }
            },
        );
        TerrainGrid::from_layers(
            10.0,
            elev,
            Array2::from_elem((h, w), TerrainType::Open),
            &params(),
        )
    }

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
}
