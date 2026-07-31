//! Drones: a third asset class beside units and sensors.
//! Spec: `docs/DESIGN.md` §9. Gates: V44, V46, V47.
//!
//! An airframe has an altitude (AGL or AMSL), a heading and a speed, and flies either a
//! waypoint path or a transit-then-orbit. It can carry a sensor, a strike payload, or
//! both. Flight is pure and draws no randomness — that lives in detection (§3) and
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

/// What a strike drone is sent to attack (§9.3). Assigned targets only; autonomous
/// selection is deferred to the kill-chain work.
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
    /// Maximum turn rate, degrees/second. Implies a minimum turn radius `v / ω`. The
    /// large default makes turns near-instant, so a type that doesn't care about turn
    /// performance tracks its waypoints exactly.
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

// Hand-written for the same reason as `UnitType` and `WeaponType`: deriving would zero
// the silhouette width, the turn rate (freezing the heading) and the release range — all
// silent failures.
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
    /// Whether the *opposing* side currently holds a track on this airframe. Derived
    /// from [`AirState::last_seen_s`], refreshed each decision epoch (§10.1).
    pub detected: bool,
    /// Sim time this airframe was last observed by the opposing side, if ever.
    pub last_seen_s: Option<f64>,
    /// Sim time of the *first* detection by any sensor — the moment a track enters the
    /// cueing network (§9.5).
    pub detected_at_s: Option<f64>,
    /// Index into [`crate::sim::Sim::sensors`] of the sensor that first detected it.
    pub detected_by: Option<usize>,
    /// When each sensor first saw this airframe, by sensor index.
    ///
    /// A battery checks its own entry to know whether its radar has the target, instead
    /// of guessing from whoever detected first, which is what keeps the §9.5 timeline
    /// exact when self-cueing. `BTreeMap` for deterministic iteration order.
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
            last_seen_s: None,
            detected_at_s: None,
            detected_by: None,
            seen_by: BTreeMap::new(),
            stats,
            sensor,
            payload,
        }
    }

    /// Height above the ground beneath — the actor height `h` from §1.2 that LOS,
    /// viewshed and sensing all take.
    ///
    /// The AGL/AMSL distinction lives entirely here: AGL carries its height along, AMSL
    /// has terrain eat into it. Clamping at zero puts an AMSL airframe below local ground
    /// on the deck rather than underground.
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

/// Wrap an angle difference to `(-180, 180]`: the signed short way round.
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
