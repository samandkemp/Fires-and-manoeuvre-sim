//! The kill chain: what a side has been *told* to shoot first.
//! Spec: `docs/DESIGN.md` §13. Gates: V66.
//!
//! # Why this is not just another weight on the payoff
//!
//! §10.2 allocates fire by maximising `P(kill) × value`, which is what an omniscient
//! optimiser would do. Real crews are not omniscient optimisers. They do not hold a
//! kill-probability table; they hold **orders** - engage air defence before manoeuvre,
//! shoot the command post first - and they follow them whether or not the shot is a good
//! one.
//!
//! So a declared priority is **strict by default**: a shooter that can reach anything in a
//! higher tier takes it, even at a worse kill probability than a lower-tier target offers.
//! That is not a crude approximation of the optimiser; it is a different and, for a directed
//! force, more faithful decision rule.
//!
//! Which makes the mode switch a measurable question rather than a preference. Running the
//! same scenario under `strict` doctrine and under the payoff-optimal allocation puts a
//! number on **what directive control costs against optimal control** - an answer this
//! model can give and hand-waving cannot.
//!
//! # What a priority entry may name
//!
//! Three things, checked in this order and all equally valid:
//!
//! | Entry | Matches |
//! |---|---|
//! | an asset **id** | that one asset - how a gate pins an exact target |
//! | a **role** | every asset whose stat block declares it (`role = "artillery"`) |
//! | a **class** | `unit`, `air_defence`, `c2`, `air` - always available, no declaration |
//! | [`ALL`] | anything at all - the tier that says "and then everyone else, equally" |
//!
//! # There is no "no doctrine"
//!
//! A side always has one. Omitting the block gives `priority = ["all"]`: a single tier
//! holding every target, ranked among itself by the ordinary §10.2 payoff - which *is* the
//! undirected behaviour. So the engine has one code path rather than two, and the identity
//! with the pre-doctrine model holds **by construction** (one tier means one solve over
//! every target, which is exactly what the old code did) rather than by a separate branch
//! that has to be kept honest.
//!
//! `"all"` is usable mid-list too, which makes the bottom tier explicit:
//! `["c2", "air_defence", "all"]` reads as the fire plan it is.
//!
//! A role never masks its class: a battery with `role = "sam"` matches both `"sam"` and
//! `"air_defence"`, so a coarse doctrine keeps working when a stat block gets more specific.
//!
//! Every name is checked against the scenario when the sim is built. A priority naming
//! nothing is a load error, not an empty tier - the same reasoning as the schema's
//! `deny_unknown_fields`: a tier that silently matches nothing produces a study of a
//! doctrine nobody is following.

use std::collections::BTreeSet;

/// The universal-match name: a tier containing everything not already claimed above it.
///
/// Also the whole of the default priority, which is what lets "this side has no fire plan"
/// and "this side's fire plan is one tier" be the *same* case rather than two.
pub const ALL: &str = "all";

/// How a declared priority combines with the §10.2 payoff.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DoctrineMode {
    /// Tier decides first; the payoff only breaks ties *within* a tier.
    ///
    /// The default, because a side that has bothered to write a priority list means it.
    /// Implemented by solving the assignment one tier at a time, which makes the ordering
    /// exact - no large-number bonus that float arithmetic could quietly swallow.
    #[default]
    Strict,
    /// Priority multiplies the target's value; the payoff still decides.
    ///
    /// Doctrine as a thumb on the scale rather than an instruction. Tier `k` is scaled by
    /// `falloff^-k`, so a higher tier is preferred *when the shot is comparable* and a
    /// certain kill can still outrank a hopeless one two tiers up.
    Weighted,
}

/// A side's target priority. Always present - see the module header.
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Doctrine {
    /// Ordered, highest priority first. Each entry names an id, a role, or a class.
    pub priority: Vec<String>,
    /// How the priority is applied.
    #[serde(default)]
    pub mode: DoctrineMode,
    /// `Weighted` only: how sharply value falls away down the list. Tier `k` is multiplied
    /// by `falloff^-k`, so 2.0 halves the value of each successive tier.
    ///
    /// Values at or below 1 make every tier equal, which would silently turn doctrine off;
    /// [`Doctrine::falloff`] clamps to just above 1 so a mistyped dial degrades to "almost
    /// no preference" rather than to "no preference at all, and no way to tell".
    #[serde(default = "default_falloff")]
    pub weight_falloff: f32,
}

fn default_falloff() -> f32 {
    2.0
}

impl Default for Doctrine {
    /// One tier holding everything: the undirected model, expressed as a fire plan.
    fn default() -> Self {
        Self {
            priority: vec![ALL.to_owned()],
            mode: DoctrineMode::Strict,
            weight_falloff: default_falloff(),
        }
    }
}

impl Doctrine {
    /// Is this the default - one tier, everything equal?
    ///
    /// Not used to branch the allocation, which has one path either way. It is what lets a
    /// front-end say "no fire plan" instead of printing `["all"]` at someone.
    #[must_use]
    pub fn is_undirected(&self) -> bool {
        self.priority.len() == 1 && self.priority[0] == ALL
    }

    /// The clamped falloff actually used.
    #[must_use]
    pub fn falloff(&self) -> f32 {
        self.weight_falloff.max(1.000_001)
    }

    /// Which tier a target sits in: its index in the priority list, or the bottom tier
    /// (`priority.len()`) if nothing names it.
    ///
    /// The **first** matching entry wins, so a list can name one battery by id and then the
    /// whole class beneath it - `["sam-1", "air_defence"]` singles out that launcher and
    /// leaves the others a tier lower.
    #[must_use]
    pub fn tier_of(&self, names: &TargetNames) -> usize {
        self.priority
            .iter()
            .position(|p| names.matches(p))
            .unwrap_or(self.priority.len())
    }

    /// The value multiplier for a tier under [`DoctrineMode::Weighted`].
    #[must_use]
    pub fn weight_for_tier(&self, tier: usize) -> f32 {
        self.falloff().powi(-(tier as i32))
    }

    /// How many tiers there are, including the implicit bottom one.
    #[must_use]
    pub fn tier_count(&self) -> usize {
        self.priority.len() + 1
    }
}

/// The names one asset answers to: its id, its declared role, and its class.
///
/// Built per target rather than matched inline so the three-way rule lives in one place -
/// and so the load-time check and the per-epoch lookup cannot disagree about what a
/// priority entry means.
pub struct TargetNames<'a> {
    /// The asset's scenario id.
    pub id: &'a str,
    /// Its stat block's `role`, if it declared one.
    pub role: Option<&'a str>,
    /// Its asset class: `unit`, `air_defence`, `c2` or `air`.
    pub class: &'a str,
}

impl TargetNames<'_> {
    /// Does `name` refer to this asset?
    ///
    /// [`ALL`] refers to everything, which is what makes the default priority a single
    /// tier containing the whole field.
    #[must_use]
    pub fn matches(&self, name: &str) -> bool {
        name == ALL || self.id == name || self.role == Some(name) || self.class == name
    }
}

/// Every name anything on the field answers to - the vocabulary a priority list may use.
///
/// Collected once when the sim is built, so an unmatched entry is caught at load with a
/// list of what *would* have worked, rather than becoming an empty tier nobody notices.
#[derive(Default, Debug)]
pub struct Vocabulary(pub BTreeSet<String>);

impl Vocabulary {
    /// Add everything one asset answers to.
    pub fn insert(&mut self, names: &TargetNames) {
        // Always valid, and valid even on an empty map: "engage everything" is a coherent
        // instruction to a side with nothing to engage.
        self.0.insert(ALL.to_owned());
        self.0.insert(names.id.to_owned());
        self.0.insert(names.class.to_owned());
        if let Some(r) = names.role {
            self.0.insert(r.to_owned());
        }
    }

    /// The first priority entry that names nothing on the field, if any.
    #[must_use]
    pub fn first_unmatched(&self, priority: &[String]) -> Option<String> {
        priority.iter().find(|p| !self.0.contains(*p)).cloned()
    }

    /// Everything that *would* have matched, for an error message.
    #[must_use]
    pub fn known(&self) -> String {
        self.0.iter().cloned().collect::<Vec<_>>().join(", ")
    }
}

/// One directly ordered engagement: this shooter, that target, no solver involved.
///
/// The bluntest instrument here, and the one a gate usually wants. An ordered shooter is
/// removed from the assignment problem entirely, so "gun-a engages sam-1" is a fact about
/// the run rather than a likely outcome of it. Everything not under orders is allocated
/// normally, so a scenario can pin one pairing and let the rest be solved.
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Order {
    /// Id of the shooter - a unit or an air-defence battery.
    pub shooter: String,
    /// Id of what it is to engage.
    pub target: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names<'a>(id: &'a str, role: Option<&'a str>, class: &'a str) -> TargetNames<'a> {
        TargetNames { id, role, class }
    }

    #[test]
    fn a_name_may_be_an_id_a_role_or_a_class() {
        let sam = names("sam-1", Some("sam"), "air_defence");
        assert!(sam.matches("sam-1"), "by id");
        assert!(sam.matches("sam"), "by role");
        assert!(sam.matches("air_defence"), "by class");
        assert!(!sam.matches("ciws"));
        // A role must not mask the class, or a coarse doctrine would stop working the
        // moment a stat block became more specific.
        let plain = names("ad-2", None, "air_defence");
        assert!(plain.matches("air_defence"));
    }

    #[test]
    fn the_first_matching_entry_decides_the_tier() {
        let doc = Doctrine {
            priority: vec!["sam-1".to_owned(), "air_defence".to_owned()],
            mode: DoctrineMode::Strict,
            weight_falloff: 2.0,
        };
        assert_eq!(doc.tier_of(&names("sam-1", Some("sam"), "air_defence")), 0);
        assert_eq!(doc.tier_of(&names("sam-2", Some("sam"), "air_defence")), 1);
        // Unlisted falls to the implicit bottom tier, not out of the problem.
        assert_eq!(doc.tier_of(&names("tank-1", None, "unit")), 2);
        assert_eq!(doc.tier_count(), 3);
    }

    #[test]
    fn weights_fall_away_down_the_list() {
        let doc = Doctrine {
            priority: vec!["a".to_owned(), "b".to_owned()],
            mode: DoctrineMode::Weighted,
            weight_falloff: 2.0,
        };
        assert!((doc.weight_for_tier(0) - 1.0).abs() < 1e-6);
        assert!((doc.weight_for_tier(1) - 0.5).abs() < 1e-6);
        assert!((doc.weight_for_tier(2) - 0.25).abs() < 1e-6);
    }

    /// A falloff of 1 or less would make every tier equal - doctrine silently off, with
    /// nothing in the output to say so.
    #[test]
    fn a_degenerate_falloff_is_clamped_rather_than_ignored() {
        for bad in [1.0f32, 0.5, 0.0, -3.0] {
            let doc = Doctrine {
                priority: vec!["a".to_owned()],
                mode: DoctrineMode::Weighted,
                weight_falloff: bad,
            };
            assert!(
                doc.weight_for_tier(1) < doc.weight_for_tier(0),
                "falloff {bad} must still prefer the higher tier"
            );
        }
    }

    #[test]
    fn an_unmatched_priority_entry_is_reported_with_what_would_have_worked() {
        let mut vocab = Vocabulary::default();
        vocab.insert(&names("gun-1", Some("artillery"), "unit"));
        vocab.insert(&names("sam-1", None, "air_defence"));

        assert_eq!(vocab.first_unmatched(&["artillery".to_owned()]), None);
        assert_eq!(vocab.first_unmatched(&["air_defence".to_owned()]), None);
        assert_eq!(
            vocab.first_unmatched(&["artilery".to_owned()]),
            Some("artilery".to_owned()),
            "a typo must be caught, not become an empty tier"
        );
        assert!(vocab.known().contains("artillery"));
    }
}
