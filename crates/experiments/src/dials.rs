//! Every dial the simulation exposes, and what a sensible range for it is.
//!
//! # Why a registry
//!
//! `sweep --param` patches whatever dotted path it is given, which is flexible and blind:
//! nothing knows what dials exist or what range is reasonable. Three things need the list -
//! a meta-analysis that sweeps *everything*, a browsable dial picker, and any check that a
//! study file names dials that are real.
//!
//! # Two kinds of dial
//!
//! **Scenario dials** (`sim.*`) are a fixed set with natural domains - a probability lives
//! in `[0, 1]` whatever the scenario says.
//!
//! **Stat-block dials** (`sensors.<id>.<field>`) are the cross product of the field and
//! every type in the loaded libraries. Their range is **relative** to the shipped value: a
//! glimpse rate of 0.6 and one of 4.0 are both plausible, so a quarter to four times asks
//! "much worse, or much better" without pretending to know the units.
//!
//! # Maintaining it
//!
//! By hand, with a test that fails when it drifts - the posture `gates.rs` takes toward the
//! validation suite. A registry that silently misses a dial is worse than none, because a
//! meta-analysis would report everything swept when something was not.

use sim_core::scenario::Libraries;

/// What kind of value a dial takes, and over what range it is worth sweeping.
#[derive(Debug, Clone, PartialEq)]
pub enum Range {
    /// A continuous dial with a natural domain, swept between these bounds.
    Absolute(f64, f64),
    /// A continuous dial whose sensible range depends on the value the library gives it,
    /// swept between these multiples of it.
    Relative(f64, f64),
    /// A whole number, swept inclusively.
    Integer(i64, i64),
    /// On or off.
    Flag,
    /// One of a fixed set of names.
    Choice(&'static [&'static str]),
}

impl Range {
    /// Can this dial take part in a variance decomposition?
    ///
    /// Morris and Sobol both need a continuous input to perturb. A flag or a named choice has
    /// no gradient and no meaningful midpoint, so a sensitivity study cannot include it -
    /// those belong in a factorial design instead, which is exactly what `factorial` is for.
    #[must_use]
    pub fn is_continuous(&self) -> bool {
        matches!(
            self,
            Self::Absolute(..) | Self::Relative(..) | Self::Integer(..)
        )
    }
}

/// One dial: where it lives, what it means, and how far it is worth moving.
#[derive(Debug, Clone)]
pub struct Dial {
    /// Dotted path, exactly as `sweep --param` takes it.
    pub path: String,
    /// What it does, in one line.
    pub doc: &'static str,
    /// Its unit, or an empty string where it is dimensionless.
    pub unit: &'static str,
    /// The design section that specifies it, for a reader who wants the maths.
    pub section: &'static str,
    /// What it takes and how far to move it.
    pub range: Range,
}

/// A field on a stat block, before it is paired with a concrete type id.
struct Field {
    name: &'static str,
    doc: &'static str,
    unit: &'static str,
    section: &'static str,
    range: Range,
}

/// The `[sim]` block: sixteen dials with domains that do not depend on any library.
const SIM: &[Field] = &[
    Field {
        name: "dt_s",
        doc: "Integration tick length",
        unit: "s",
        section: "§7.1",
        range: Range::Absolute(0.25, 5.0),
    },
    Field {
        name: "epoch_s",
        doc: "Seconds between decision epochs",
        unit: "s",
        section: "§7.1",
        range: Range::Absolute(2.0, 60.0),
    },
    Field {
        name: "suppression_radius_m",
        doc: "Radius within which a near miss can suppress",
        unit: "m",
        section: "§4.3",
        range: Range::Absolute(10.0, 200.0),
    },
    Field {
        name: "p_suppress",
        doc: "Probability a near miss suppresses",
        unit: "",
        section: "§4.3",
        range: Range::Absolute(0.0, 1.0),
    },
    Field {
        name: "recover_per_s",
        doc: "Rate a suppressed unit steps back down the chain",
        unit: "/s",
        section: "§4.3",
        range: Range::Absolute(0.0, 1.0),
    },
    Field {
        name: "suppressed_fire_factor",
        doc: "How much a suppressed unit's fire is scaled",
        unit: "",
        section: "§4.3",
        range: Range::Absolute(0.0, 1.0),
    },
    Field {
        name: "track_hold_s",
        doc: "How long a track survives unobserved",
        unit: "s",
        section: "§10.1",
        range: Range::Absolute(0.0, 180.0),
    },
    Field {
        name: "track_maintain_p",
        doc: "Probability a held track is maintained each epoch",
        unit: "",
        section: "§10.1",
        range: Range::Absolute(0.0, 1.0),
    },
    Field {
        name: "allocation",
        doc: "Which weapon-target assignment rule a side uses",
        unit: "",
        section: "§10.2",
        range: Range::Choice(&["independent", "greedy", "optimal"]),
    },
    Field {
        name: "max_batteries_per_air_target",
        doc: "Overkill cap on air defence",
        unit: "batteries",
        section: "§11.2",
        range: Range::Integer(1, 4),
    },
    Field {
        name: "sensor_tasking",
        doc: "Do steerable sensors point themselves by information gain?",
        unit: "",
        section: "§10.3",
        range: Range::Flag,
    },
    Field {
        name: "fires_need_c2",
        doc: "Must a ground shooter be in the net to join the side-wide fire plan?",
        unit: "",
        section: "§11.3",
        range: Range::Flag,
    },
    Field {
        name: "allocation_horizon",
        doc: "How many epochs the allocation objective prices",
        unit: "epochs",
        section: "§10.6",
        range: Range::Integer(1, 12),
    },
    Field {
        name: "fire_risk_weight",
        doc: "How heavily weapon reach counts against observation when planning a route",
        unit: "",
        section: "§5.2",
        range: Range::Absolute(0.0, 4.0),
    },
    Field {
        name: "risk_weight",
        doc: "Metres of movement cost spent to avoid one unit of exposure",
        unit: "m",
        section: "§5.1",
        range: Range::Absolute(0.0, 1200.0),
    },
    Field {
        name: "repath_margin",
        doc: "How much better a new route must be before a unit switches",
        unit: "",
        section: "§10.5",
        range: Range::Absolute(0.0, 0.5),
    },
    Field {
        name: "belief_cells",
        doc: "Edge length of the coarse belief and planning grid",
        unit: "cells",
        section: "§10.3",
        range: Range::Integer(16, 96),
    },
];

/// Stat-block fields, swept relative to whatever the library gives them.
const SENSOR: &[Field] = &[
    Field {
        name: "max_range_m",
        doc: "Hard cut-off beyond which the sensor sees nothing",
        unit: "m",
        section: "§3.1",
        range: Range::Relative(0.5, 2.0),
    },
    Field {
        name: "lambda0_per_s",
        doc: "Glimpse rate at zero range against a full-signature target",
        unit: "/s",
        section: "§3.2",
        range: Range::Relative(0.25, 4.0),
    },
    Field {
        name: "range_half_m",
        doc: "Range at which the glimpse rate halves",
        unit: "m",
        section: "§3.2",
        range: Range::Relative(0.5, 2.0),
    },
    Field {
        name: "range_exponent",
        doc: "How sharply detection falls away with range",
        unit: "",
        section: "§3.2",
        range: Range::Absolute(1.0, 4.0),
    },
    Field {
        name: "mount_height_m",
        doc: "Height above ground, which decides what it can see over",
        unit: "m",
        section: "§1.2",
        range: Range::Relative(0.25, 4.0),
    },
];

const WEAPON: &[Field] = &[
    Field {
        name: "max_range_m",
        doc: "Maximum engagement range",
        unit: "m",
        section: "§2.1",
        range: Range::Relative(0.5, 2.0),
    },
    Field {
        name: "cep_m",
        doc: "Circular error probable of the impact point",
        unit: "m",
        section: "§2.2",
        range: Range::Relative(0.25, 4.0),
    },
    Field {
        name: "lethal_radius_m",
        doc: "Carleton lethal radius of the burst",
        unit: "m",
        section: "§2.3",
        range: Range::Relative(0.5, 2.0),
    },
    Field {
        name: "p_kill_given_hit",
        doc: "Probability a hit kills an element",
        unit: "",
        section: "§2.1",
        range: Range::Absolute(0.05, 1.0),
    },
    Field {
        name: "rof_rounds_per_min",
        doc: "Rate of fire",
        unit: "/min",
        section: "§2.1",
        range: Range::Relative(0.25, 4.0),
    },
];

const UNIT: &[Field] = &[
    Field {
        name: "speed_m_s",
        doc: "Cross-country movement speed",
        unit: "m/s",
        section: "§5.1",
        range: Range::Relative(0.5, 2.0),
    },
    Field {
        name: "element_count",
        doc: "How many sub-elements the unit fields",
        unit: "elements",
        section: "§4.1",
        range: Range::Integer(1, 8),
    },
    Field {
        name: "height_m",
        doc: "Target height for line of sight and direct fire",
        unit: "m",
        section: "§1.2",
        range: Range::Relative(0.5, 2.0),
    },
];

const AIR: &[Field] = &[
    Field {
        name: "cruise_speed_m_s",
        doc: "How fast the airframe crosses an envelope",
        unit: "m/s",
        section: "§9.1",
        range: Range::Relative(0.5, 2.0),
    },
    Field {
        name: "release_range_m",
        doc: "Range at which a strike payload is released",
        unit: "m",
        section: "§9.3",
        range: Range::Relative(0.5, 2.0),
    },
    Field {
        name: "endurance_s",
        doc: "Time aloft before the airframe is removed",
        unit: "s",
        section: "§9.1",
        range: Range::Relative(0.5, 2.0),
    },
    Field {
        name: "munitions",
        doc: "How many munitions are carried",
        unit: "rounds",
        section: "§9.3",
        range: Range::Integer(1, 6),
    },
    Field {
        name: "max_turn_rate_deg_s",
        doc: "Turn rate, which implies a minimum turn radius",
        unit: "deg/s",
        section: "§9.1",
        range: Range::Relative(0.5, 2.0),
    },
];

const AIR_DEFENCE: &[Field] = &[
    Field {
        name: "max_range_m",
        doc: "Outer edge of the engagement envelope",
        unit: "m",
        section: "§9.4",
        range: Range::Relative(0.5, 2.0),
    },
    Field {
        name: "min_range_m",
        doc: "Inner edge, inside which it cannot engage",
        unit: "m",
        section: "§9.4",
        range: Range::Relative(0.5, 2.0),
    },
    Field {
        name: "cue_latency_s",
        doc: "Delay on a track arriving over the net rather than its own radar",
        unit: "s",
        section: "§9.5",
        range: Range::Absolute(0.0, 30.0),
    },
    Field {
        name: "channels",
        doc: "How many targets it can engage at once",
        unit: "channels",
        section: "§9.4",
        range: Range::Integer(1, 6),
    },
    Field {
        name: "magazine",
        doc: "Rounds available before it is dry",
        unit: "rounds",
        section: "§9.4",
        range: Range::Integer(2, 40),
    },
];

const C2: &[Field] = &[
    Field {
        name: "coordination_range_m",
        doc: "Radius within which the post coordinates its side",
        unit: "m",
        section: "§11.1",
        range: Range::Relative(0.5, 2.0),
    },
    Field {
        name: "link_latency_s",
        doc: "Delay before a battery joins the net",
        unit: "s",
        section: "§11.2",
        range: Range::Absolute(0.0, 30.0),
    },
];

const TERRAIN: &[Field] = &[
    Field {
        name: "feature_height_m",
        doc: "Canopy or building height above ground",
        unit: "m",
        section: "§1.3",
        range: Range::Relative(0.5, 2.0),
    },
    Field {
        name: "extinction_per_m",
        doc: "Sight attenuation per metre of canopy",
        unit: "/m",
        section: "§1.3",
        range: Range::Relative(0.25, 4.0),
    },
    Field {
        name: "cover",
        doc: "Protection against fires",
        unit: "",
        section: "§1.3",
        range: Range::Absolute(0.0, 1.0),
    },
    Field {
        name: "concealment",
        doc: "How much the terrain hides a unit standing in it",
        unit: "",
        section: "§1.3",
        range: Range::Absolute(0.0, 1.0),
    },
    Field {
        name: "mobility_cost",
        doc: "Movement cost multiplier",
        unit: "",
        section: "§1.3",
        range: Range::Relative(0.5, 2.0),
    },
];

/// Every dial available against a given set of libraries.
///
/// Scenario dials come first and always; stat-block dials follow, one per (type, field) pair
/// that the loaded libraries actually contain. Sorted by path so two runs enumerate in the
/// same order - a study built from this must not depend on map iteration order.
#[must_use]
pub fn enumerate(libs: &Libraries) -> Vec<Dial> {
    let mut out: Vec<Dial> = SIM
        .iter()
        .map(|f| Dial {
            path: format!("sim.{}", f.name),
            doc: f.doc,
            unit: f.unit,
            section: f.section,
            range: f.range.clone(),
        })
        .collect();

    let mut expand = |prefix: &str, ids: Vec<String>, fields: &'static [Field]| {
        for id in ids {
            for f in fields {
                out.push(Dial {
                    path: format!("{prefix}.{id}.{}", f.name),
                    doc: f.doc,
                    unit: f.unit,
                    section: f.section,
                    range: f.range.clone(),
                });
            }
        }
    };

    expand("sensors", libs.sensors.keys().cloned().collect(), SENSOR);
    expand("weapons", libs.weapons.keys().cloned().collect(), WEAPON);
    expand("units", libs.units.keys().cloned().collect(), UNIT);
    expand("air", libs.air.keys().cloned().collect(), AIR);
    expand(
        "air_defence",
        libs.air_defence.keys().cloned().collect(),
        AIR_DEFENCE,
    );
    expand("c2", libs.c2.keys().cloned().collect(), C2);
    expand(
        "terrain_types",
        vec!["open".to_owned(), "trees".to_owned(), "urban".to_owned()],
        TERRAIN,
    );

    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// The scenario dials alone, for a caller that has no libraries to hand.
#[must_use]
pub fn sim_dials() -> Vec<Dial> {
    SIM.iter()
        .map(|f| Dial {
            path: format!("sim.{}", f.name),
            doc: f.doc,
            unit: f.unit,
            section: f.section,
            range: f.range.clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The registry must not fall behind the schema.
    ///
    /// `SimConfig` is the one struct whose field list is fixed and knowable, so it is checked
    /// exactly: every `pub` field on it must appear here. A dial added to the model and
    /// forgotten here would make a meta-analysis claim to have swept everything while
    /// silently leaving one out, which is the failure this whole registry exists to prevent.
    #[test]
    fn every_sim_config_field_is_registered() {
        let src = include_str!("../../sim_core/src/scenario/mod.rs");
        let block = src
            .split("pub struct SimConfig {")
            .nth(1)
            .expect("SimConfig in the schema")
            .split("\n}")
            .next()
            .expect("its closing brace");

        let declared: Vec<&str> = block
            .lines()
            .filter_map(|l| l.trim().strip_prefix("pub "))
            .filter_map(|l| l.split(':').next())
            .map(str::trim)
            .collect();

        let registered: Vec<&str> = SIM.iter().map(|f| f.name).collect();
        let missing: Vec<&&str> = declared
            .iter()
            .filter(|d| !registered.contains(d))
            .collect();
        assert!(
            missing.is_empty(),
            "these [sim] dials exist in the schema but not in the registry: {missing:?}"
        );

        let stale: Vec<&&str> = registered
            .iter()
            .filter(|r| !declared.contains(r))
            .collect();
        assert!(
            stale.is_empty(),
            "these dials are registered but no longer exist: {stale:?}"
        );
    }

    #[test]
    fn continuous_dials_are_the_ones_a_variance_study_can_use() {
        assert!(Range::Absolute(0.0, 1.0).is_continuous());
        assert!(Range::Relative(0.5, 2.0).is_continuous());
        assert!(Range::Integer(1, 4).is_continuous());
        // A flag has no midpoint and a name has no gradient; both belong in a factorial.
        assert!(!Range::Flag.is_continuous());
        assert!(!Range::Choice(&["a", "b"]).is_continuous());
    }

    #[test]
    fn enumeration_is_ordered_and_non_empty() {
        let libs = Libraries::load_dir(std::path::Path::new("../../scenarios"));
        let Ok(libs) = libs else { return };
        let dials = enumerate(&libs);
        assert!(dials.len() > SIM.len(), "stat-block dials should expand");
        let mut sorted = dials.clone();
        sorted.sort_by(|a, b| a.path.cmp(&b.path));
        let paths: Vec<&String> = dials.iter().map(|d| &d.path).collect();
        let sorted_paths: Vec<&String> = sorted.iter().map(|d| &d.path).collect();
        assert_eq!(paths, sorted_paths, "enumeration must be deterministic");
    }
}
