//! Scenario loading: TOML to structs to a deterministic [`TerrainGrid`]. The only I/O
//! the engine does. Spec: `docs/DESIGN.md` §1.

use crate::air::{AirType, AltitudeRef, Terminal};
use crate::air_defence::AirDefenceType;
use crate::c2::C2Type;
use crate::fires::WeaponType;
use crate::sensing::{SensorType, UnitType};
use crate::terrain::{TerrainGrid, TerrainParamsTable, TerrainSource};
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

/// A scenario: a named situation the engine can simulate — terrain, sim clock, and the
/// two forces' placed assets. Weapons join this schema in the fires phase.
///
/// # Unknown keys are rejected
///
/// Every struct in this schema carries `deny_unknown_fields`. Nearly all the dials have a
/// serde default, so without it a misspelt key — `track_hold` for `track_hold_s` — parses
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
    /// Sim clock configuration (`docs/DESIGN.md` §3.3); defaults if absent.
    #[serde(default)]
    pub sim: SimConfig,
    /// Blue force starting assets.
    #[serde(default)]
    pub blue: Force,
    /// Red force starting assets.
    #[serde(default)]
    pub red: Force,
}

/// Sim clock + suppression dials (`docs/DESIGN.md` §3.3, §4.3).
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
    /// (`docs/DESIGN.md` §10.1). This is what lets EW *break* a track rather than only
    /// prevent one: jam a tracked unit, nobody re-observes it, and the track lapses.
    #[serde(default = "default_track_hold")]
    pub track_hold_s: f32,
    /// How readily a sensor must still be able to see a target to *hold* its track:
    /// the track refreshes when `P(at least one glimpse this epoch) >= this`
    /// (`docs/DESIGN.md` §10.1). Jamming, concealment, range and LOS all feed the rate,
    /// so this is what lets EW degrade a sensor enough to break an existing track.
    #[serde(default = "default_track_maintain_p")]
    pub track_maintain_p: f32,
    /// How fire allocation is solved each epoch (`docs/DESIGN.md` §10.2):
    /// `"optimal"` (Hungarian, the default), `"greedy"`, or `"independent"` — the
    /// pre-Phase-10 rule where every shooter chose for itself.
    ///
    /// A dial rather than a constant so the cost of *not* coordinating is measurable on
    /// any scenario; `experiments/allocation_gap` sweeps all three.
    #[serde(default)]
    pub allocation: AllocationChoice,
    /// Most **ground shooters** that may be assigned to one target in an epoch. Caps
    /// overkill: a target still offers at most one slot per remaining element, whichever
    /// is smaller.
    #[serde(default = "default_max_shooters_per_target")]
    pub max_shooters_per_target: u32,
    /// Most **air-defence batteries** that may be assigned to one airframe
    /// (`docs/DESIGN.md` §11.2).
    ///
    /// A separate dial from the ground cap, because they answer different questions with
    /// different natural answers. A ground target is a multi-element unit that genuinely
    /// absorbs several shooters; an airframe is one object, so a second battery is
    /// insurance against the first missing and a third is nearly always waste.
    #[serde(default = "default_max_batteries_per_air_target")]
    pub max_batteries_per_air_target: u32,
    /// Must a ground shooter be under a live friendly C2 post to join its side's
    /// coordinated fire plan (`docs/DESIGN.md` §11.3)?
    ///
    /// **Off by default.** With it off, ground fires coordinate side-wide for free — the
    /// §10.2 assumption, defensible for a battlegroup sharing one fire-control net. With it
    /// on, a shooter inside a live post's (jammed) radius joins the side-wide assignment
    /// and a shooter outside falls back to picking for itself, exactly as air defence
    /// already works (§11.1).
    ///
    /// A dial rather than a change of rule, because flipping it unconditionally would
    /// silently turn every existing scenario into `independent` — re-baselining the Phase
    /// 10 allocation result, V56 and V39 at once, for a reason invisible in the scenario
    /// files. As a dial the cost of losing the net is *measurable* instead: sweep it.
    #[serde(default)]
    pub fires_need_c2: bool,
    /// Should steerable sensors re-point themselves each epoch to maximise expected
    /// information gain (`docs/DESIGN.md` §10.3)?
    ///
    /// **Off by default, deliberately.** A `facing_deg` written in a scenario is a
    /// statement of intent, and silently overriding it would change what every existing
    /// scenario means. It would also dissolve the §6.3 interdiction game, whose Blue
    /// strategies *are* committed postures — a sensor that re-points itself is no longer
    /// playing a strategy (V39 catches exactly this).
    ///
    /// Only affects sensors with a finite `for_width_deg`; an all-round sensor has no
    /// decision to make.
    #[serde(default = "default_sensor_tasking")]
    pub sensor_tasking: bool,
    /// Edge length of the coarse belief grid, in cells (`docs/DESIGN.md` §10.3).
    ///
    /// Belief runs at this resolution regardless of terrain size: tasking chooses between
    /// twelve 30° sectors and does not need 10 m fidelity to do it. The cost of the
    /// coverage raster behind it scales as the square of this.
    #[serde(default = "default_belief_cells")]
    pub belief_cells: usize,
}

/// Which allocation solver a scenario asks for.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AllocationChoice {
    /// Kuhn–Munkres: the side-wide optimum.
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

fn default_track_maintain_p() -> f32 {
    0.5
}

fn default_max_shooters_per_target() -> u32 {
    3
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
            allocation: AllocationChoice::default(),
            max_shooters_per_target: default_max_shooters_per_target(),
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
    /// Placed air assets — drones (`docs/DESIGN.md` §9).
    #[serde(default)]
    pub air: Vec<AirInstance>,
    /// Placed air-defence batteries (`docs/DESIGN.md` §9.4).
    #[serde(default)]
    pub air_defence: Vec<AirDefenceInstance>,
    /// Placed C2 posts, which coordinate nearby air defence (`docs/DESIGN.md` §11).
    #[serde(default)]
    pub c2: Vec<C2Instance>,
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
    /// the aim point is the final waypoint (`docs/DESIGN.md` §9.3).
    #[serde(default)]
    pub target: Option<TargetConfig>,
}

/// A strike drone's assigned target, in scenario form (the runtime form is
/// [`crate::air::TargetSpec`], which carries a `Vec2`).
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetConfig {
    /// A named ground asset — a unit, an air-defence battery, or a C2 post. The TOML key
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
    /// Use the battery's organic sensor? Setting this `false` forces the battery onto
    /// the external cueing chain, so it always pays `cue_latency_s` (§9.5) — the lever
    /// for studying cued-from-elsewhere air defence.
    #[serde(default = "default_self_cue")]
    pub self_cue: bool,
}

fn default_self_cue() -> bool {
    true
}

/// A placed jammer (`docs/DESIGN.md` §8): position + degradation dials.
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
    #[serde(default)]
    pub route: Vec<[f32; 2]>,
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
        Ok(())
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

/// Read a file, tagging the path onto any I/O error. Shared by every loader below, which
/// otherwise repeat the same five lines seven times.
fn read_to_string(path: &Path) -> Result<String, ScenarioError> {
    std::fs::read_to_string(path).map_err(|source| ScenarioError::Io {
        path: path.to_path_buf(),
        source,
    })
}

/// Parse a stat-block library — or the terrain-params table — from TOML **text**.
///
/// The string half of the `load_*` family below: each of those reads a file and calls
/// this. Public because a caller that already holds the text should not have to write it
/// back to disk to load it — `experiments`' sweep patches a dial in memory and parses the
/// result, exactly as `Scenario::from_toml_str` lets it do for a scenario.
///
/// # Errors
/// [`ScenarioError::Parse`] if the text is not valid TOML for `T`.
pub fn library_from_toml_str<T: serde::de::DeserializeOwned>(
    text: &str,
) -> Result<T, ScenarioError> {
    Ok(toml::from_str(text)?)
}

/// Load the per-terrain-type dials (`scenarios/terrain_types.toml`).
///
/// # Errors
/// As [`Scenario::load`] (validation is structural — serde requires every field).
pub fn load_terrain_params(path: &Path) -> Result<TerrainParamsTable, ScenarioError> {
    library_from_toml_str(&read_to_string(path)?)
}

/// Load the sensor-type library (`scenarios/sensors.toml`): a table of stat blocks
/// keyed by type id.
///
/// # Errors
/// As [`Scenario::load`].
pub fn load_sensor_types(path: &Path) -> Result<BTreeMap<String, SensorType>, ScenarioError> {
    library_from_toml_str(&read_to_string(path)?)
}

/// Load the unit-type library (`scenarios/units.toml`).
///
/// # Errors
/// As [`Scenario::load`].
pub fn load_unit_types(path: &Path) -> Result<BTreeMap<String, UnitType>, ScenarioError> {
    library_from_toml_str(&read_to_string(path)?)
}

/// Load the weapon-type library (`scenarios/weapons.toml`).
///
/// # Errors
/// As [`Scenario::load`].
pub fn load_weapon_types(path: &Path) -> Result<BTreeMap<String, WeaponType>, ScenarioError> {
    library_from_toml_str(&read_to_string(path)?)
}

/// Load the air-type library (`scenarios/air.toml`).
///
/// # Errors
/// As [`Scenario::load`].
pub fn load_air_types(path: &Path) -> Result<BTreeMap<String, AirType>, ScenarioError> {
    library_from_toml_str(&read_to_string(path)?)
}

/// Load the C2 type library (`scenarios/c2.toml`). Optional, like `air.toml`: a scenario
/// set without it simply has no way to coordinate air defence.
///
/// # Errors
/// As [`Scenario::load`].
pub fn load_c2_types(path: &Path) -> Result<BTreeMap<String, C2Type>, ScenarioError> {
    library_from_toml_str(&read_to_string(path)?)
}

/// Load the air-defence type library (`scenarios/air_defence.toml`).
///
/// # Errors
/// As [`Scenario::load`].
pub fn load_air_defence_types(
    path: &Path,
) -> Result<BTreeMap<String, AirDefenceType>, ScenarioError> {
    library_from_toml_str(&read_to_string(path)?)
}

/// Every stat-block library a scenario resolves its instances against.
///
/// Bundled into one struct rather than passed as a growing list of positional maps:
/// [`crate::sim::Sim::new`] takes `(scenario, libraries, seed)` and stays that way as new
/// asset classes arrive.
#[derive(Debug, Clone)]
pub struct Libraries {
    /// Per-terrain-type dials.
    pub terrain_params: TerrainParamsTable,
    /// Sensor stat blocks (`sensors.toml`).
    pub sensors: BTreeMap<String, SensorType>,
    /// Unit stat blocks (`units.toml`).
    pub units: BTreeMap<String, UnitType>,
    /// Weapon stat blocks (`weapons.toml`).
    pub weapons: BTreeMap<String, WeaponType>,
    /// Air stat blocks (`air.toml`).
    pub air: BTreeMap<String, AirType>,
    /// Air-defence stat blocks (`air_defence.toml`).
    pub air_defence: BTreeMap<String, AirDefenceType>,
    /// C2 post stat blocks (`c2.toml`).
    pub c2: BTreeMap<String, C2Type>,
}

impl Libraries {
    /// Terrain dials with every stat-block library empty — the base for tests that
    /// supply only the libraries they exercise:
    /// `Libraries { units, ..Libraries::with_terrain(params) }`.
    #[must_use]
    pub fn with_terrain(terrain_params: TerrainParamsTable) -> Self {
        Self {
            terrain_params,
            sensors: BTreeMap::new(),
            units: BTreeMap::new(),
            weapons: BTreeMap::new(),
            air: BTreeMap::new(),
            air_defence: BTreeMap::new(),
            c2: BTreeMap::new(),
        }
    }

    /// Load every library from a `scenarios/`-shaped directory. The air and air-defence
    /// libraries are optional: a directory without them loads as empty maps, so
    /// pre-Phase-9 scenario sets still work.
    ///
    /// # Errors
    /// As [`Scenario::load`], for any library that exists but fails to parse.
    pub fn load_dir(dir: &Path) -> Result<Self, ScenarioError> {
        Ok(Self {
            terrain_params: load_terrain_params(&dir.join("terrain_types.toml"))?,
            sensors: load_sensor_types(&dir.join("sensors.toml"))?,
            units: load_unit_types(&dir.join("units.toml"))?,
            weapons: load_weapon_types(&dir.join("weapons.toml"))?,
            air: load_optional(&dir.join("air.toml"), load_air_types)?,
            air_defence: load_optional(&dir.join("air_defence.toml"), load_air_defence_types)?,
            c2: load_optional(&dir.join("c2.toml"), load_c2_types)?,
        })
    }
}

/// Load a library only if the file is there, else an empty map.
///
/// A generic `fn` rather than a closure: a closure would be monomorphised to the first
/// element type it is called with, so a second call for a different library would not
/// type-check.
fn load_optional<T>(
    path: &Path,
    load: fn(&Path) -> Result<BTreeMap<String, T>, ScenarioError>,
) -> Result<BTreeMap<String, T>, ScenarioError> {
    if path.exists() {
        load(path)
    } else {
        Ok(BTreeMap::new())
    }
}
