//! Command and control: who is allowed to coordinate with whom.
//! Spec: `docs/DESIGN.md` §11. Gates: V59.
//!
//! # Why coordination is an asset, not a switch
//!
//! Ground fires coordinate side-wide for free (§10.2) — a modelling simplification that
//! is defensible for a battlegroup sharing one fire-control net. Air defence is different:
//! point-defence batteries are genuinely autonomous unless something is deliberately
//! fielded to tie them together, and that something can be jammed, moved, or destroyed.
//!
//! So a **C2 post** is a placed asset with a coordination radius. Batteries inside a live
//! friendly post's radius allocate as one group; batteries outside act on their own. The
//! consequence is the interesting part: destroying the post does not kill a single
//! battery, but the defence **decoheres** — every battery reverts to shooting at whatever
//! is nearest, and the raid gets the duplicated engagements and the leakers that follow
//! from that.
//!
//! That makes "suppress the enemy's command post" a strictly better first move than
//! "shoot one more launcher", which is the real-world result the model should produce
//! without being told to.

use crate::sim::Side;
use glam::Vec2;

/// A C2 post's stat block (`scenarios/c2.toml`). Placeholder dials, as everywhere.
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct C2Type {
    /// How far the post can coordinate, metres. A battery within this radius of a live
    /// friendly post joins its coordinated group.
    ///
    /// Deliberately a plain radius rather than a line-of-sight or terrain-aware link:
    /// the interesting variable here is *whether coordination exists at all*, and a
    /// radius makes that legible on the map. A comms model with terrain masking is a
    /// clean later refinement — the §9.5 cue-latency machinery is the natural seam.
    pub coordination_range_m: f32,
    /// How long an asset must have been inside the radius before it is actually in the
    /// net, seconds (`docs/DESIGN.md` §11.2).
    ///
    /// Joining a fire-control net is not instantaneous: the battery has to be handed the
    /// air picture and told what it is now responsible for. **Defaults to zero**, so the
    /// pre-latency behaviour is exactly recovered — and so a sweep can turn this on
    /// *alone*, without the jamming effect confounding it.
    ///
    /// Matters mainly for a post or battery that moves. Emplaced assets pay it once at
    /// t = 0 and then never again, which is the honest answer: a static defence is set up
    /// before the raid arrives.
    #[serde(default)]
    pub link_latency_s: f32,
    /// Height above ground, metres — for LOS as a *target*, since a post is something
    /// the enemy will want to find and kill.
    #[serde(default = "default_height")]
    pub height_m: f32,
    /// Silhouette width as a target, metres.
    #[serde(default = "default_width")]
    pub silhouette_width_m: f32,
    /// Per-modality signature, as [`crate::sensing::UnitType::signature`]. A command post
    /// is typically *more* conspicuous than a launcher, not less — antennas and vehicle
    /// concentration — which is what makes it findable.
    #[serde(default)]
    pub signature: std::collections::BTreeMap<String, f32>,
    /// How many vehicles the post is made of; attrition removes them one at a time
    /// (`docs/DESIGN.md` §12).
    #[serde(default = "default_elements")]
    pub element_count: u32,
}

fn default_elements() -> u32 {
    1
}

fn default_height() -> f32 {
    3.0
}

fn default_width() -> f32 {
    4.0
}

impl Default for C2Type {
    fn default() -> Self {
        Self {
            coordination_range_m: 5000.0,
            link_latency_s: 0.0,
            height_m: default_height(),
            silhouette_width_m: default_width(),
            signature: std::collections::BTreeMap::new(),
            element_count: default_elements(),
        }
    }
}

/// A placed C2 post.
#[derive(Clone, Debug)]
pub struct C2State {
    /// Scenario id.
    pub id: String,
    /// Owning side — a post only coordinates its own side's assets.
    pub side: Side,
    /// World position, metres.
    pub pos: Vec2,
    /// Resolved stat block.
    pub stats: C2Type,
    /// Vehicles remaining. Zero means destroyed, and the defence it was coordinating
    /// reverts to acting independently from the next tick.
    pub elements: u32,
}

impl C2State {
    /// Place a post.
    #[must_use]
    pub fn new(id: &str, side: Side, pos: Vec2, stats: C2Type) -> Self {
        Self {
            id: id.to_owned(),
            side,
            pos,
            elements: stats.element_count.max(1),
            stats,
        }
    }

    /// Still functioning?
    #[must_use]
    pub fn alive(&self) -> bool {
        self.elements > 0
    }

    /// Does this post coordinate an asset at `pos`?
    ///
    /// Horizontal range, not slant: a coordination link is a communications relationship,
    /// not a sightline, and the §9.1 slant convention exists for *sensing and weapon*
    /// ranges. Using it here would make a post on a hill mysteriously worse at talking to
    /// the battery beneath it.
    #[must_use]
    pub fn covers(&self, pos: Vec2) -> bool {
        self.covers_jammed(pos, 1.0)
    }

    /// As [`C2State::covers`], with the post's link degraded by EW.
    ///
    /// `link_quality` is the [`crate::ew::jamming_factor`] at the post: `1` clear, `→ 0`
    /// blinded. It scales the **radius**, so jamming does not flip the link off — it pulls
    /// it in, and a battery sitting on top of the post keeps talking to it while the ones
    /// on the flanks fall out of the net first. That is the right shape: a comms link
    /// degrades with range against a noise floor, and raising the floor is what a jammer
    /// does.
    ///
    /// With no jammers the factor is exactly `1` and this is bit-for-bit [`C2State::covers`]
    /// — the same identity posture EW takes everywhere else (§8, V40).
    #[must_use]
    pub fn covers_jammed(&self, pos: Vec2, link_quality: f32) -> bool {
        self.alive() && self.pos.distance(pos) <= self.stats.coordination_range_m * link_quality
    }
}
