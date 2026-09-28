//! Scenario loading: TOML to structs to a deterministic [`TerrainGrid`]. The only I/O
//! the engine does. Spec: `docs/THEORY.md` §1.

use crate::air_defence::AirDefenceType;
use crate::airframes::{AirType, AltitudeRef, Terminal};
use crate::c2::C2Type;
use crate::doctrine::{Doctrine, Order};
use crate::sensing::{SensorType, UnitType};
use crate::terrain::{TerrainGrid, TerrainParamsTable, TerrainSource};
use crate::weapon_effects::WeaponType;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Everything that can go wrong loading a scenario. Typed so callers (experiments, the
/// app) can distinguish a missing file from a syntax error from a semantic one.
#[derive(Debug, thiserror::Error)]
pub enum ScenarioError {
    /// The file could not be read.
    #[error("could not read {path}")]
    Io {
        /// The path we tried to read.
        path: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The file was not valid TOML, or did not match the schema.
    #[error("invalid TOML: {0}")]
    Parse(#[from] toml::de::Error),
    /// The file parsed but is not a usable scenario (e.g. zero-sized terrain).
    #[error("invalid scenario: {0}")]
    Invalid(String),
}

/// A scenario: a named situation the engine can simulate - terrain, sim clock, and the
/// two forces' placed assets. Weapons join this schema in the fires phase.
///
/// # Unknown keys are rejected
///
/// Every struct in this schema carries `deny_unknown_fields`. Nearly all the dials have a
/// serde default, so without it a misspelt key - `track_hold` for `track_hold_s` - parses
/// perfectly, takes the default, and produces a study of a dial nobody set. That failure
/// is invisible: the run succeeds and the answer is simply about a different question.
/// Refusing the key turns it into a load error naming the file.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    /// Human-readable scenario name.
    pub name: String,
    /// The seed used unless a run overrides it.
    #[serde(default)]
    pub default_seed: u64,
    /// Terrain grid definition.
    pub terrain: TerrainConfig,
    /// Sim clock configuration (`docs/THEORY.md` §3.3); defaults if absent.
    #[serde(default)]
    pub sim: SimConfig,
    /// Blue force starting assets.
    #[serde(default)]
    pub blue: Force,
    /// Red force starting assets.
    #[serde(default)]
    pub red: Force,
}

/// Sim clock + suppression dials (`docs/THEORY.md` §3.3, §4.3).
#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SimConfig {
    /// Integration tick, seconds.
    #[serde(default = "default_dt_s")]
    pub dt_s: f32,
    /// Decision-epoch length, seconds.
    #[serde(default = "default_epoch_s")]
    pub epoch_s: f32,
    /// A round landing within this distance of a unit is a near-miss (suppresses), m.
    #[serde(default = "default_suppression_radius")]
    pub suppression_radius_m: f32,
    /// Probability a single near-miss steps a unit's suppression up one level.
    #[serde(default = "default_p_suppress")]
    pub p_suppress: f32,
    /// Suppression recovery rate (per second, one level down).
    #[serde(default = "default_recover_per_s")]
    pub recover_per_s: f32,
    /// Outgoing-fire effectiveness multiplier while Suppressed (`< 1`).
    #[serde(default = "default_suppressed_fire_factor")]
    pub suppressed_fire_factor: f32,
    /// How long a track survives without being re-observed, seconds
    /// (`docs/THEORY.md` §10.1). This is what lets EW *break* a track rather than only
    /// prevent one: jam a tracked unit, nobody re-observes it, and the track lapses.
    #[serde(default = "default_track_hold")]
    pub track_hold_s: f32,
    /// How readily a sensor must still be able to see a target to *hold* its track:
    /// the track refreshes when `P(at least one glimpse this epoch) >= this`
    /// (`docs/THEORY.md` §10.1). Jamming, concealment, range and LOS all feed the rate,
    /// so this is what lets EW degrade a sensor enough to break an existing track.
    #[serde(default = "default_track_maintain_p")]
    pub track_maintain_p: f32,
    /// How fire allocation is solved each epoch (`docs/THEORY.md` §10.2):
    /// `"optimal"` (Hungarian, the default), `"greedy"`, or `"independent"` - every shooter
    /// choosing for itself.
    ///
    /// A dial rather than a constant so the cost of *not* coordinating is measurable on any
    /// scenario; `sweep --param sim.allocation` compares all three, paired.
    #[serde(default)]
    pub allocation: AllocationChoice,
    /// Most **air-defence batteries** that may be assigned to one airframe
    /// (`docs/THEORY.md` §11.2).
    ///
    /// Air only - ground fires have no cap (§11.4). A ground target is a multi-element unit
    /// that genuinely absorbs several shooters; an airframe is one object, so a second
    /// battery is insurance against the first missing and a missile is a countable round
    /// that can be wasted.
    #[serde(default = "default_max_batteries_per_air_target")]
    pub max_batteries_per_air_target: u32,
    /// Must a ground shooter be under a live friendly C2 post to join its side's
    /// coordinated fire plan (`docs/THEORY.md` §11.3)?
    ///
    /// **Off by default**, so ground fires coordinate side-wide for free - the §10.2
    /// assumption, defensible for a battlegroup sharing one fire-control net. On, a shooter
    /// inside a live post's (jammed) radius joins the side-wide assignment and one outside
    /// picks for itself, as air defence already works (§11.1).
    ///
    /// A dial rather than a change of rule: flipping it unconditionally would reduce every
    /// existing scenario to `independent`, re-baselining the §10.2 allocation result, V56
    /// and V39 at once. As a dial, the cost of losing the net is a number instead.
    #[serde(default)]
    pub fires_need_c2: bool,
    /// Should steerable sensors re-point themselves each epoch to maximise expected
    /// information gain (`docs/THEORY.md` §10.3)?
    ///
    /// **Off by default.** A `facing_deg` written in a scenario is a statement of intent,
    /// and overriding it would change what every existing scenario means. It would also
    /// dissolve the §6.3 interdiction game, whose Blue strategies *are* committed postures -
    /// a sensor that re-points itself is not playing a strategy (V39 catches this).
    ///
    /// Only affects sensors with a finite `for_width_deg`; an all-round sensor has no
    /// decision to make.
    #[serde(default = "default_sensor_tasking")]
    pub sensor_tasking: bool,
    /// How many decision epochs the fire-allocation objective looks ahead (§10.2).
    ///
    /// `1` scores only the epoch being decided, and is an exact identity (V75). Above 1, a
    /// target's threat is weighted by how long it would go on being dangerous if it
    /// survived, so killing a shooter is preferred to damaging several bystanders.
    ///
    /// The dial exists because solving a single-epoch objective *exactly* is myopically
    /// right and measurably worse over an engagement than a greedy rule that spreads fire.
    #[serde(default = "default_allocation_horizon")]
    pub allocation_horizon: u32,
    /// How heavily being *shootable* counts against being *observable* in the movement
    /// planner's risk raster (§5.2).
    ///
    /// `0` - the default and an exact identity - makes risk purely enemy observation. Above
    /// zero, ground inside an enemy weapon's reach is priced too, so "least-risk" stops
    /// meaning "least-observed".
    ///
    /// Two terms rather than one blended raster, because they are separate facts: a unit can
    /// be watched from ground nothing can shoot, and shelled from ground nothing can see.
    #[serde(default)]
    pub fire_risk_weight: f32,
    /// Default exchange rate between movement cost and exposure for a unit with an
    /// objective (§5.1). A unit may override it; `0` plans the shortest route regardless of
    /// who is watching.
    #[serde(default = "default_risk_weight")]
    pub risk_weight: f32,
    /// How much better a new route must be before a unit abandons the one it is on, as a
    /// fraction of the held route's cost (§10.5).
    ///
    /// Without it a unit re-deciding every epoch dithers between two near-equal routes as
    /// costs wobble - the movement analogue of the target lock (§13.4), and it takes the
    /// same answer: switching is itself a decision with a cost, so it should need something
    /// to change on the ground rather than a rounding difference.
    #[serde(default = "default_repath_margin")]
    pub repath_margin: f32,
    /// Edge length of the coarse belief grid, in cells (`docs/THEORY.md` §10.3).
    ///
    /// Belief runs at this resolution whatever the terrain size - tasking chooses between
    /// twelve 30° sectors and does not need 10 m fidelity to do it. Also the movement
    /// planner's grid (§10.5). The coverage raster behind it costs the square of this.
    #[serde(default = "default_belief_cells")]
    pub belief_cells: usize,
}

/// Which allocation solver a scenario asks for.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AllocationChoice {
    /// Kuhn-Munkres: the side-wide optimum.
    #[default]
    Optimal,
    /// Repeatedly take the best remaining pairing.
    Greedy,
    /// Each shooter picks for itself, ignoring the rest of the side.
    Independent,
}

impl From<AllocationChoice> for crate::allocation::Solver {
    fn from(c: AllocationChoice) -> Self {
        match c {
            AllocationChoice::Optimal => Self::Optimal,
            AllocationChoice::Greedy => Self::Greedy,
            AllocationChoice::Independent => Self::Independent,
        }
    }
}

fn default_dt_s() -> f32 {
    1.0
}

fn default_epoch_s() -> f32 {
    10.0
}

fn default_suppression_radius() -> f32 {
    35.0
}

fn default_p_suppress() -> f32 {
    0.15
}

fn default_recover_per_s() -> f32 {
    0.05
}

fn default_suppressed_fire_factor() -> f32 {
    0.4
}

fn default_track_hold() -> f32 {
    45.0
}

fn default_allocation_horizon() -> u32 {
    1
}

fn default_risk_weight() -> f32 {
    50.0
}

fn default_repath_margin() -> f32 {
    0.1
}

fn default_track_maintain_p() -> f32 {
    0.5
}

fn default_max_batteries_per_air_target() -> u32 {
    2
}

fn default_sensor_tasking() -> bool {
    false
}

fn default_belief_cells() -> usize {
    48
}

impl Default for SimConfig {
    fn default() -> Self {
        Self {
            dt_s: default_dt_s(),
            epoch_s: default_epoch_s(),
            suppression_radius_m: default_suppression_radius(),
            p_suppress: default_p_suppress(),
            recover_per_s: default_recover_per_s(),
            suppressed_fire_factor: default_suppressed_fire_factor(),
            track_hold_s: default_track_hold(),
            track_maintain_p: default_track_maintain_p(),
            allocation_horizon: default_allocation_horizon(),
            fire_risk_weight: 0.0,
            risk_weight: default_risk_weight(),
            repath_margin: default_repath_margin(),
            allocation: AllocationChoice::default(),
            max_batteries_per_air_target: default_max_batteries_per_air_target(),
            fires_need_c2: false,
            sensor_tasking: default_sensor_tasking(),
            belief_cells: default_belief_cells(),
        }
    }
}

/// One side's placed assets.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Force {
    /// Placed sensors.
    #[serde(default)]
    pub sensors: Vec<SensorInstance>,
    /// Placed units.
    #[serde(default)]
    pub units: Vec<UnitInstance>,
    /// Placed jammers (protect this side's units from enemy detection).
    #[serde(default)]
    pub jammers: Vec<JammerInstance>,
    /// Placed air assets - drones (`docs/THEORY.md` §9).
    #[serde(default)]
    pub air: Vec<AirInstance>,
    /// Placed air-defence batteries (`docs/THEORY.md` §9.4).
    #[serde(default)]
    pub air_defence: Vec<AirDefenceInstance>,
    /// Placed C2 posts, which coordinate nearby air defence (`docs/THEORY.md` §11).
    #[serde(default)]
    pub c2: Vec<C2Instance>,
    /// What this side has been told to shoot first (`docs/THEORY.md` §13).
    ///
    /// **Always present.** Omitting the block gives `priority = ["all"]` - one tier holding
    /// every target, which *is* the undirected §10.2 behaviour. So there is no "doctrine or
    /// not" branch anywhere downstream; the undirected case is simply the one-tier case.
    #[serde(default)]
    pub doctrine: Doctrine,
    /// Engagements ordered outright, bypassing the assignment (`docs/THEORY.md` §13.3).
    #[serde(default)]
    pub orders: Vec<Order>,
}

/// A placed C2 post: a type id from `c2.toml` plus where it is.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct C2Instance {
    /// Unique-in-scenario id.
    pub id: String,
    /// Key into the C2 type library.
    #[serde(rename = "type")]
    pub type_id: String,
    /// World position `[x, y]`, metres.
    pub pos: [f32; 2],
}

/// A placed air asset: a type id from `air.toml` plus where it is, how it is flying, and
/// what (if anything) it has been sent to attack.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AirInstance {
    /// Unique-in-scenario id.
    pub id: String,
    /// Key into the air-type library.
    #[serde(rename = "type")]
    pub type_id: String,
    /// World position, metres `[x, y]`.
    pub pos: [f32; 2],
    /// Altitude, metres, in the frame given by `altitude_ref`.
    #[serde(default)]
    pub altitude_m: f32,
    /// Is `altitude_m` above ground (`agl`, the default) or above sea level (`amsl`)?
    #[serde(default)]
    pub altitude_ref: AltitudeRef,
    /// Initial heading, degrees (0° = east, CCW).
    #[serde(default)]
    pub heading_deg: f32,
    /// Speed override, metres/second; omitted uses the type's cruise speed.
    #[serde(default)]
    pub speed_m_s: Option<f32>,
    /// Flight-plan waypoints as world points.
    #[serde(default)]
    pub waypoints: Vec<[f32; 2]>,
    /// What to do at the final waypoint: `"hold"` (default) or
    /// `{ orbit = { radius_m = .., clockwise = .. } }`.
    #[serde(default)]
    pub terminal: Terminal,
    /// Assigned strike target: `{ unit = "id" }` or `{ point = [x, y] }`. Omitted means
    /// the aim point is the final waypoint (`docs/THEORY.md` §9.3).
    #[serde(default)]
    pub target: Option<TargetConfig>,
}

/// A strike drone's assigned target, in scenario form (the runtime form is
/// [`crate::airframes::TargetSpec`], which carries a `Vec2`).
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetConfig {
    /// A named ground asset - a unit, an air-defence battery, or a C2 post. The TOML key
    /// stays `unit` for compatibility with scenarios written before batteries and posts
    /// were targetable; `asset` is the clearer alias and means the same thing.
    #[serde(alias = "asset")]
    Unit(String),
    /// A fixed ground point `[x, y]`.
    Point([f32; 2]),
}

/// A placed air-defence battery: a type id from `air_defence.toml` plus its position and
/// whether its organic sensor is switched on.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AirDefenceInstance {
    /// Unique-in-scenario id.
    pub id: String,
    /// Key into the air-defence type library.
    #[serde(rename = "type")]
    pub type_id: String,
    /// World position, metres `[x, y]`.
    pub pos: [f32; 2],
    /// Does the battery act on its **own** radar, or wait for a track over the net?
    ///
    /// `false` forces it onto the external cueing chain, so it always pays
    /// `cue_latency_s` (§9.5) - the lever for studying cued-from-elsewhere air defence.
    /// The radar still runs: this is about who the battery listens to, not about
    /// emission. For that, see `emitting`.
    #[serde(default = "default_true")]
    pub self_cue: bool,
    /// Is the organic radar **transmitting**?
    ///
    /// `false` is EMCON: the radar is off, so it detects nothing, cannot cue its own
    /// battery, and gives an anti-radiation missile nothing to home on (§12.3) - it
    /// lands with `silent_cep_m` instead of `cep_m`.
    ///
    /// Separate from `self_cue` because the two are separate decisions and were once the
    /// same flag. Sharing one meant a battery could take the missile protection of going
    /// dark while still seeing everything, which is the survivability of EMCON without
    /// its cost. A battery with no organic sensor at all is silent regardless.
    #[serde(default = "default_true")]
    pub emitting: bool,
}

fn default_true() -> bool {
    true
}

/// A placed jammer (`docs/THEORY.md` §8): position + degradation dials.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JammerInstance {
    /// World position, metres `[x, y]`.
    pub pos: [f32; 2],
    /// Peak detection degradation at the centre, `[0, 1]`.
    pub power: f32,
    /// Effect radius, metres.
    pub radius_m: f32,
}

/// A placed sensor: a type id from `sensors.toml` plus position and facing.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SensorInstance {
    /// Unique-in-scenario id, shown in event feeds.
    pub id: String,
    /// Key into the sensor-type library.
    #[serde(rename = "type")]
    pub type_id: String,
    /// World position, metres `[x, y]`.
    pub pos: [f32; 2],
    /// Facing, degrees (0° = east, CCW); matters only with a finite field of regard.
    #[serde(default)]
    pub facing_deg: f32,
}

/// A placed unit: a type id from `units.toml` plus position and an optional route.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnitInstance {
    /// Unique-in-scenario id.
    pub id: String,
    /// Key into the unit-type library.
    #[serde(rename = "type")]
    pub type_id: String,
    /// World position, metres `[x, y]` (the route start if a route is given).
    pub pos: [f32; 2],
    /// Optional movement route as world waypoints; empty = static.
    ///
    /// A **scripted** route: the unit follows it exactly, which is what every scenario did
    /// before §10.5 and what most still do.
    #[serde(default)]
    pub route: Vec<[f32; 2]>,
    /// Where the unit is trying to *get to*, if it is deciding its own route (§10.5).
    ///
    /// The alternative to `route`, and mutually exclusive with it. A unit with an objective
    /// re-plans each decision epoch against the live risk raster, so a sensor placed on its
    /// path changes where it goes - which a scripted route cannot express.
    ///
    /// Declaring **neither** is a static unit. That is what makes the identity structural
    /// rather than dial-gated: a scenario with no objective anywhere does no planning at
    /// all, rather than having a branch switched off (V72).
    #[serde(default)]
    pub objective: Option<[f32; 2]>,
    /// How many metres of movement cost this unit will spend to avoid one unit of exposure
    /// (§5.1's exchange rate `w`). Defaults to `[sim] risk_weight`.
    ///
    /// Per unit because it is a statement about *this* commander's caution, and because
    /// sweeping it on one unit while another holds still is how the trade between arriving
    /// quickly and arriving alive gets measured.
    #[serde(default)]
    pub risk_weight: Option<f32>,
}

/// The terrain block of a scenario: grid dimensions and how to generate the elevation.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerrainConfig {
    /// Square cell size, metres.
    pub cell_size_m: f32,
    /// Cells east.
    pub width_cells: usize,
    /// Cells north.
    pub height_cells: usize,
    /// How to generate the elevation raster.
    pub source: TerrainSource,
}

impl Scenario {
    /// Load and validate a scenario from a TOML file.
    ///
    /// # Errors
    /// [`ScenarioError::Io`] if the file can't be read, [`ScenarioError::Parse`] if it
    /// isn't valid TOML / schema, [`ScenarioError::Invalid`] if it fails validation.
    pub fn load(path: &Path) -> Result<Self, ScenarioError> {
        Self::from_toml_str(&read_to_string(path)?)
    }

    /// Parse and validate a scenario from an in-memory TOML string (used by tests and
    /// any non-file source).
    ///
    /// # Errors
    /// As [`Scenario::load`], minus the I/O case.
    pub fn from_toml_str(text: &str) -> Result<Self, ScenarioError> {
        let scenario: Scenario = toml::from_str(text)?; // `?` maps toml::de::Error via #[from]
        scenario.validate()?;
        Ok(scenario)
    }

    fn validate(&self) -> Result<(), ScenarioError> {
        let t = &self.terrain;
        if t.width_cells == 0 || t.height_cells == 0 {
            return Err(ScenarioError::Invalid(
                "terrain dimensions must be non-zero".into(),
            ));
        }
        // Reject zero, negative, NaN, and infinity in one explicit test.
        if !t.cell_size_m.is_finite() || t.cell_size_m <= 0.0 {
            return Err(ScenarioError::Invalid(
                "cell_size_m must be positive and finite".into(),
            ));
        }
        // A unit either follows a scripted route or decides its own (§10.5). Declaring both
        // is ambiguous - neither "plan, then ignore the plan" nor "follow the route, then
        // re-plan" is obviously meant - so it is refused at load rather than resolved by a
        // precedence rule nobody would remember. Same argument as `deny_unknown_fields`.
        for (side, force) in [("blue", &self.blue), ("red", &self.red)] {
            for u in &force.units {
                if !u.route.is_empty() && u.objective.is_some() {
                    return Err(ScenarioError::Invalid(format!(
                        "{side} unit '{}' declares both `route` and `objective`; a unit                          either follows a route or plans to an objective, not both",
                        u.id
                    )));
                }
                if let Some(w) = u.risk_weight {
                    require_non_negative(&format!("{side} unit '{}' risk_weight", u.id), w)?;
                }
            }
        }
        self.sim.validate()
    }

    /// Build this scenario's terrain with the given per-type dials and seed.
    ///
    /// Deterministic: same `(scenario, params, seed)` → bit-identical terrain.
    #[must_use]
    pub fn build_terrain(&self, params: &TerrainParamsTable, seed: u64) -> TerrainGrid {
        self.terrain.source.build(
            self.terrain.cell_size_m,
            self.terrain.width_cells,
            self.terrain.height_cells,
            seed,
            params,
        )
    }
}

mod input_contract;
mod loading;

use input_contract::{require_non_negative, require_positive};
use loading::read_to_string;

pub use loading::*;
