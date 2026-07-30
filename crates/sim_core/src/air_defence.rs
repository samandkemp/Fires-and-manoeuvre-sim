//! Air defence: the counter-drone half of `docs/DESIGN.md` §9. Validated by V48–V51.
//!
//! Two engagement models, because the point is that **time-to-kill has a different
//! distribution** in each — that is what makes a gun and a missile trade off differently
//! against a raid:
//!
//! - [`AdEngagement::Gun`] — a Poisson kill process while the target is in the envelope,
//!   structurally the same maths as the §3.2 glimpse model. `TTK ~ Exp(λ)`.
//! - [`AdEngagement::Missile`] — discrete shoot-look-shoot: a shot takes `range/speed`
//!   to arrive, then resolves as a Bernoulli trial; a miss costs a reload. Shots-to-kill
//!   is Geometric(p).
//!
//! The other half of the model is the **cueing timeline** (§9.5): a battery may cue
//! itself from an organic sensor or wait for a track over the net, paying
//! `cue_latency_s` — the Tx/Rx lever that decides whether the engagement window ever
//! opens at all.

use crate::los;
use crate::sim::Side;
use crate::terrain::TerrainGrid;
use crate::SimRng;
use glam::Vec2;
use rand::Rng;
use std::collections::BTreeMap;

/// How a battery kills — the choice that sets the time-to-kill distribution.
#[derive(Clone, Copy, PartialEq, Debug, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdEngagement {
    /// Gun / CIWS: a continuous Poisson kill process while the target is in envelope.
    Gun {
        /// Kill rate λ, per second. `E[TTK] = 1/λ`.
        kill_rate_per_s: f32,
    },
    /// Missile: discrete shots with a flight time and a single-shot kill probability.
    Missile {
        /// Single-shot kill probability `p`.
        ssk_p: f32,
        /// Interceptor speed, metres/second (sets the flight time to the target).
        missile_speed_m_s: f32,
        /// Delay after a miss before the next launch, seconds.
        reload_s: f32,
    },
}

/// An air-defence type's stat block (`scenarios/air_defence.toml`) — placeholder dials.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct AirDefenceType {
    /// Engagement model and its parameters.
    pub engagement: AdEngagement,
    /// Minimum engagement slant range, metres (the inner dead zone).
    #[serde(default)]
    pub min_range_m: f32,
    /// Maximum engagement slant range, metres.
    pub max_range_m: f32,
    /// Bottom of the engageable altitude band, metres above ground.
    #[serde(default)]
    pub min_alt_m: f32,
    /// Top of the engageable altitude band, metres above ground. This is what separates
    /// a low-tier CIWS from a high-tier SAM.
    pub max_alt_m: f32,
    /// Height of the launcher/mount above its own ground, metres.
    #[serde(default = "default_mount_height")]
    pub mount_height_m: f32,
    /// Does the engagement need a clear sightline to the target?
    #[serde(default = "yes")]
    pub requires_los: bool,
    /// System/crew delay between a track becoming actionable and the first shot, seconds.
    #[serde(default)]
    pub reaction_time_s: f32,
    /// Comms delay on a track cued from someone else's sensor, seconds (§9.5). The lever.
    #[serde(default)]
    pub cue_latency_s: f32,
    /// Interceptors (missiles) or bursts (gun) available; `0` = unlimited.
    #[serde(default)]
    pub magazine: u32,
    /// Simultaneous engagements — the saturation lever a raid plays against.
    #[serde(default = "one_channel")]
    pub channels: u32,
    /// Organic sensor type id (key into the sensor library), if the battery has its own.
    #[serde(default)]
    pub sensor: Option<String>,
}

fn default_mount_height() -> f32 {
    3.0
}

fn yes() -> bool {
    true
}

fn one_channel() -> u32 {
    1
}

impl Default for AirDefenceType {
    fn default() -> Self {
        Self {
            engagement: AdEngagement::Gun {
                kill_rate_per_s: 0.0,
            },
            min_range_m: 0.0,
            max_range_m: 0.0,
            min_alt_m: 0.0,
            max_alt_m: 0.0,
            mount_height_m: default_mount_height(),
            requires_los: true,
            reaction_time_s: 0.0,
            cue_latency_s: 0.0,
            magazine: 0,
            channels: one_channel(),
            sensor: None,
        }
    }
}

/// One open engagement occupying a channel.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Engagement {
    /// Index into [`crate::sim::Sim::air`] of the target being engaged.
    pub target: usize,
    /// When a missile shot arrives and resolves; `None` for a gun, which resolves every
    /// tick for as long as the target stays in the envelope.
    pub resolves_at_s: Option<f64>,
}

/// A placed air-defence asset with its resolved stat block and live engagement state.
#[derive(Clone, Debug)]
pub struct AirDefenceState {
    /// Scenario id.
    pub id: String,
    /// Owning side.
    pub side: Side,
    /// World position, metres.
    pub pos: Vec2,
    /// Resolved stat block.
    pub stats: AirDefenceType,
    /// Is the organic sensor in use? Turning this off forces the battery onto the
    /// external cueing chain, so it always pays `cue_latency_s` (§9.5).
    pub self_cue: bool,
    /// Index into [`crate::sim::Sim::sensors`] of the organic sensor, if any — what
    /// makes "my own radar saw it" distinguishable from "it came over the net".
    pub sensor_idx: Option<usize>,
    /// Interceptors/bursts remaining (`u32::MAX` stands for an unlimited magazine).
    pub magazine_left: u32,
    /// Open engagements; never longer than `stats.channels`.
    pub engagements: Vec<Engagement>,
    /// Earliest time the next shot may be launched (reload gate), seconds.
    pub ready_at_s: f64,
    /// Seam for mounting the launcher on a unit, so a strike drone could later SEAD it.
    /// Unused in v1: batteries are standalone and not attritable (§9.7).
    pub carrier: Option<usize>,
}

impl AirDefenceState {
    /// Place a battery. `sensor_idx` is the index of its organic sensor in the sim's
    /// sensor list, if it was given one.
    #[must_use]
    pub fn new(
        id: &str,
        side: Side,
        pos: Vec2,
        stats: AirDefenceType,
        self_cue: bool,
        sensor_idx: Option<usize>,
    ) -> Self {
        let magazine_left = if stats.magazine == 0 {
            u32::MAX
        } else {
            stats.magazine
        };
        Self {
            id: id.to_owned(),
            side,
            pos,
            stats,
            self_cue,
            sensor_idx,
            magazine_left,
            engagements: Vec::new(),
            ready_at_s: 0.0,
            carrier: None,
        }
    }

    /// Time at which a track becomes actionable to this battery
    /// (`docs/DESIGN.md` §9.5). The battery acts on whichever route to the track arrives
    /// first — its own radar, or the network:
    ///
    /// ```text
    /// actionable_at = min( own_sensor_seen_s,                  // organic: no comms hop
    ///                      first_detected_s + cue_latency_s )  // handed over the net
    ///                + reaction_time_s
    /// ```
    ///
    /// Taking the minimum is what makes a self-cueing battery correct even when someone
    /// else's sensor saw the target first: its own radar still closes the loop without
    /// paying the comms delay. A battery with `self_cue = false`, or none of its own,
    /// only ever has the network route.
    #[must_use]
    pub fn actionable_at(
        &self,
        first_detected_s: Option<f64>,
        own_sensor_seen_s: Option<f64>,
    ) -> Option<f64> {
        let over_the_net = first_detected_s.map(|t| t + f64::from(self.stats.cue_latency_s));
        let earliest = match (own_sensor_seen_s, over_the_net) {
            (Some(own), Some(net)) => Some(own.min(net)),
            (own, net) => own.or(net),
        }?;
        Some(earliest + f64::from(self.stats.reaction_time_s))
    }

    /// When this battery's own sensor first saw a target, given that target's per-sensor
    /// detection times. `None` unless the battery has an organic sensor, that sensor is
    /// switched on (`self_cue`), and it has actually seen the target.
    #[must_use]
    pub fn own_sensor_seen(&self, seen_by: &BTreeMap<usize, f64>) -> Option<f64> {
        if !self.self_cue {
            return None;
        }
        seen_by.get(&self.sensor_idx?).copied()
    }

    /// Is the battery able to commit another engagement right now — a free channel, a
    /// round left, and the reload elapsed?
    #[must_use]
    pub fn can_open(&self, now_s: f64) -> bool {
        self.engagements.len() < self.stats.channels as usize
            && self.magazine_left > 0
            && now_s >= self.ready_at_s
    }

    /// Is this battery already engaging `target`?
    #[must_use]
    pub fn engaging(&self, target: usize) -> bool {
        self.engagements.iter().any(|e| e.target == target)
    }

    /// Commit an engagement on `target` at slant range `range_m`, consuming a round
    /// (a missile for [`AdEngagement::Missile`], a burst allocation for
    /// [`AdEngagement::Gun`]). The caller must have checked [`AirDefenceState::can_open`].
    pub fn open(&mut self, target: usize, now_s: f64, range_m: f32) {
        let resolves_at_s = match self.stats.engagement {
            AdEngagement::Gun { .. } => None,
            AdEngagement::Missile {
                missile_speed_m_s, ..
            } => Some(now_s + f64::from(flight_time_s(range_m, missile_speed_m_s))),
        };
        self.engagements.push(Engagement {
            target,
            resolves_at_s,
        });
        self.magazine_left = self.magazine_left.saturating_sub(1);
    }

    /// Drop any engagement whose target no longer qualifies (dead, or out of envelope).
    pub fn drop_engagements(&mut self, mut still_valid: impl FnMut(usize) -> bool) {
        self.engagements.retain(|e| still_valid(e.target));
    }

    /// Resolve engagements that are due this tick, appending `(target, killed)` to `out`.
    ///
    /// A gun rolls `1 − e^{−λ·dt}` every tick the engagement is open (memoryless, so the
    /// result is independent of the tick size). A missile resolves once, when its shot
    /// arrives; a miss frees the channel and starts the reload.
    pub fn resolve_due(
        &mut self,
        now_s: f64,
        dt_s: f32,
        rng: &mut SimRng,
        out: &mut Vec<(usize, bool)>,
    ) {
        match self.stats.engagement {
            AdEngagement::Gun { kill_rate_per_s } => {
                let p = p_kill_tick(kill_rate_per_s, dt_s);
                // Fixed index order, one draw per open engagement per tick — the
                // determinism accounting unit, as in the sensing loop.
                let mut killed = Vec::new();
                for (i, e) in self.engagements.iter().enumerate() {
                    if rng.random::<f32>() < p {
                        killed.push(i);
                        out.push((e.target, true));
                    }
                }
                for i in killed.into_iter().rev() {
                    self.engagements.remove(i);
                }
            }
            AdEngagement::Missile {
                ssk_p, reload_s, ..
            } => {
                let mut done = Vec::new();
                for (i, e) in self.engagements.iter().enumerate() {
                    if e.resolves_at_s.is_some_and(|t| now_s >= t) {
                        let hit = rng.random::<f32>() < ssk_p;
                        out.push((e.target, hit));
                        done.push(i);
                        if !hit {
                            // Shoot-look-shoot: a miss costs a reload before the next
                            // launch.
                            self.ready_at_s = now_s + f64::from(reload_s);
                        }
                    }
                }
                for i in done.into_iter().rev() {
                    self.engagements.remove(i);
                }
            }
        }
    }
}

/// The slant range at which this battery can engage `target`, or `None` if the target is
/// outside its envelope (`docs/DESIGN.md` §9.4).
///
/// Returns the range rather than a bare bool because every caller that asks "can I engage
/// this?" immediately needs "at what range?" — the missile flight time is
/// `range / missile_speed`. Checks are ordered cheapest-first: the altitude band, then
/// slant range, then (only if `requires_los`) the sightline, which is the expensive one.
#[must_use]
pub fn engagement_range(
    stats: &AirDefenceType,
    terrain: &TerrainGrid,
    ad_pos: Vec2,
    target_pos: Vec2,
    target_agl_m: f32,
) -> Option<f32> {
    if target_agl_m < stats.min_alt_m || target_agl_m > stats.max_alt_m {
        return None;
    }
    let r = los::slant_range(
        terrain,
        ad_pos,
        stats.mount_height_m,
        target_pos,
        target_agl_m,
    );
    if r < stats.min_range_m || r > stats.max_range_m {
        return None;
    }
    if stats.requires_los
        && !los::visible(
            terrain,
            ad_pos,
            stats.mount_height_m,
            target_pos,
            target_agl_m,
        )
    {
        return None;
    }
    Some(r)
}

/// Is `target` inside this battery's engagement envelope? The bool form of
/// [`engagement_range`], for callers that do not need the range.
#[must_use]
pub fn in_envelope(
    stats: &AirDefenceType,
    terrain: &TerrainGrid,
    ad_pos: Vec2,
    target_pos: Vec2,
    target_agl_m: f32,
) -> bool {
    engagement_range(stats, terrain, ad_pos, target_pos, target_agl_m).is_some()
}

/// Per-tick kill probability of a gun: `1 − e^{−λ·dt}`. The same memoryless form as
/// [`crate::sensing::p_detect_tick`], so the result is independent of the tick size.
#[must_use]
pub fn p_kill_tick(kill_rate_per_s: f32, dt_s: f32) -> f32 {
    1.0 - (-f64::from(kill_rate_per_s) * f64::from(dt_s)).exp() as f32
}

/// Interceptor flight time to a target at `range_m`, seconds.
#[must_use]
pub fn flight_time_s(range_m: f32, missile_speed_m_s: f32) -> f32 {
    if missile_speed_m_s <= 0.0 {
        f32::INFINITY
    } else {
        range_m / missile_speed_m_s
    }
}

/// Expected time-to-kill of a gun: `1/λ` (§9.4).
#[must_use]
pub fn expected_ttk_gun(kill_rate_per_s: f32) -> f32 {
    if kill_rate_per_s <= 0.0 {
        f32::INFINITY
    } else {
        1.0 / kill_rate_per_s
    }
}

/// Expected time-to-kill of a shoot-look-shoot missile battery (§9.4):
/// `E[TTK] = t_f/p + (1/p − 1)·t_r`, from `E[shots] = 1/p` (geometric) and a time to the
/// N-th arrival of `N·t_f + (N−1)·t_r`.
#[must_use]
pub fn expected_ttk_missile(ssk_p: f32, flight_time_s: f32, reload_s: f32) -> f32 {
    if ssk_p <= 0.0 {
        return f32::INFINITY;
    }
    flight_time_s / ssk_p + (1.0 / ssk_p - 1.0) * reload_s
}

/// The effective engagement window (`docs/DESIGN.md` §9.5).
///
/// The cueing clock starts at **detection**, not at envelope entry, so early warning and
/// comms latency trade directly against one another. For a target detected
/// `warning_lead_s` before it enters the envelope and in the envelope for
/// `in_envelope_s`:
///
/// ```text
/// W_eff = max(0, W − max(0, L + R − D))
/// ```
///
/// The delay only costs anything once `L + R` outruns the warning lead `D`: a cue that
/// has already aged through the network by the time the target arrives is free. With
/// `D = 0` (detected as it enters) this reduces to the familiar `W − L − R`.
#[must_use]
pub fn effective_window_s(
    in_envelope_s: f32,
    warning_lead_s: f32,
    cue_latency_s: f32,
    reaction_s: f32,
) -> f32 {
    let overrun = (cue_latency_s + reaction_s - warning_lead_s).max(0.0);
    (in_envelope_s - overrun).max(0.0)
}

/// The **critical cue latency** `L* = W + D − R`: beyond this the effective window is
/// zero and every target leaks, however lethal the battery is (§9.5). Early warning
/// raises it one second per second.
#[must_use]
pub fn critical_latency_s(in_envelope_s: f32, warning_lead_s: f32, reaction_s: f32) -> f32 {
    (in_envelope_s + warning_lead_s - reaction_s).max(0.0)
}

/// Probability a target survives a gun defence over an effective window:
/// `exp(−λ·W_eff)`.
#[must_use]
pub fn p_leak_gun(kill_rate_per_s: f32, window_s: f32) -> f32 {
    (-f64::from(kill_rate_per_s) * f64::from(window_s)).exp() as f32
}

/// How many shots a missile battery gets inside an effective window:
/// `⌊(W_eff − t_f)/(t_f + t_r)⌋ + 1`, or 0 if the first shot cannot arrive in time.
#[must_use]
pub fn shot_opportunities(window_s: f32, flight_time_s: f32, reload_s: f32) -> u32 {
    if window_s < flight_time_s || flight_time_s.is_infinite() {
        return 0;
    }
    let cycle = flight_time_s + reload_s;
    if cycle <= 0.0 {
        return u32::MAX;
    }
    (((window_s - flight_time_s) / cycle).floor() as u32) + 1
}

/// Probability a target survives `shots` independent missile engagements: `(1 − p)^k`.
#[must_use]
pub fn p_leak_missile(ssk_p: f32, shots: u32) -> f32 {
    (1.0 - ssk_p).powi(shots as i32)
}
