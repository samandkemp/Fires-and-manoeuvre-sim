//! What one run produced.
//!
//! Every field is read back from the sim's own event logs and final state, never
//! accumulated alongside the sim as it runs. There is therefore no second bookkeeping path
//! to drift out of step with the model - if a metric is wrong, the log is wrong, and the
//! app's feed would be showing the same wrong thing.
//!
//! Adding a metric means three edits in this file: a field, a [`COLUMNS`] entry, and a
//! line in [`Outcome::values`]. The last two are tied together by the array length `N`, so
//! forgetting one of them will not compile.

use sim_core::sim::{Side, Sim};

/// How many metrics one run reports. Ties [`COLUMNS`] and [`Outcome::values`] together, so
/// adding a field to one without the other will not compile.
pub const N: usize = 19;

/// Column order, shared by the per-seed file, the summary file and the console, so all
/// three read the same way.
pub const COLUMNS: [&str; N] = [
    "blue_losses",
    "red_losses",
    "blue_units_killed",
    "red_units_killed",
    "detections",
    "first_detection_s",
    "fire_events",
    "air_launched",
    "air_downed",
    "air_leakers",
    "munitions_released",
    "ground_casualties_from_air",
    "ad_shots",
    "ad_rounds_left",
    "ad_batteries_killed",
    "c2_posts_killed",
    "blue_cleared_s",
    "red_cleared_s",
    "epochs",
];

/// One run's result. `f64` throughout because everything here is about to be averaged.
#[derive(Default, Clone, Copy, Debug, PartialEq)]
pub struct Outcome {
    /// Ground sub-elements lost, by side.
    pub blue_losses: f64,
    pub red_losses: f64,
    /// Whole units reduced to zero elements, by side.
    pub blue_units_killed: f64,
    pub red_units_killed: f64,
    /// Detection events logged, both sides and both domains.
    pub detections: f64,
    /// Sim time of the first detection either way; the run length if nothing was seen, so
    /// the column stays comparable instead of mixing in a sentinel.
    pub first_detection_s: f64,
    /// Fires resolutions that produced casualties.
    pub fire_events: f64,
    /// Air: launched, shot down, and those that survived to release a munition.
    pub air_launched: f64,
    pub air_downed: f64,
    pub air_leakers: f64,
    pub munitions_released: f64,
    pub ground_casualties_from_air: f64,
    /// Air-defence shots taken (§9.4). The denominator for "how many rounds per kill".
    pub ad_shots: f64,
    /// Interceptors left across all batteries with a finite magazine. Phase 11 found that
    /// C2 coordination buys **ammunition**, not kills, which is invisible without this.
    pub ad_rounds_left: f64,
    /// Batteries and posts reduced to zero elements (§12) - what SEAD is trying to do.
    pub ad_batteries_killed: f64,
    pub c2_posts_killed: f64,
    /// When a side's last ground element died; the run length if it never did.
    ///
    /// Usually the metric that answers "was this better?", because losses saturate. Once
    /// everything on one side is dead by 600 s in every arm, `red_losses` is the same
    /// number everywhere and only the *time* distinguishes them - which is exactly how
    /// the Phase 10 allocation result was measured (`docs/DESIGN.md` §10.2).
    pub blue_cleared_s: f64,
    pub red_cleared_s: f64,
    /// Decision epochs resolved: the run's length in the units decisions happen on.
    pub epochs: f64,
}

impl Outcome {
    /// The metrics in [`COLUMNS`] order.
    #[must_use]
    pub fn values(&self) -> [f64; N] {
        [
            self.blue_losses,
            self.red_losses,
            self.blue_units_killed,
            self.red_units_killed,
            self.detections,
            self.first_detection_s,
            self.fire_events,
            self.air_launched,
            self.air_downed,
            self.air_leakers,
            self.munitions_released,
            self.ground_casualties_from_air,
            self.ad_shots,
            self.ad_rounds_left,
            self.ad_batteries_killed,
            self.c2_posts_killed,
            self.blue_cleared_s,
            self.red_cleared_s,
            self.epochs,
        ]
    }

    /// Index of a metric by name, for `--metric red_losses` on the command line.
    #[must_use]
    pub fn column(name: &str) -> Option<usize> {
        COLUMNS.iter().position(|c| *c == name)
    }
}

/// Run one battle to `until_s` and read the outcome out of the sim's logs.
///
/// Takes an already-built sim so the caller can reuse its terrain across trials; the
/// caller is responsible for having reset it first.
pub fn run_one(sim: &mut Sim, until_s: f64) -> Outcome {
    let air_launched = sim.air().len() as f64;

    // Stepped rather than `run_until`, only so the clearance times can be observed as they
    // happen - they are not derivable afterwards, because the logs say a unit was killed
    // but an air-delivered burst does not name which one. The loop condition is exactly
    // `run_until`'s, so this advances the sim identically; the per-tick check is a handful
    // of integer comparisons against a tick that costs tens of microseconds.
    // A side that fields no ground units at all is not "cleared at t = 1 s" - there was
    // nothing to clear. `air_raid` is exactly that: Red is three drones and no ground.
    let (mut blue_present, mut red_present) = (false, false);
    for u in sim.units() {
        match u.side {
            Side::Blue => blue_present |= u.initial_elements > 0,
            Side::Red => red_present |= u.initial_elements > 0,
        }
    }

    let (mut blue_cleared_s, mut red_cleared_s) = (None, None);
    while sim.time_s() < until_s {
        sim.step_one();
        let (mut blue_live, mut red_live) = (0_u32, 0_u32);
        for u in sim.units() {
            match u.side {
                Side::Blue => blue_live += u.elements,
                Side::Red => red_live += u.elements,
            }
        }
        // `get_or_insert` only records the *first* time a side hit zero, which is what
        // "cleared" means; nothing can bring elements back, but the guard costs nothing.
        if blue_present && blue_live == 0 {
            blue_cleared_s.get_or_insert(sim.time_s());
        }
        if red_present && red_live == 0 {
            red_cleared_s.get_or_insert(sim.time_s());
        }
    }

    let mut o = Outcome {
        air_launched,
        detections: (sim.events().len() + sim.air_events().len()) as f64,
        fire_events: sim.fire_events().len() as f64,
        epochs: sim.epochs_run() as f64,
        // A side that was never cleared reports the run length, so the column stays a
        // comparable time rather than mixing in a sentinel. Read it with the kill counts
        // beside it: "600" means "not by 600 s", not "at 600 s".
        blue_cleared_s: blue_cleared_s.unwrap_or(until_s),
        red_cleared_s: red_cleared_s.unwrap_or(until_s),
        ..Default::default()
    };

    // Losses come from each unit's own record of what it started with, so this reads the
    // same whether the caller snapshotted anything or not.
    for u in sim.units() {
        let lost = f64::from(u.initial_elements.saturating_sub(u.elements));
        match u.side {
            Side::Blue => {
                o.blue_losses += lost;
                o.blue_units_killed += f64::from(u32::from(!u.alive()));
            }
            Side::Red => {
                o.red_losses += lost;
                o.red_units_killed += f64::from(u32::from(!u.alive()));
            }
        }
    }

    o.air_downed = sim.air().iter().filter(|a| !a.alive).count() as f64;
    o.munitions_released = sim.strike_events().len() as f64;
    o.ground_casualties_from_air = sim
        .strike_events()
        .iter()
        .map(|e| f64::from(e.casualties))
        .sum();
    // "Leaker" = an airframe that survived to release: exactly what the strike log records.
    let mut leakers: Vec<usize> = sim.strike_events().iter().map(|e| e.air).collect();
    leakers.sort_unstable();
    leakers.dedup();
    o.air_leakers = leakers.len() as f64;

    o.ad_shots = sim.air_defence_events().len() as f64;
    // An unlimited magazine is `u32::MAX`; counting it would swamp the column and mean
    // nothing, so only finite magazines contribute.
    o.ad_rounds_left = sim
        .air_defence()
        .iter()
        .filter(|d| d.magazine_left != u32::MAX)
        .map(|d| f64::from(d.magazine_left))
        .sum();
    o.ad_batteries_killed = sim.air_defence().iter().filter(|d| !d.alive()).count() as f64;
    o.c2_posts_killed = sim.c2().iter().filter(|c| !c.alive()).count() as f64;

    let first_ground = sim.events().first().map(|e| e.time_s);
    let first_air = sim.air_events().first().map(|e| e.time_s);
    o.first_detection_s = match (first_ground, first_air) {
        (Some(a), Some(b)) => a.min(b),
        (Some(t), None) | (None, Some(t)) => t,
        (None, None) => until_s,
    };
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The three places a metric is declared must agree. `values()` and `COLUMNS` are tied
    /// by `N` at compile time; this pins that no name was left blank or duplicated.
    #[test]
    fn columns_are_distinct_and_named() {
        let mut seen: Vec<&str> = COLUMNS.to_vec();
        seen.sort_unstable();
        let before = seen.len();
        seen.dedup();
        assert_eq!(seen.len(), before, "duplicate column name in COLUMNS");
        assert!(COLUMNS.iter().all(|c| !c.is_empty()));
        assert_eq!(Outcome::default().values().len(), COLUMNS.len());
        assert_eq!(Outcome::column("red_losses"), Some(1));
        assert_eq!(Outcome::column("not_a_metric"), None);
    }
}
