[Index](README.md) · [§1 Terrain & LOS →](01-terrain-and-los.md)

---

## 0. The strand index

Each body of theory, the object it contributes, and where that object is worked out.
[`docs/MATHS.md`](../MATHS.md) carries the rationale; this table is the map.

| Strand | Canonical object | Realised as | Section | Gates |
|---|---|---|---|---|
| Optimal control | `ẋ = f(x,u)`, `u ∈ U(x)` | turn-rate-limited flight; phase-integrated orbits | §9.2 | V46, V47 |
| Dynamic programming | `J*(x) = min_u [c(x,u) + J*(f(x,u))]` | least-risk pathing; Dijkstra as label-setting value iteration | §5 | V25–V27 |
| Stochastic processes | Poisson rates, Gaussian dispersion, Markov chains | detection `λ`; CEP and Carleton damage; suppression chain; time-to-kill | §§2–4, §9.4 | V14–V24, V28–V31, V48–V49 |
| Game theory | `v = max_x min_y xᵀAy` | zero-sum interdiction game by fictitious play | §6 | V32–V39 |
| Partial observability | belief `b_t(s)`, the posterior over enemy position given `z_{1:t}` | belief filter with negative information; EW on the rate | §8 | V40–V43 |
| Combinatorial optimisation | `max Σ payoff[i][j] x_ij` over an assignment | side-wide weapon–target allocation (Kuhn–Munkres) | §10.2 | V56 |
| Information-gain control | maximise `H(b) − E[H(b')]` over the available looks | belief-driven sensor tasking | §10.3 | V57 |

Two structural properties cut across all five and are treated as first-class:

- **Determinism.** Same binary, same `(scenario, seed)` → bit-identical output. Every
  phase added since has been *appended* to the loop and draws zero RNG when its inputs
  are empty, so each new subsystem reduces to an exact identity when switched off
  (§8 for EW, §9.6 for air). Cross-platform bit-equality is not chased; float tests use
  explicit tolerances.
- **Validated before optimised.** No model is made fast until it has a gate against a
  closed form. Where that meant keeping a slow reference (the brute-force viewshed, the
  fixed-step LOS oracle, the greedy allocator), the reference is kept.
