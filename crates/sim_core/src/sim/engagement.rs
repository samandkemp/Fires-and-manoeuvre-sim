//! Ground fires: who shoots whom, and what one epoch of shooting does.
//! Spec: `docs/DESIGN.md` §2, §4.1–§4.2, §10.2. Gates: V19–V24, V30, V31, V56.
//!
//! One epoch:
//!
//! 1. [`Sim::allocate_fires`] — each side assigns **all** its shooters at once, by
//!    solving a weapon–target assignment problem (§10.2). Direct fire needs LOS and
//!    range; indirect needs a live *track* and range, but no LOS (it arcs over).
//! 2. Work out each shot once — range, hit probability, dispersion — since none of it
//!    varies across the burst.
//! 3. Fire `rof × epoch × elements` rounds, each resolving against the target.
//! 4. Apply suppression from the near-misses afterwards, in a fixed unit order.
//!
//! Fire volume scaling with *live elements* is what makes an aimed-fire duel obey
//! Lanchester's square law (V30): lose half your strength and you lose half your output.
//!
//! Allocation is deterministic and draws no randomness; only the rounds roll dice, and
//! they still roll in shooter index order. That is what keeps the draw stream comparable
//! with the pre-allocation engine (V58).

use super::{FireEvent, FireTarget, Side, Sim};
use crate::allocation::{self, Solver};
use crate::doctrine::{Doctrine, DoctrineMode, TargetNames};
use crate::fires::{self, WeaponClass, WeaponType};
use crate::los;
use glam::Vec2;
use rand::Rng;

/// Firing height of a ground shooter above its own ground, metres — the sightline and
/// slant-range origin for direct fire.
pub(super) const SHOOTER_HEIGHT_M: f32 = 2.0;

/// Everything about a burst that does not change from round to round.
///
/// Range, hit probability and dispersion depend only on the shooter, the target and the
/// weapon, all fixed for the epoch — so they are computed once here rather than per
/// round. Only the dice differ between rounds.
enum Shot {
    /// Direct fire: a per-round kill probability against one element.
    Direct { p_kill: f32 },
    /// Indirect fire: a burst dispersion, plus the two damage modifiers.
    ///
    /// `cover` and `effectiveness` are kept as separate fields rather than pre-multiplied
    /// into one factor. Float multiplication is not associative, so folding them would
    /// give `d·(c·e)` where the original computed `(d·c)·e` — a difference of one ulp,
    /// but enough to flip a knife-edge kill roll and silently re-baseline V22/V24.
    Indirect {
        sigma_m: f32,
        lethal_radius_m: f32,
        cover: f32,
        effectiveness: f32,
    },
}

/// What ground fires need to know about a target, whichever list it lives in.
///
/// Units, air-defence batteries and C2 posts differ in what they *do* and in nothing that
/// matters to a shell. Gathering the four facts a shot depends on — where, how big, how
/// many left, is it locatable — is what let counter-battery be added by widening a list
/// rather than by writing a second fires model (`docs/DESIGN.md` §12.4).
#[derive(Clone, Copy)]
pub(super) struct TargetState {
    pub pos: Vec2,
    pub height_m: f32,
    pub silhouette_width_m: f32,
    pub elements: u32,
    /// Can the enemy put indirect fire on it? For a unit this is its track (§10.1); for a
    /// battery or post see [`Sim::emplacement_is_located`].
    pub located: bool,
    /// `value` from the stat block, if the scenario set one.
    pub declared_value: Option<f32>,
    /// Raw threat score, for the derived value when none is declared.
    pub threat: f32,
}

impl Sim {
    /// The names a target answers to in a priority list (`docs/DESIGN.md` §13.1).
    pub(super) fn target_names(&self, t: FireTarget) -> TargetNames<'_> {
        match t {
            FireTarget::Unit(i) => TargetNames {
                id: &self.units[i].id,
                role: self.units[i].stats.role.as_deref(),
                class: "unit",
            },
            FireTarget::AirDefence(i) => TargetNames {
                id: &self.air_defence[i].id,
                role: self.air_defence[i].stats.role.as_deref(),
                class: "air_defence",
            },
            FireTarget::C2(i) => TargetNames {
                id: &self.c2[i].id,
                role: self.c2[i].stats.role.as_deref(),
                class: "c2",
            },
        }
    }

    /// Every name on the field, for the load-time doctrine check. Includes airframes and
    /// shooters, which are not ground-fire *targets* but may be named by an order or by
    /// air-defence doctrine.
    pub(super) fn all_target_names(&self) -> Vec<TargetNames<'_>> {
        let mut out: Vec<TargetNames<'_>> = (0..self.units.len())
            .map(|i| self.target_names(FireTarget::Unit(i)))
            .collect();
        out.extend(
            (0..self.air_defence.len()).map(|i| self.target_names(FireTarget::AirDefence(i))),
        );
        out.extend((0..self.c2.len()).map(|i| self.target_names(FireTarget::C2(i))));
        out.extend(self.air.iter().map(|a| TargetNames {
            id: &a.id,
            role: a.stats.role.as_deref(),
            class: "air",
        }));
        out
    }

    /// This side's target priority. Always present — the undirected case is one tier
    /// holding everything (`docs/DESIGN.md` §13).
    pub(super) fn doctrine_of(&self, side: Side) -> &Doctrine {
        &self.doctrine[side as usize]
    }

    /// The fire-relevant facts about a target, whichever list it is in.
    pub(super) fn target_state(&self, t: FireTarget) -> TargetState {
        match t {
            FireTarget::Unit(i) => {
                let u = &self.units[i];
                TargetState {
                    pos: u.pos,
                    height_m: u.stats.height_m,
                    silhouette_width_m: u.stats.silhouette_width_m,
                    elements: u.elements,
                    located: u.detected,
                    declared_value: u.stats.value,
                    threat: Self::raw_threat(u),
                }
            }
            FireTarget::AirDefence(i) => {
                let d = &self.air_defence[i];
                TargetState {
                    pos: d.pos,
                    height_m: d.stats.height_m,
                    silhouette_width_m: d.stats.silhouette_width_m,
                    elements: d.elements,
                    located: self.emplacement_is_located(t),
                    declared_value: d.stats.value,
                    // No derived threat: a battery's danger is to aircraft, which is not
                    // measurable on the same scale as a unit's rof x lethality x reach.
                    // See `target_value`.
                    threat: 0.0,
                }
            }
            FireTarget::C2(i) => {
                let c = &self.c2[i];
                TargetState {
                    pos: c.pos,
                    height_m: c.stats.height_m,
                    silhouette_width_m: c.stats.silhouette_width_m,
                    elements: c.elements,
                    located: self.emplacement_is_located(t),
                    declared_value: c.stats.value,
                    // A post has no firepower at all (§12.2), so no derived threat either.
                    threat: 0.0,
                }
            }
        }
    }

    /// Remove `n` elements from whatever list the target lives in.
    fn apply_casualties(&mut self, t: FireTarget, n: u32) {
        match t {
            FireTarget::Unit(i) => {
                self.units[i].elements = self.units[i].elements.saturating_sub(n);
            }
            FireTarget::AirDefence(i) => {
                self.air_defence[i].elements = self.air_defence[i].elements.saturating_sub(n);
            }
            FireTarget::C2(i) => {
                self.c2[i].elements = self.c2[i].elements.saturating_sub(n);
            }
        }
    }

    /// Can the enemy put **indirect** fire on this emplacement (`docs/DESIGN.md` §12.4)?
    ///
    /// Neither batteries nor posts go through the §3.2 glimpse loop, so neither has a
    /// track in the ordinary sense. Rather than invent one, this asks the question
    /// counter-battery acquisition actually asks: **has it given itself away?**
    ///
    /// - A **battery** has, if it is transmitting (`emitting` with a live radar) or has
    ///   fired. Those are the two real ways a site is located: ESM on its emissions, or a
    ///   counter-battery track back along its rounds.
    /// - A **post** has, if it is coordinating anything — a command post is found because
    ///   it is talking, which is the same argument in a different band.
    ///
    /// Deterministic, and draws **no randomness**. That is not just tidiness: a stochastic
    /// acquisition here would insert draws into every scenario fielding air defence and
    /// shift the stream underneath V50, V51, V59 and V60 for no modelling gain.
    ///
    /// It also joins the two halves of §12.3 — switching a radar off already made an ARM
    /// miss; it now also hides the battery from artillery. One decision, two consequences.
    ///
    /// Public because it is a question worth asking from outside: it is what a gate checks
    /// directly rather than inferring from a hit, and what a front-end would draw to show
    /// which emplacements have given themselves away.
    #[must_use]
    pub fn emplacement_is_located(&self, t: FireTarget) -> bool {
        match t {
            FireTarget::Unit(_) => false,
            FireTarget::AirDefence(i) => {
                let d = &self.air_defence[i];
                let emitting = d.emitting && d.sensor_idx.is_some_and(|s| self.sensor_active(s));
                // `last_fired_s`, not a scan of the event log: this test runs once per
                // (shooter, target) pair per epoch, and the log grows for the whole run, so
                // the scan made the cost of an epoch depend on how long the battle had
                // already lasted. Same answer — both ask whether any engagement has
                // resolved — at O(1).
                d.alive() && (emitting || d.last_fired_s.is_some())
            }
            FireTarget::C2(i) => {
                let post = &self.c2[i];
                post.alive()
                    && self.air_defence.iter().any(|d| {
                        d.side == post.side
                            && d.alive()
                            && post.covers_jammed(d.pos, self.link_quality_at(post.pos, post.side))
                    })
            }
        }
    }

    /// One epoch of fires. Each side allocates all its shooters, then they fire in index
    /// order.
    pub(super) fn resolve_fires(&mut self) {
        // Reused across epochs so a battle does not allocate per epoch.
        let mut near_misses = std::mem::take(&mut self.near_misses);
        near_misses.clear();
        near_misses.resize(self.units.len(), 0);

        // Both sides allocate against the same board, before anyone shoots — so neither
        // side gets to react to casualties the other has not taken yet. Sorted by shooter
        // so rounds still resolve in unit index order, which is the determinism unit.
        let mut orders: Vec<(usize, FireTarget)> = [Side::Blue, Side::Red]
            .into_iter()
            .flat_map(|side| self.allocate_side(side))
            .collect();
        orders.sort_unstable();

        // Record the locks (§13.4). Written for *every* shooter, so one that was allocated
        // nothing has its lock cleared rather than keeping a stale one — an idle gun is not
        // still engaging what it shot at two epochs ago.
        for u in &mut self.units {
            u.engaging = None;
        }
        for &(s_idx, target) in &orders {
            self.units[s_idx].engaging = Some(target);
        }

        for (s_idx, target) in orders {
            let shooter = &self.units[s_idx];
            // The allocation was made against the board as it stood at the start of the
            // epoch; a shooter killed by an earlier shooter this epoch does not fire.
            if !shooter.alive() {
                continue;
            }
            let effectiveness = shooter
                .suppression
                .fire_effectiveness(self.suppressed_fire_factor);
            let Some(weapon) = shooter.weapon.clone() else {
                continue;
            };
            let (shooter_pos, elements) = (shooter.pos, shooter.elements);

            let shot = self.prepare_shot(&weapon, shooter_pos, target, effectiveness);
            // Every live element fires the weapon's per-element round count.
            let per_element = (weapon.rof_rounds_per_min * self.epoch_s / 60.0).round() as u32;
            let rounds = per_element.saturating_mul(elements);
            let (target_pos, mut remaining) = {
                let t = self.target_state(target);
                (t.pos, t.elements)
            };

            let mut casualties = 0u32;
            for _ in 0..rounds {
                if remaining == 0 {
                    break;
                }
                let (killed, near) = self.fire_one_round(&shot, target_pos, remaining);
                remaining -= killed;
                casualties += killed;
                // Only *units* have a suppression state to be shaken. A battery or a post
                // is either intact or not (§4.3 is a model of people under fire, and its
                // Free/Suppressed/Pinned chain gates movement and outgoing fire, neither
                // of which an emplacement does).
                if let FireTarget::Unit(i) = target {
                    near_misses[i] += near;
                }
            }

            if casualties > 0 {
                let before = self.target_state(target).elements;
                self.apply_casualties(target, casualties);
                self.fire_events.push(FireEvent {
                    time_s: self.time_s,
                    shooter: s_idx,
                    target,
                    casualties,
                    killed: before > 0 && self.target_state(target).elements == 0,
                });
            }
        }

        // Apply near-miss suppression after all firing (fixed order).
        for (u_idx, &count) in near_misses.iter().enumerate() {
            for _ in 0..count {
                if self.rng.random::<f32>() < self.p_suppress {
                    self.units[u_idx].suppression = self.units[u_idx].suppression.step_up();
                }
            }
        }

        self.near_misses = near_misses; // hand the buffer back
    }

    /// Everything about the burst that the dice do not change (§2.2, §2.3).
    fn prepare_shot(
        &self,
        weapon: &WeaponType,
        shooter_pos: Vec2,
        t: FireTarget,
        effectiveness: f32,
    ) -> Shot {
        let target = self.target_state(t);
        let cover = self.cover_at(target.pos);
        match weapon.class {
            WeaponClass::Direct => {
                // The round flies the slant distance, so dispersion scales with that.
                let range = los::slant_range(
                    &self.terrain,
                    shooter_pos,
                    SHOOTER_HEIGHT_M,
                    target.pos,
                    target.height_m,
                );
                let p_hit = fires::direct_p_hit(
                    weapon.dispersion_mrad,
                    range,
                    target.silhouette_width_m,
                    target.height_m,
                );
                Shot::Direct {
                    p_kill: p_hit * weapon.p_kill_given_hit * (1.0 - cover) * effectiveness,
                }
            }
            WeaponClass::Indirect => Shot::Indirect {
                sigma_m: fires::sigma_from_cep(weapon.cep_m),
                lethal_radius_m: weapon.lethal_radius_m,
                cover,
                effectiveness,
            },
        }
    }

    /// Resolve one round: returns `(elements_killed, near_miss)`. `remaining` is the
    /// target's live element count before this round — indirect fire rolls each one.
    fn fire_one_round(&mut self, shot: &Shot, target_pos: Vec2, remaining: u32) -> (u32, u32) {
        match *shot {
            Shot::Direct { p_kill } => {
                if self.rng.random::<f32>() < p_kill {
                    (1, 0) // element destroyed
                } else {
                    (0, 1) // round passed close — a near-miss
                }
            }
            Shot::Indirect {
                sigma_m,
                lethal_radius_m,
                cover,
                effectiveness,
            } => {
                let burst = fires::sample_burst(target_pos, sigma_m, &mut self.rng);
                let miss = burst.distance(target_pos);
                // Same multiplication order as before the hoist — see `Shot::Indirect`.
                let dmg =
                    fires::carleton_damage(miss, lethal_radius_m) * (1.0 - cover) * effectiveness;
                // Each remaining element independently survives or not.
                let mut killed = 0u32;
                for _ in 0..remaining {
                    if self.rng.random::<f32>() < dmg {
                        killed += 1;
                    }
                }
                (killed, u32::from(miss < self.suppression_radius_m))
            }
        }
    }

    /// Assign every one of `side`'s shooters, splitting them by whether they are in the
    /// side's fire-control net (`docs/DESIGN.md` §11.3).
    ///
    /// With `[sim] fires_need_c2` off — the default — this is one call with the whole
    /// side, exactly as before, and the C2 lists are never consulted. With it on, the side
    /// solves **two** problems: the netted shooters coordinate, and the rest each pick for
    /// themselves. They are solved separately rather than as one problem with constraints,
    /// because "not in the net" means precisely "does not know what anyone else is doing" —
    /// an unnetted shooter must not be allowed to avoid a target because a netted one took
    /// it.
    ///
    /// Deterministic, and draws no randomness.
    fn allocate_side(&self, side: Side) -> Vec<(usize, FireTarget)> {
        let live_shooter = |i: usize| {
            let u = &self.units[i];
            u.side == side
                && u.alive()
                && u.weapon.is_some()
                // Pinned units emit nothing (V31), so they are not in the problem.
                && u.suppression
                    .fire_effectiveness(self.suppressed_fire_factor)
                    > 0.0
        };
        let mut all: Vec<usize> = (0..self.units.len()).filter(|&i| live_shooter(i)).collect();

        // Three ways a shooter leaves the problem before it is solved, in this order:
        //
        // 1. it is **ordered** to engage something reachable (§13.3);
        // 2. it is already **locked** onto a target it can still engage (§13.4);
        // 3. otherwise it is allocated.
        //
        // Orders outrank locks so a new order re-tasks a gun that is mid-engagement —
        // an order is the one thing that should break a lock.
        let mut assigned = self.ordered_engagements(side, &all);
        all.retain(|s| !assigned.iter().any(|(a, _)| a == s));

        for &s in &all {
            if let Some(t) = self.units[s].engaging {
                if self.can_engage(s, t) {
                    assigned.push((s, t));
                }
            }
        }
        all.retain(|s| !assigned.iter().any(|(a, _)| a == s));

        // Locked and ordered shooters still occupy slots on their targets, or the overkill
        // cap would be silently bypassed by anything holding a lock (V56).
        let taken = self.slots_taken(side, &assigned);

        if !self.fires_need_c2 {
            assigned.extend(self.allocate_by_doctrine(side, &all, self.allocation.into(), &taken));
            return assigned;
        }
        let (netted, alone): (Vec<usize>, Vec<usize>) =
            all.into_iter().partition(|&i| self.under_c2(i));
        assigned.extend(self.allocate_by_doctrine(side, &netted, self.allocation.into(), &taken));
        assigned.extend(self.allocate_by_doctrine(side, &alone, Solver::Independent, &taken));
        assigned
    }

    /// How many shooters are already committed to each engageable target, parallel to
    /// [`Sim::engageable_targets`].
    ///
    /// A lock is a shooter on a target just as much as a fresh assignment is, so it counts
    /// against that target's discount. Without this the sequence would restart for every
    /// shooter that happened to be re-deciding, and a target already covered by three
    /// locked guns would look as attractive to a fourth as an untouched one.
    fn slots_taken(&self, side: Side, assigned: &[(usize, FireTarget)]) -> Vec<u32> {
        let targets = self.engageable_targets(side);
        targets
            .iter()
            .map(|t| assigned.iter().filter(|(_, a)| a == t).count() as u32)
            .collect()
    }

    /// Engagements this side has ordered outright (`docs/DESIGN.md` §13.3).
    ///
    /// An order stands only while the pairing is **actually engageable** — alive, in range,
    /// and in line of sight if the weapon needs it. When it is not, the order lapses for
    /// that epoch and the shooter rejoins the assignment; it resumes the moment the target
    /// is reachable again.
    ///
    /// That is the same rule doctrine follows, and deliberately so. An unreachable pairing
    /// is *blocked*, never merely preferred, so no shooter can be left idle facing a target
    /// it cannot touch while something it could engage goes unengaged. A gate wanting a
    /// hard-forced pairing sets up a reachable one, so nothing is lost by making the two
    /// mechanisms agree.
    fn ordered_engagements(&self, side: Side, shooters: &[usize]) -> Vec<(usize, FireTarget)> {
        if self.orders[side as usize].is_empty() {
            return Vec::new();
        }
        let mut out: Vec<(usize, FireTarget)> = Vec::new();
        for order in &self.orders[side as usize] {
            let Some(&s) = shooters
                .iter()
                .find(|&&i| self.units[i].id == order.shooter)
            else {
                continue;
            };
            // Already ordered elsewhere: the first order for a shooter wins, so a scenario
            // listing two for the same gun gets the one it wrote first, not the last.
            if out.iter().any(|(a, _)| *a == s) {
                continue;
            }
            let Some(t) = self.named_fire_target(&order.target) else {
                continue;
            };
            if self.can_engage(s, t) {
                out.push((s, t));
            }
        }
        out
    }

    /// Can this shooter engage this target *right now*?
    ///
    /// The one eligibility test, shared by orders, target locks and the assignment payoff,
    /// so all three agree about what "reachable" means. Wraps [`Sim::kill_fraction`],
    /// which already folds in every gate: alive, in range, in line of sight for direct
    /// fire, and holding a live track for indirect (§2, §10.1, §12.4).
    fn can_engage(&self, shooter: usize, t: FireTarget) -> bool {
        self.target_state(t).elements > 0 && self.kill_fraction(shooter, t).is_some_and(|q| q > 0.0)
    }

    /// Resolve an id to a ground-fire target, searching units, batteries then posts — the
    /// same one namespace `TargetSpec::Named` uses for strike targets (§12.1).
    fn named_fire_target(&self, id: &str) -> Option<FireTarget> {
        (0..self.units.len())
            .find(|&i| self.units[i].id == id)
            .map(FireTarget::Unit)
            .or_else(|| {
                (0..self.air_defence.len())
                    .find(|&i| self.air_defence[i].id == id)
                    .map(FireTarget::AirDefence)
            })
            .or_else(|| {
                (0..self.c2.len())
                    .find(|&i| self.c2[i].id == id)
                    .map(FireTarget::C2)
            })
    }

    /// Allocate `shooters` under this side's doctrine (`docs/DESIGN.md` §13.2).
    ///
    /// - **No doctrine**: one call, exactly the pre-doctrine behaviour (§7.4).
    /// - **Weighted**: one call, with each target's value scaled by its tier — the payoff
    ///   still decides, doctrine is a thumb on the scale.
    /// - **Strict**: the assignment is solved **one tier at a time**, highest first. Any
    ///   shooter that can reach a tier takes something in it; only those left unassigned
    ///   fall through to the next.
    ///
    /// Solving tier by tier is what makes strict ordering *exact*. The obvious alternative
    /// — a large bonus added to a higher tier's payoff — is the trap `allocation::INELIGIBLE`
    /// already fell into once: at the magnitudes needed to dominate, `1e18 + 10.0 == 1e18`
    /// in f64 and the payoff differences inside a tier vanish. A sequence of small exact
    /// problems has no such failure mode.
    fn allocate_by_doctrine(
        &self,
        side: Side,
        shooters: &[usize],
        solver: Solver,
        taken: &[u32],
    ) -> Vec<(usize, FireTarget)> {
        let targets = self.engageable_targets(side);
        let doc = self.doctrine_of(side);
        let tiers: Vec<usize> = targets
            .iter()
            .map(|&t| doc.tier_of(&self.target_names(t)))
            .collect();

        if doc.mode == DoctrineMode::Weighted {
            let weights: Vec<f32> = tiers.iter().map(|&k| doc.weight_for_tier(k)).collect();
            return self.allocate_fires(shooters, &targets, &weights, taken, solver);
        }

        let mut remaining: Vec<usize> = shooters.to_vec();
        let mut out = Vec::new();
        for tier in 0..doc.tier_count() {
            if remaining.is_empty() {
                break;
            }
            let (in_tier, tier_taken): (Vec<FireTarget>, Vec<u32>) = targets
                .iter()
                .zip(&tiers)
                .zip(taken)
                .filter(|((_, &k), _)| k == tier)
                .map(|((&t, _), &n)| (t, n))
                .unzip();
            if in_tier.is_empty() {
                continue;
            }
            let weights = vec![1.0f32; in_tier.len()];
            let chosen = self.allocate_fires(&remaining, &in_tier, &weights, &tier_taken, solver);
            remaining.retain(|s| !chosen.iter().any(|(a, _)| a == s));
            out.extend(chosen);
        }
        out
    }

    /// Is this unit inside a live friendly C2 post's (jammed) coordination radius?
    /// The same test air defence uses, so "in the net" means one thing (§11.1).
    fn under_c2(&self, unit_idx: usize) -> bool {
        let u = &self.units[unit_idx];
        self.c2.iter().any(|post| {
            post.side == u.side
                && post.covers_jammed(u.pos, self.link_quality_at(post.pos, post.side))
        })
    }

    /// Assign `shooters` to enemies of `side`, as `(shooter, target)` pairs.
    /// `docs/DESIGN.md` §10.2.
    ///
    /// The old rule was "each shooter takes the nearest enemy it can engage", decided
    /// independently. That wastes fire in the obvious way: three tanks all engage the one
    /// nearest target while a second, equally dangerous one is left alone. Here the group
    /// solves for all its shooters at once.
    fn allocate_fires(
        &self,
        shooters: &[usize],
        targets: &[FireTarget],
        doctrine_weights: &[f32],
        taken: &[u32],
        solver: Solver,
    ) -> Vec<(usize, FireTarget)> {
        if shooters.is_empty() || targets.is_empty() {
            return Vec::new();
        }

        // What each shooter would do to each target, as a fraction of the target
        // destroyed this epoch.
        let kill_fraction: Vec<Vec<Option<f64>>> = shooters
            .iter()
            .map(|&s| targets.iter().map(|&t| self.kill_fraction(s, t)).collect())
            .collect();

        let value_scale = self.threat_scale();
        // Every target offers a slot to every free shooter, and slot k is worth less than
        // slot k-1 (see `slot_weight`). There is deliberately **no hard cap**.
        //
        // There used to be one, `max_shooters_per_target`, and it was a hard cap that
        // *idled* shooters: a target offered `min(elements, cap)` slots, so once targets
        // were scarcer than shooters — which for indirect fire is most of the opening,
        // since a target must be tracked before it can be shot at — the surplus shooters
        // were assigned nothing and fired nothing. Measured on `fires_c2.toml`, that made
        // a side which had been *split in two* by `fires_need_c2` fight better than a
        // coordinated one, because the cap was applied once per fire-control problem and a
        // split side therefore got it twice (§11.4).
        //
        // The geometric discount below already prices piling on. Truncating it as well
        // said "rather than overkill, do nothing", which is the wrong trade whenever there
        // is nothing else to shoot. Offering one slot per free shooter means the marginal
        // shooter always has somewhere to go, at a value the discount has already decided
        // is small.
        let mut slot_target = Vec::new();
        let mut slot_weight = Vec::new();
        for (t_pos, &t) in targets.iter().enumerate() {
            // Slots held by locked or ordered shooters are not offered again
            // (`slots_taken`); they only shift where this target's discount sequence
            // starts, so the (k+1)-th shooter is discounted for the k already committed.
            let count = shooters.len() as u32;
            // A representative kill probability for this target, over the shooters that
            // can actually engage it. Exact when they are identical, which is the case
            // the geometric discount below is derived for.
            let engaging: Vec<f64> = kill_fraction.iter().filter_map(|row| row[t_pos]).collect();
            let q_bar = if engaging.is_empty() {
                0.0
            } else {
                engaging.iter().sum::<f64>() / engaging.len() as f64
            };
            let value = f64::from(self.target_value(t, value_scale) * doctrine_weights[t_pos]);
            // Slot k is the (k+1)-th shooter *including* those already committed, so the
            // discount continues the sequence rather than restarting it.
            for k in taken[t_pos]..taken[t_pos] + count {
                slot_target.push(t_pos);
                // Geometric discount: the (k+1)-th shooter only helps if the k before it
                // all failed, which happens with probability (1 - q)^k. This is the
                // standard weapon-target-assignment decomposition, and it is exact when
                // the shooters on a target are alike.
                slot_weight.push(value * (1.0 - q_bar).powi(k as i32));
            }
        }

        let payoff: Vec<Vec<f64>> = kill_fraction
            .iter()
            .map(|row| {
                slot_target
                    .iter()
                    .zip(&slot_weight)
                    .map(|(&t_pos, &weight)| match row[t_pos] {
                        Some(q) if q > 0.0 => q * weight,
                        _ => allocation::ineligible(),
                    })
                    .collect()
            })
            .collect();

        allocation::solve(&payoff, solver)
            .into_iter()
            .enumerate()
            .filter_map(|(i, slot)| slot.map(|j| (shooters[i], targets[slot_target[j]])))
            .collect()
    }

    /// The fraction of target `t` that shooter `s` expects to destroy this epoch, or
    /// `None` if it cannot engage at all.
    ///
    /// Direct fire needs LOS and range; indirect fire needs a live track and range but no
    /// LOS. Range is checked before LOS deliberately: the range test is a couple of
    /// terrain samples, an LOS traversal walks the grid.
    fn kill_fraction(&self, s: usize, t: FireTarget) -> Option<f64> {
        let shooter = &self.units[s];
        let target = self.target_state(t);
        let weapon = shooter.weapon.as_ref()?;

        if weapon.class == WeaponClass::Indirect && !target.located {
            return None;
        }
        // Slant range (docs/DESIGN.md §9.1) — the one range convention.
        let range = los::slant_range(
            &self.terrain,
            shooter.pos,
            SHOOTER_HEIGHT_M,
            target.pos,
            target.height_m,
        );
        if range > weapon.max_range_m || range < weapon.min_range_m {
            return None;
        }

        let effectiveness = shooter
            .suppression
            .fire_effectiveness(self.suppressed_fire_factor);
        let cover = self.cover_at(target.pos);
        let rounds = f64::from(self.rounds_this_epoch(weapon, shooter.elements));
        let elements = f64::from(target.elements.max(1));

        let fraction = match weapon.class {
            WeaponClass::Direct => {
                if !los::visible(
                    &self.terrain,
                    shooter.pos,
                    SHOOTER_HEIGHT_M,
                    target.pos,
                    target.height_m,
                ) {
                    return None;
                }
                let p_hit = fires::direct_p_hit(
                    weapon.dispersion_mrad,
                    range,
                    target.silhouette_width_m,
                    target.height_m,
                );
                // Each round removes at most one element.
                let p_kill =
                    f64::from(p_hit * weapon.p_kill_given_hit * (1.0 - cover) * effectiveness);
                rounds * p_kill / elements
            }
            WeaponClass::Indirect => {
                // Each round rolls every surviving element, so the expected fraction
                // removed per round is just the expected damage.
                let sigma = fires::sigma_from_cep(weapon.cep_m);
                let per_round = fires::expected_area_damage(0.0, sigma, weapon.lethal_radius_m)
                    * (1.0 - cover)
                    * effectiveness;
                rounds * f64::from(per_round)
            }
        };
        Some(fraction.clamp(0.0, 1.0))
    }

    /// Rounds one unit fires in an epoch: the weapon's rate, times its live elements.
    fn rounds_this_epoch(&self, weapon: &WeaponType, elements: u32) -> u32 {
        let per_element = (weapon.rof_rounds_per_min * self.epoch_s / 60.0).round() as u32;
        per_element.saturating_mul(elements)
    }

    /// The largest raw threat score on the field, used to normalise [`Sim::target_value`].
    /// Zero when nobody is armed, in which case value is size alone.
    fn threat_scale(&self) -> f32 {
        self.units
            .iter()
            .filter(|u| u.alive())
            .map(Self::raw_threat)
            .fold(0.0f32, f32::max)
    }

    /// How dangerous a unit is, before normalisation: rate of fire × lethality × reach.
    /// Unarmed units score zero.
    fn raw_threat(unit: &super::UnitState) -> f32 {
        unit.weapon.as_ref().map_or(0.0, |w| {
            w.rof_rounds_per_min * w.p_kill_given_hit.max(0.01) * w.max_range_m
        })
    }

    /// What destroying target `t` is worth (`docs/DESIGN.md` §10.2).
    ///
    /// The `value` dial on the stat block wins when set. Otherwise it is derived:
    /// `elements × (1 + threat/threat_max)`, so a unit is worth its size, doubled if it is
    /// the most dangerous thing on the field. Deriving from size *and* threat means an
    /// unscored stat block still ranks sensibly — an unarmed truck is worth something, a
    /// full-strength gun battery a great deal more.
    ///
    /// **An emplacement scores no derived threat** (§12.4). A battery's danger is to
    /// aircraft and a post's is to nobody at all, so neither has an output measurable on
    /// the same scale as a unit's `rof × lethality × reach` — and inventing a conversion
    /// would be arithmetic dressed as doctrine. They fall back to `1.0` per element, and a
    /// scenario that wants artillery to prefer the SAM over the tanks says so with `value`.
    /// That is what the dial is for: expressing "kill the radar first" is a judgement, not
    /// a derivation.
    fn target_value(&self, t: FireTarget, threat_scale: f32) -> f32 {
        let target = self.target_state(t);
        let per_element = target.declared_value.unwrap_or_else(|| {
            if threat_scale > 0.0 {
                1.0 + target.threat / threat_scale
            } else {
                1.0
            }
        });
        target.elements as f32 * per_element.max(0.0)
    }

    /// Every enemy asset `side` may shoot at, in a fixed list-then-index order.
    ///
    /// Units first, so a scenario with no air defence and no posts produces exactly the
    /// list it always did and every existing result is untouched (§7.4). Batteries and
    /// posts are appended, which is the same additive posture §12 took for strike targets.
    fn engageable_targets(&self, side: Side) -> Vec<FireTarget> {
        let mut out: Vec<FireTarget> = (0..self.units.len())
            .filter(|&i| self.units[i].side != side && self.units[i].alive())
            .map(FireTarget::Unit)
            .collect();
        out.extend(
            (0..self.air_defence.len())
                .filter(|&i| self.air_defence[i].side != side && self.air_defence[i].alive())
                .map(FireTarget::AirDefence),
        );
        out.extend(
            (0..self.c2.len())
                .filter(|&i| self.c2[i].side != side && self.c2[i].alive())
                .map(FireTarget::C2),
        );
        out
    }

    /// Terrain cover `∈ [0, 1]` at a world position (0 outside the grid).
    pub(super) fn cover_at(&self, pos: Vec2) -> f32 {
        match self.terrain.transform().world_to_cell(pos) {
            Some((ix, iy)) => self.terrain.cover()[[iy, ix]],
            None => 0.0,
        }
    }
}
