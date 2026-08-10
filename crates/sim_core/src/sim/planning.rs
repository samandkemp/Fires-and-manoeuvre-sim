//! Movement decisions in the loop: a unit with an objective plans its own route.
//! Spec: `docs/DESIGN.md` §5, §10.5. Gates: V72–V74.
//!
//! Until now movement was the one decision still scripted. Fires are allocated (§10.2) and
//! sensors are tasked (§10.3), but a route was drawn by hand — so `movement::least_risk_path`
//! was called only from `experiments/` and `validation/`, and the dynamic-programming strand
//! sat *beside* the model rather than inside it.
//!
//! A unit with an `objective` re-solves its route each decision epoch against the live risk
//! raster. That is what lets sensing and EW bite on **manoeuvre**: place a sensor across a
//! route and the mover goes round it, which a scripted waypoint list cannot express.
//!
//! # Why the decision grid is coarse
//!
//! Measured: a risk raster at full terrain resolution costs ~4 s for a 1000x1000 map with
//! two sensors, because every cell asks every sensor for a detection rate and each of those
//! walks a sightline. At one decision epoch per 10 s that is a hundred times the cost of the
//! rest of the simulation put together, and a 500-seed study becomes a fortnight.
//!
//! So planning happens on a coarse grid — the same reasoning §10.3 gives for belief, and the
//! same resolution dial. A commander choosing an approach at ten-second intervals is not
//! choosing between adjacent 10 m cells; the unit still *moves* continuously at full
//! resolution, and only the route it is following is planned coarsely.
//!
//! The coarse edge cost is `terrain::move_cost`'s own formula — distance x mean mobility x
//! slope factor — evaluated on cell **aggregates** rather than point samples. At a coarse
//! grid equal to the terrain it is exactly `move_cost`, which is a property worth stating
//! because it says the coarsening is an approximation of the real cost rather than a
//! different cost that happens to look similar.
//!
//! # Determinism
//!
//! Planning draws **no randomness**, and a scenario in which no unit has an objective does
//! no work here at all — the identity is structural rather than dial-gated (V72).

use super::{Side, Sim};
use crate::movement::least_cost_path;
use crate::sensing::{self, UnitType};
use crate::terrain::TerrainGrid;
use glam::Vec2;
use ndarray::Array2;

/// Slope penalties, matching `TerrainGrid::move_cost` so the coarse cost is the same
/// formula on aggregated inputs rather than a second opinion about hills.
const UPHILL_PENALTY: f32 = 4.0;
const DOWNHILL_PENALTY: f32 = 1.5;

/// The mover risk is scored against.
///
/// Risk is "how detectable would *a* unit be here", not "how detectable is this particular
/// unit" — so it is a property of the ground and the enemy's sensors, computed once per side
/// rather than once per unit. A unit that is unusually stealthy is not less endangered by a
/// well-watched valley; it just survives it more often, which the detection model already
/// says at the point it matters. §5.2 defines the raster this way.
fn reference_mover() -> UnitType {
    UnitType {
        height_m: 2.5,
        silhouette_width_m: 3.0,
        element_count: 1,
        signature: std::collections::BTreeMap::from([("optical".to_owned(), 0.6)]),
        ..Default::default()
    }
}

/// The coarse decision grid: terrain aggregates that never change, plus a per-side risk
/// raster that does.
pub(super) struct Planner {
    /// Edge length in coarse cells.
    cells: usize,
    /// World size of one coarse cell, metres.
    cell_size_m: f32,
    /// Mean mobility cost per coarse cell; infinite only where *every* fine cell is
    /// impassable — at 200 m resolution "there is a way through" is the honest reading.
    mobility: Array2<f32>,
    /// Mean elevation per coarse cell, for the slope factor.
    elevation: Array2<f32>,
    /// Enemy observation coverage as seen by each side, `[Blue, Red]`, normalised to [0, 1].
    risk: [Array2<f32>; 2],
    /// The epoch each side's raster was built for; `None` means never.
    built_at: [Option<u64>; 2],
}

impl Planner {
    /// Aggregate the terrain onto a `cells x cells` grid. Done once, when the first unit
    /// with an objective is placed.
    fn new(terrain: &TerrainGrid, cells: usize) -> Self {
        let cells = cells.max(2);
        let (w, h) = (terrain.width(), terrain.height());
        let mut mobility = Array2::<f32>::zeros((cells, cells));
        let mut elevation = Array2::<f32>::zeros((cells, cells));

        for cy in 0..cells {
            for cx in 0..cells {
                let (x0, x1) = (
                    cx * w / cells,
                    ((cx + 1) * w / cells).max(cx * w / cells + 1),
                );
                let (y0, y1) = (
                    cy * h / cells,
                    ((cy + 1) * h / cells).max(cy * h / cells + 1),
                );
                let (mut m_sum, mut z_sum, mut n, mut passable) = (0.0f32, 0.0f32, 0u32, 0u32);
                for fy in y0..y1.min(h) {
                    for fx in x0..x1.min(w) {
                        let m = terrain.mobility_cost()[[fy, fx]];
                        z_sum += terrain.elevation()[[fy, fx]];
                        n += 1;
                        if m.is_finite() {
                            m_sum += m;
                            passable += 1;
                        }
                    }
                }
                let n = n.max(1) as f32;
                elevation[[cy, cx]] = z_sum / n;
                // Impassable only where nothing gets through.
                mobility[[cy, cx]] = if passable == 0 {
                    f32::INFINITY
                } else {
                    m_sum / passable as f32
                };
            }
        }

        let extent = w as f32 * terrain.transform().cell_size_m();
        Self {
            cells,
            cell_size_m: extent / cells as f32,
            mobility,
            elevation,
            risk: [
                Array2::<f32>::zeros((cells, cells)),
                Array2::<f32>::zeros((cells, cells)),
            ],
            built_at: [None; 2],
        }
    }

    /// World centre of a coarse cell.
    fn centre(&self, terrain: &TerrainGrid, cx: usize, cy: usize) -> Vec2 {
        let ex = terrain.width() as f32 * terrain.transform().cell_size_m();
        let ey = terrain.height() as f32 * terrain.transform().cell_size_m();
        Vec2::new(
            (cx as f32 + 0.5) * ex / self.cells as f32,
            (cy as f32 + 0.5) * ey / self.cells as f32,
        )
    }

    /// Which coarse cell a world position falls in, clamped to the grid.
    fn cell_of(&self, terrain: &TerrainGrid, p: Vec2) -> (usize, usize) {
        let ex = terrain.width() as f32 * terrain.transform().cell_size_m();
        let ey = terrain.height() as f32 * terrain.transform().cell_size_m();
        let f = |v: f32, extent: f32| -> usize {
            let frac = (v / extent.max(f32::EPSILON)).clamp(0.0, 1.0);
            ((frac * self.cells as f32) as usize).min(self.cells - 1)
        };
        (f(p.x, ex), f(p.y, ey))
    }

    /// `move_cost`'s formula on cell aggregates: distance x mean mobility x slope factor.
    fn edge_cost(&self, from: (usize, usize), to: (usize, usize)) -> f32 {
        let (m_from, m_to) = (self.mobility[[from.1, from.0]], self.mobility[[to.1, to.0]]);
        if !m_from.is_finite() || !m_to.is_finite() {
            return f32::INFINITY;
        }
        let diagonal = from.0 != to.0 && from.1 != to.1;
        let dist = self.cell_size_m
            * if diagonal {
                std::f32::consts::SQRT_2
            } else {
                1.0
            };
        let dz = self.elevation[[to.1, to.0]] - self.elevation[[from.1, from.0]];
        let grade = dz / dist;
        let slope = 1.0 + UPHILL_PENALTY * grade.max(0.0) + DOWNHILL_PENALTY * (-grade).max(0.0);
        dist * 0.5 * (m_from + m_to) * slope
    }
}

impl Sim {
    /// Re-plan every unit that has an objective. Called at the decision epoch, after tracks
    /// are maintained and before fires are resolved.
    ///
    /// A no-op — and a zero-allocation, zero-draw one — when no unit has an objective, which
    /// is what makes V72 an identity by construction.
    pub(super) fn replan_movement(&mut self) {
        if !self
            .units
            .iter()
            .any(|u| u.objective.is_some() && u.alive())
        {
            return;
        }
        let cells = self.tasking.cells();
        if self.planner.is_none() {
            self.planner = Some(Planner::new(&self.terrain, cells));
        }

        for side in [Side::Blue, Side::Red] {
            if self
                .units
                .iter()
                .any(|u| u.side == side && u.objective.is_some() && u.alive())
            {
                self.refresh_risk(side);
            }
        }

        for idx in 0..self.units.len() {
            let (Some(goal), true) = (self.units[idx].objective, self.units[idx].alive()) else {
                continue;
            };
            self.replan_one(idx, goal);
        }
    }

    /// Rebuild one side's view of enemy observation coverage, once per epoch.
    fn refresh_risk(&mut self, side: Side) {
        let epoch = self.epochs_run;
        let planner = self.planner.as_ref().expect("planner built by caller");
        if planner.built_at[side as usize] == Some(epoch) {
            return;
        }

        // Enemy sensors that are actually emitting — a dead battery's radar and one under
        // EMCON both drop out here for free, because `sensor_active` is the one predicate
        // (§12.5).
        let enemy: Vec<(Vec2, f32, f32, crate::sensing::SensorType)> = (0..self.sensors.len())
            .filter(|&i| self.sensors[i].side != side && self.sensor_active(i))
            .map(|i| {
                let (pos, height, facing) = self.sensor_view(i);
                (pos, height, facing, self.sensors[i].stats.clone())
            })
            .collect();

        let mover = reference_mover();
        let cells = planner.cells;
        let mut risk = Array2::<f32>::zeros((cells, cells));
        if !enemy.is_empty() {
            let centres: Vec<Vec<Vec2>> = (0..cells)
                .map(|cy| {
                    (0..cells)
                        .map(|cx| planner.centre(&self.terrain, cx, cy))
                        .collect()
                })
                .collect();
            for cy in 0..cells {
                for cx in 0..cells {
                    let at = centres[cy][cx];
                    let mut worst = 0.0f32;
                    for (pos, _height, facing, stats) in &enemy {
                        worst = worst.max(sensing::detection_rate(
                            &self.terrain,
                            stats,
                            *pos,
                            *facing,
                            &mover,
                            at,
                        ));
                    }
                    risk[[cy, cx]] = worst;
                }
            }
            // Normalise so `risk_weight` means the same thing whatever the sensors are:
            // an exchange rate against a [0, 1] exposure, not against a raw rate whose
            // scale moves with `lambda0_per_s`.
            let max = risk.iter().copied().fold(0.0f32, f32::max);
            if max > 0.0 {
                risk.mapv_inplace(|v| v / max);
            }
        }

        let planner = self.planner.as_mut().expect("planner built by caller");
        planner.risk[side as usize] = risk;
        planner.built_at[side as usize] = Some(epoch);
    }

    /// Plan one unit, adopting the new route only if it is enough better than the held one.
    fn replan_one(&mut self, idx: usize, goal: Vec2) {
        let planner = self.planner.as_ref().expect("planner built by caller");
        let side = self.units[idx].side;
        let from = planner.cell_of(&self.terrain, self.units[idx].pos);
        let to = planner.cell_of(&self.terrain, goal);
        if from == to {
            // Already in the objective's cell: steer straight at it rather than planning a
            // path of length zero and then having nothing to follow.
            self.units[idx].route = vec![goal];
            self.units[idx].route_idx = 0;
            return;
        }

        let weight = self.units[idx].risk_weight.unwrap_or(self.risk_weight);
        let risk = &planner.risk[side as usize];
        let Some(path) = least_cost_path(
            (planner.cells, planner.cells),
            |a, b| planner.edge_cost(a, b),
            risk,
            from,
            to,
            weight,
        ) else {
            return; // walled off: keep whatever route is held rather than stopping dead
        };

        // Hysteresis. Re-costing the held route on the *current* raster is the honest
        // comparison: a route only looks worse because the risk moved, and that is exactly
        // when switching is justified.
        if !self.units[idx].route.is_empty() {
            let held = self.route_cost(idx, weight);
            if path.cost >= held * (1.0 - self.repath_margin) {
                return;
            }
        }

        let mut route: Vec<Vec2> = path
            .cells
            .iter()
            .skip(1) // the cell the unit is standing in
            .map(|&(cx, cy)| planner.centre(&self.terrain, cx, cy))
            .collect();
        // Finish at the objective itself, not at the centre of its cell.
        route.pop();
        route.push(goal);
        self.units[idx].route = route;
        self.units[idx].route_idx = 0;
    }

    /// What the route a unit is currently following would cost if planned now.
    fn route_cost(&self, idx: usize, weight: f32) -> f32 {
        let planner = self.planner.as_ref().expect("planner built by caller");
        let side = self.units[idx].side;
        let risk = &planner.risk[side as usize];
        let unit = &self.units[idx];
        let mut total = 0.0;
        let mut prev = planner.cell_of(&self.terrain, unit.pos);
        for wp in unit.route.iter().skip(unit.route_idx) {
            let cell = planner.cell_of(&self.terrain, *wp);
            if cell == prev {
                continue;
            }
            // Waypoints are adjacent cells by construction, but a held route may have been
            // set before the unit moved, so step through any gap in a straight line.
            let (mut cx, mut cy) = prev;
            while (cx, cy) != cell {
                let nx = cx + usize::from(cell.0 > cx) - usize::from(cell.0 < cx);
                let ny = cy + usize::from(cell.1 > cy) - usize::from(cell.1 < cy);
                total += planner.edge_cost((cx, cy), (nx, ny)) + weight * risk[[ny, nx]];
                if !total.is_finite() {
                    return f32::INFINITY;
                }
                (cx, cy) = (nx, ny);
            }
            prev = cell;
        }
        total
    }
}
