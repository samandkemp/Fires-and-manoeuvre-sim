[Index](README.md) · [← §4 Suppression & attrition](04-suppression-and-attrition.md) · [§6 Game-theoretic layer →](06-game-theory.md)

---

## 5. Movement as DP

Least-risk pathing over the terrain grid: a mover chooses a route trading **mobility
cost** (time/effort) against **exposure risk**. The value function is exactly a
shortest-path / DP problem, so Dijkstra over the cell graph *is* the DP solution here
(no separate value-iteration needed until risk becomes time-varying in later phases).

### 5.1 Formulation

8-connected grid graph; each edge `from → to` costs

$$
c(\text{from} \to \text{to}) = c_{\text{move}}(\text{from}, \text{to}) + w \cdot \text{risk}(\text{to})
$$

`move_cost` is the terrain edge cost (mean mobility × slope factor × distance; ∞ =
impassable). The slope factor is

$$
s(g) = 1 + 4\max(0, g) + 1.5\max(0, -g), \qquad g = \frac{\Delta z}{\text{horizontal distance}}
$$

so **flat ground is the cheapest case and both gradients cost more** - ascent about 2.7×
harder than descent, not descent for free. That is the intended reading (a steep descent is
slow for a tracked vehicle, not an advantage), but "penalises uphill harder than downhill"
is easy to misread as "downhill is cheaper than flat", so: it is not. The two constants are
still `const` in `terrain.rs` rather than dials in the movement TOML - the one piece of
Phase 5 data-drivenness still owed. `risk(cell)` is a caller-supplied exposure raster in `[0, 1]`; `risk_weight`
(metres of mobility-cost the mover will spend to avoid one unit of risk) tunes caution.
The least-cost path minimises total `Σ edge_cost` - an **additive** cost so the problem
is a clean shortest path. *(Alternative considered: multiplicative survival
`Π(1−p_death)` maximisation - richer but the log turns it additive anyway; additive with
a supplied risk raster is the smallest thing that gives the least-risk behaviour and a
clean Dijkstra gate. Documented in QUESTIONS §E.)*

Solved with Dijkstra (binary heap), skipping infinite (impassable) edges. Returns the
path and its total cost, or `None` if the goal is unreachable.

### 5.2 Risk raster

For the interactive demo, risk is **enemy observation coverage**: for each cell, the
detection rate a reference mover would suffer from the best-placed enemy sensor
(`max` over enemy sensors of `detection_rate` against a reference unit), normalised to
`[0, 1]`. This reuses the sensing model, so "least-risk path" literally means
"route that stays hardest to see" - the see-without-being-seen idea made navigable. The
path solver itself is agnostic: any `[0, 1]` raster works.

### 5.3 Validation gates (V25-V27)

| # | Property | Reference |
|---|----------|-----------|
| V25 | zero-risk = shortest path | with `risk_weight = 0` on uniform-mobility flat terrain, path cost equals the closed-form 8-connected distance `(max−min)+√2·min` scaled by cell mobility |
| V26 | risk avoidance monotone | raising `risk_weight` never increases total risk exposure along the optimum; a high-risk barrier gets routed around once the weight is high enough |
| V27 | optimality | Dijkstra cost matches an independent exhaustive/Bellman-Ford reference on a small grid; path is contiguous, in-bounds, endpoint-correct |
