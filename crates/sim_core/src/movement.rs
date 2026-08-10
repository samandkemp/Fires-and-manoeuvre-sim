//! Least-risk pathing as dynamic programming. Spec: `docs/DESIGN.md` §5. Gates: V25–V27.
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
    // `total_cmp`, not `==`: `Ord` requires a total order *consistent with* `Eq`, and the
    // two disagree on NaN — `==` says "different", `total_cmp` says "equal". `BinaryHeap`
    // is entitled to rely on that consistency, so the two must be spelled the same way.
    fn eq(&self, other: &Self) -> bool {
        self.cost.total_cmp(&other.cost) == Ordering::Equal
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
    least_cost_path(
        (w, h),
        |from, to| terrain.move_cost(from, to),
        risk,
        start,
        goal,
        risk_weight,
    )
}

/// The same search over **any** 8-connected grid, given a per-edge move cost.
///
/// [`least_risk_path`] is this with the terrain's own `move_cost`. It is separated so the
/// in-loop planner (§10.5) can run the *identical* algorithm over its coarse decision grid
/// rather than a second implementation that would need its own gates: V25, V26 and V27
/// constrain this function, and reach the coarse planner through it.
///
/// `move_cost` must return `f32::INFINITY` for an impassable edge and a non-negative,
/// finite cost otherwise. Dijkstra is only the dynamic-programming solution while every
/// edge weight is non-negative; a negative one would make a settled cell reachable more
/// cheaply later and the answer would be quietly wrong rather than obviously so.
///
/// # Panics
/// If `risk`'s shape does not match `(w, h)`, or an endpoint is out of bounds.
#[must_use]
pub fn least_cost_path(
    (w, h): (usize, usize),
    move_cost: impl Fn((usize, usize), (usize, usize)) -> f32,
    risk: &Array2<f32>,
    start: (usize, usize),
    goal: (usize, usize),
    risk_weight: f32,
) -> Option<Path> {
    assert_eq!(risk.dim(), (h, w), "risk raster must match the grid shape");
    assert!(
        start.0 < w && start.1 < h && goal.0 < w && goal.1 < h,
        "endpoints in bounds"
    );
    debug_assert!(
        risk_weight >= 0.0 && risk.iter().all(|r| *r >= 0.0),
        "risk and risk_weight must be non-negative: Dijkstra requires non-negative edges"
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
            let step = move_cost(cell, to);
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
