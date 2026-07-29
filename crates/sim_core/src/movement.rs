//! Movement as dynamic programming: least-risk pathing over the terrain grid.
//! Specified in `docs/DESIGN.md` §5; validated by V25–V27.
//!
//! The value function is a shortest path over the 8-connected cell graph with edge cost
//! `move_cost(from,to) + risk_weight·risk(to)`, so Dijkstra *is* the DP solution.

use crate::terrain::TerrainGrid;
use ndarray::Array2;
use std::cmp::Ordering;
use std::collections::BinaryHeap;

/// The 8 grid-neighbour offsets.
const NEIGHBOURS: [(isize, isize); 8] = [
    (1, 0),
    (-1, 0),
    (0, 1),
    (0, -1),
    (1, 1),
    (1, -1),
    (-1, 1),
    (-1, -1),
];

/// A frontier entry, ordered so `BinaryHeap` (a max-heap) pops the *lowest* cost.
struct Frontier {
    cost: f32,
    cell: (usize, usize),
}
impl PartialEq for Frontier {
    fn eq(&self, other: &Self) -> bool {
        self.cost == other.cost
    }
}
impl Eq for Frontier {}
impl Ord for Frontier {
    fn cmp(&self, other: &Self) -> Ordering {
        other.cost.total_cmp(&self.cost) // reversed → min-heap
    }
}
impl PartialOrd for Frontier {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// A computed least-risk route.
#[derive(Clone, Debug, PartialEq)]
pub struct Path {
    /// Cells from start to goal inclusive.
    pub cells: Vec<(usize, usize)>,
    /// Total path cost (mobility + weighted risk).
    pub cost: f32,
}

/// Least-cost path from `start` to `goal` over the 8-connected grid, trading mobility
/// against `risk_weight · risk`. `None` if the goal is unreachable (walled off by
/// impassable terrain). `docs/DESIGN.md` §5.1.
///
/// # Panics
/// If `risk`'s shape does not match the terrain, or an endpoint is out of bounds.
#[must_use]
pub fn least_risk_path(
    terrain: &TerrainGrid,
    risk: &Array2<f32>,
    start: (usize, usize),
    goal: (usize, usize),
    risk_weight: f32,
) -> Option<Path> {
    let (w, h) = (terrain.width(), terrain.height());
    assert_eq!(risk.dim(), (h, w), "risk raster must match terrain shape");
    assert!(
        start.0 < w && start.1 < h && goal.0 < w && goal.1 < h,
        "endpoints in bounds"
    );

    let mut dist: Array2<f32> = Array2::from_elem((h, w), f32::INFINITY);
    let mut prev: Array2<Option<(usize, usize)>> = Array2::from_elem((h, w), None);
    let mut heap = BinaryHeap::new();

    dist[[start.1, start.0]] = 0.0;
    heap.push(Frontier {
        cost: 0.0,
        cell: start,
    });

    while let Some(Frontier { cost, cell }) = heap.pop() {
        if cell == goal {
            break;
        }
        if cost > dist[[cell.1, cell.0]] {
            continue; // a stale, superseded frontier entry
        }
        for (dx, dy) in NEIGHBOURS {
            let nx = cell.0 as isize + dx;
            let ny = cell.1 as isize + dy;
            if nx < 0 || ny < 0 || nx >= w as isize || ny >= h as isize {
                continue;
            }
            let to = (nx as usize, ny as usize);
            let step = terrain.move_cost(cell, to);
            if !step.is_finite() {
                continue;
            }
            let edge = step + risk_weight * risk[[to.1, to.0]];
            let nd = cost + edge;
            if nd < dist[[to.1, to.0]] {
                dist[[to.1, to.0]] = nd;
                prev[[to.1, to.0]] = Some(cell);
                heap.push(Frontier { cost: nd, cell: to });
            }
        }
    }

    let goal_cost = dist[[goal.1, goal.0]];
    if !goal_cost.is_finite() {
        return None;
    }

    // Walk predecessors back from the goal.
    let mut cells = vec![goal];
    let mut cur = goal;
    while let Some(p) = prev[[cur.1, cur.0]] {
        cells.push(p);
        cur = p;
    }
    cells.reverse();
    Some(Path {
        cells,
        cost: goal_cost,
    })
}

/// Total risk exposure `Σ risk(cell)` accumulated along a path (excluding the start) —
/// the quantity `risk_weight` trades against, used to check risk-avoidance behaviour.
#[must_use]
pub fn path_risk(path: &Path, risk: &Array2<f32>) -> f32 {
    path.cells.iter().skip(1).map(|&(x, y)| risk[[y, x]]).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terrain::{TerrainGrid, TerrainParams, TerrainParamsTable, TerrainType};

    fn open_params() -> TerrainParamsTable {
        let open = TerrainParams {
            feature_height_m: 0.0,
            extinction_per_m: 0.0,
            cover: 0.0,
            concealment: 0.0,
            mobility_cost: 1.0,
        };
        TerrainParamsTable {
            open,
            trees: open,
            urban: open,
        }
    }

    /// Flat, uniform-mobility terrain (so move_cost = cell size on straight edges).
    fn flat(w: usize, h: usize) -> TerrainGrid {
        TerrainGrid::from_layers(
            10.0,
            Array2::zeros((h, w)),
            Array2::from_elem((h, w), TerrainType::Open),
            &open_params(),
        )
    }

    // V25: with no risk, the cost is the closed-form 8-connected distance.
    #[test]
    fn v25_zero_risk_is_shortest_path() {
        let g = flat(20, 20);
        let risk = Array2::zeros((20, 20));
        let path = least_risk_path(&g, &risk, (2, 3), (14, 9), 0.0).expect("reachable");

        // (dx, dy) = (12, 6): 6 diagonal steps + 6 straight, on flat ground z=0 so the
        // slope factor is 1 and mobility is 1 → cost = (6·√2 + 6)·cell_size.
        let (dx, dy) = (12.0f32, 6.0f32);
        let expected = ((dx - dy) + dy * std::f32::consts::SQRT_2) * 10.0;
        assert!(
            (path.cost - expected).abs() < 1e-2,
            "zero-risk cost {} should equal 8-connected distance {expected}",
            path.cost
        );
        assert_eq!(path.cells.first(), Some(&(2, 3)));
        assert_eq!(path.cells.last(), Some(&(14, 9)));
    }

    // V26: a high-risk wall gets routed around once caution is high enough, and total
    // risk exposure is monotone non-increasing in risk_weight.
    #[test]
    fn v26_risk_avoidance_monotone() {
        let g = flat(21, 21);
        // A vertical risk wall at column 10 spanning most rows, with a gap at the top.
        let mut risk = Array2::zeros((21, 21));
        for iy in 0..18 {
            risk[[iy, 10]] = 1.0;
        }
        let (start, goal) = ((3, 9), (17, 9));

        let mut last_exposure = f32::INFINITY;
        let mut routed_around = false;
        for &wgt in &[0.0f32, 20.0, 100.0, 500.0] {
            let p = least_risk_path(&g, &risk, start, goal, wgt).unwrap();
            let exposure = path_risk(&p, &risk);
            assert!(
                exposure <= last_exposure + 1e-4,
                "risk exposure must not rise with caution (w={wgt}: {exposure} vs {last_exposure})"
            );
            last_exposure = exposure;
            if exposure == 0.0 {
                routed_around = true;
            }
        }
        assert!(
            routed_around,
            "high caution should find the zero-risk detour"
        );
    }

    // V27: matches an independent Bellman-Ford relaxation on a small random-ish grid.
    #[test]
    fn v27_matches_bellman_ford() {
        let g = flat(8, 8);
        let mut risk = Array2::zeros((8, 8));
        for iy in 0..8 {
            for ix in 0..8 {
                risk[[iy, ix]] = ((ix * 7 + iy * 3) % 5) as f32 / 4.0;
            }
        }
        let start = (0, 0);
        let dijkstra = least_risk_path(&g, &risk, start, (7, 7), 30.0).unwrap();
        let reference = bellman_ford_cost(&g, &risk, start, 30.0);
        assert!(
            (dijkstra.cost - reference[[7, 7]]).abs() < 1e-2,
            "Dijkstra {} vs Bellman-Ford {}",
            dijkstra.cost,
            reference[[7, 7]]
        );
        // Path is contiguous 8-neighbour steps.
        for pair in dijkstra.cells.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            assert!(a.0.abs_diff(b.0) <= 1 && a.1.abs_diff(b.1) <= 1);
        }
    }

    /// Independent shortest-cost reference by repeated relaxation (Bellman–Ford style).
    fn bellman_ford_cost(
        g: &TerrainGrid,
        risk: &Array2<f32>,
        start: (usize, usize),
        risk_weight: f32,
    ) -> Array2<f32> {
        let (w, h) = (g.width(), g.height());
        let mut dist = Array2::from_elem((h, w), f32::INFINITY);
        dist[[start.1, start.0]] = 0.0;
        for _ in 0..(w * h) {
            let mut changed = false;
            for iy in 0..h {
                for ix in 0..w {
                    if !dist[[iy, ix]].is_finite() {
                        continue;
                    }
                    for (dx, dy) in NEIGHBOURS {
                        let nx = ix as isize + dx;
                        let ny = iy as isize + dy;
                        if nx < 0 || ny < 0 || nx >= w as isize || ny >= h as isize {
                            continue;
                        }
                        let to = (nx as usize, ny as usize);
                        let step = g.move_cost((ix, iy), to);
                        if !step.is_finite() {
                            continue;
                        }
                        let nd = dist[[iy, ix]] + step + risk_weight * risk[[to.1, to.0]];
                        if nd < dist[[to.1, to.0]] {
                            dist[[to.1, to.0]] = nd;
                            changed = true;
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }
        dist
    }
}
