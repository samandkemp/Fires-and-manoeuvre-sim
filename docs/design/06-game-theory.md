[Index](README.md) · [← §5 Movement as DP](05-movement-as-dp.md) · [§7 The simulation loop →](07-the-simulation-loop.md)

---

## 6. Game-theoretic layer

Decisions (user, 2026-07-29): the first game is the **combined detect-then-engage
interdiction game**; solved by **fictitious play** (no LP dependency); strategies are
**Blue position vs Red route**. This is the capstone - it exercises terrain, LOS,
sensing, movement, fires, and suppression through one solved zero-sum game.

### 6.1 Movement in the sim (prerequisite)

Units gain a route and a speed. `UnitType.speed_m_s` (default 0 = static);
`UnitState` carries `route: Vec<Vec2>` + `route_idx`. Each tick, a live **unpinned**
unit advances `speed·dt` metres along its polyline (consuming multiple segments if a
tick's travel spans them); a **Pinned** unit does not move (suppression gating movement).
Reaching the last waypoint halts it. Detection and fires use the updated positions.
*Gates:* a unit on a straight route is at distance `speed·t` after `t` s; a pinned unit
does not advance.

### 6.2 The zero-sum solver - fictitious play

For a payoff matrix `A` (row = Blue/maximiser, col = Red/minimiser), fictitious play
alternates best responses to the opponent's empirical play:

```
each round:
  i* = argmaxᵢ Σⱼ A[i][j]·col_counts[j]     (Blue best-responds)
  j* = argminⱼ Σᵢ A[i][j]·row_counts[i]     (Red best-responds)
  row_counts[i*] += 1;  col_counts[j*] += 1
```

Time-average strategies `row_counts/T`, `col_counts/T` converge to a Nash equilibrium
and the value converges (Robinson 1951, for zero-sum). The value is bracketed by
`v_low = minⱼ (x·A[:,j])` (Blue's guarantee) and `v_high = maxᵢ (A[i,:]·y)` (Red's
guarantee); the gap `v_high − v_low → 0` measures convergence. Pure algorithm, no
dependency, and the convergence is itself an OR demonstration.

### 6.3 The interdiction payoff

- **Blue strategy:** a position `b` holding a sensor + a co-located **observed indirect**
  shooter (mortar). Detection gates its fire (indirect ⇒ needs a detection).
- **Red strategy:** a route `r` (candidate paths across the map - some from the
  least-risk pather at varying caution, some direct).
- **Payoff `A[b][r]` = expected Red attrition** (fraction of Red elements lost) when Red
  traverses `r` while Blue at `b` watches and bombards - estimated by a short headless
  Monte-Carlo battle averaged over seeds. Blue maximises attrition, Red minimises.
  Zero-sum in the attrition metric.

The matrix is built once (MC estimate per cell), then fictitious play solves it - so the
solver stays cheap even though payoff construction is the expensive part. Kept tractable
with small strategy sets (~6-8 each) and short battles, all in the Bevy-free
`experiments` crate; profile before growing.

### 6.4 Validation gates (V32-V39)

| # | Property | Reference |
|---|----------|-----------|
| V32 | matching pennies | FP value → 0, both strategies → (½, ½) |
| V33 | rock-paper-scissors | FP value → 0, both strategies → uniform |
| V34 | saddle point | a game with a pure equilibrium → that value, deterministic strategies |
| V35 | strict dominance | a strictly dominated strategy converges to ~0 weight |
| V36 | skew-symmetric | `A = −Aᵀ` ⇒ value 0 (fair game); value bracket closes |
| V37 | route-following | a unit on a straight route is at `speed·t` after `t` s (± one tick) |
| V38 | pinned halts | a Pinned unit does not advance along its route |
| V39 | interdiction sanity | a route outside every Blue position's view is "safe" (Red weights it, value falls); a Blue spot covering all routes raises the value |
