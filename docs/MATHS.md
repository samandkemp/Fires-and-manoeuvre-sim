# The mathematics

Six bodies of theory, each doing a job the others cannot. Each is stated here in its own
symbols, with what it buys, the code it lives in, and the gate that holds it honest.

This is the **argument for why each tool is the right one**. It is not the specification:
[`docs/design/`](design/) works each subsystem out in full, and
[`docs/VALIDATION.md`](VALIDATION.md) says what every gate is checked against. If you want
the same models in plain terms with worked numbers instead, read
[`docs/HOW_IT_WORKS.md`](HOW_IT_WORKS.md).

| Strand | Object | Section |
|---|---|---|
| Optimal control | `ẋ = f(x,u)`, `u ∈ U(x)` | [1](#1-optimal-control--the-continuous-dynamics) |
| Dynamic programming | `J*(x) = min_u [c(x,u) + J*(f(x,u))]` | [2](#2-dynamic-programming--movement-as-a-least-risk-value-function) |
| Stochastic processes | Poisson rates, Gaussian dispersion, Markov chains | [3](#3-stochastic-processes--detection-fires-suppression-attrition) |
| Game theory | `v = max_x min_y xᵀAy` | [4](#4-game-theory--sensing-against-counter-sensing) |
| Partial observability | belief `b_t(s)` given `z_{1:t}` | [5](#5-partial-observability--belief-and-the-value-of-not-seeing) |
| Combinatorial optimisation | `max Σ payoff[i][j] x_ij` over an assignment | [6](#6-combinatorial-optimisation--who-shoots-what) |

The simulation loop is **hybrid continuous/discrete**. Between decision epochs the state
integrates continuously; at each epoch the discrete decisions are set — what to shoot,
where to move, where to look. That split is not a convenience, it is the structure:
continuous dynamics are an optimal-control problem, the epoch-to-epoch choices are a
dynamic program, and the two only compose cleanly if they are kept apart.

## 1. Optimal control — the continuous dynamics

State evolves under a control you may choose but not violate:

$$
\dot{x} = f(x, u), \qquad u \in U(x)
$$

For an airframe the state is `x = (p, ψ, v)` — position, heading, speed — and the
binding constraint is a **turn rate**, not a position:

$$
|\dot{\psi}| \le \omega_{\max} \quad\Longrightarrow\quad r_{\min} = \frac{v}{\omega_{\max}}
$$

A rate limit is what separates a flight path from a polyline. Ask a drone to fly a
90° corner and it cannot; it flies an arc of radius `r_min`, arriving late and displaced.
That error is exactly what an air defence's engagement window is made of, so modelling
heading as a state with a bounded derivative is load-bearing rather than decorative.

Orbits integrate the **phase** rather than steering toward the circle:

$$
\theta(t + \Delta t) = \theta(t) \pm \frac{v}{R} \Delta t, \qquad p = c + R (\cos\theta, \sin\theta)
$$

Steering would accumulate radius drift over a long loiter; integrating phase holds
`‖p − c‖ = R` exactly and gives the lap time `T = 2πR/v` in closed form to test against.

*Lives in* [`air.rs`](../crates/sim_core/src/air.rs) *·  gates V46, V47*

## 2. Dynamic programming — movement as a least-risk value function

A mover choosing a route is solving Bellman's equation over the terrain grid:

$$
J^*(x) = \min_{u \in U(x)} \Big[ c(x, u) + J^*\big(f(x, u)\big) \Big]
$$

with `x` a cell, `u` one of its eight neighbours, and the edge cost

$$
c(x, u) = c_{\text{move}}(x, u) + w \cdot \text{risk}(u)
$$

`move_cost` is mobility × slope × distance (∞ for impassable); `risk ∈ [0,1]` is a
supplied exposure raster. Because every cost is non-negative there are no negative
cycles, so Dijkstra returns `J*` exactly — label-setting value iteration, not an
approximation of it. Full value iteration is held in reserve for when risk becomes
time-varying and the one-sweep ordering no longer holds.

The interesting object is `w`. It is an **exchange rate**: the metres of mobility cost
a commander will spend to avoid one unit of exposure. Sweeping `w` from 0 upward traces
the Pareto frontier between arriving quickly and arriving alive, which is a far more
useful answer than any single "optimal" route. Additive costs were chosen over
maximising survival `Π(1 − p_death)` because the logarithm turns that into the same
additive problem anyway, and additive keeps the Dijkstra gate clean.

Risk defaults to **enemy observation coverage** — for each cell, the detection rate a
reference mover would suffer from the best-placed enemy sensor. So "least-risk path"
literally means "the route that stays hardest to see", and the see-without-being-seen
idea becomes navigable.

*Lives in* [`movement.rs`](../crates/sim_core/src/movement.rs) *·  gates V25–V27*

## 3. Stochastic processes — detection, fires, suppression, attrition

Four processes, chosen so that each has a closed form to test against.

### Detection is a rate, never a per-tick probability

Sensor `s` detects unit `u` as a Poisson process of rate

$$
\lambda(s,u) = \lambda_0 \cdot f(r) \cdot \sigma_m(u) \cdot \tau(s,u) \cdot \big(1 - c(u)\big),
\qquad f(r) = \frac{1}{1 + (r/r_{1/2})^{n}}
$$

where `σ_m` is signature in the sensor's modality, `τ = exp(−κL)` the canopy
transmittance along the sightline, `c` the target's terrain concealment, and `r` slant
range. The rate is zero when LOS is blocked, range exceeds the cutoff, or the target
falls outside the field of regard.

$$
P(\text{detected by } t) = 1 - e^{-\lambda t}
\qquad\text{per tick:}\quad p = 1 - e^{-\lambda \Delta t}
$$

Modelling a **rate** rather than a per-tick probability is what makes the answer
independent of the tick size. Memorylessness gives the exact identity

$$
\prod_k e^{-\lambda \Delta t_k} = e^{-\lambda \sum_k \Delta t_k}
$$

for any subdivision of the interval, so halving `dt` cannot change the detection
statistics. A per-tick probability would silently make the physics a function of the
integrator — a class of bug that is very hard to see and very easy to ship. V17 checks
the identity to float tolerance.

### Fires: two Gaussians, two closed forms

Direct fire scatters isotropically about the aim point with `σ(r) = δ·r/1000` for an
angular dispersion `δ` mrad. Deflection and elevation errors are independent, so hitting
a `W × H` silhouette is a product of two one-dimensional Gaussian integrals:

$$
P_{\text{hit}}(r) = \mathrm{erf}\left(\frac{W}{2\sigma(r)\sqrt{2}}\right) \cdot
\mathrm{erf}\left(\frac{H}{2\sigma(r)\sqrt{2}}\right)
$$

Indirect fire samples a burst `b ~ N(aim, σ²I)` with `σ = CEP / √(2 ln 2)` — the
circular-Gaussian CEP identity — and delivers the **Carleton** incapacitation kernel
`D(ρ) = exp(−ρ² / 2R_L²)` at burst-to-target distance `ρ`. Marginalising the kernel over
the burst distribution is a Gaussian convolving a Gaussian, so the expected damage at
aim offset `d` is exact:

$$
\mathbb{E}[D(d)] = \frac{R_L^2}{\sigma^2 + R_L^2}
\exp\left(-\frac{d^2}{2 (\sigma^2 + R_L^2)}\right)
$$

That closed form is the reason for this kernel. A cookie-cutter lethality disc is
simpler and cheaper, but it has no analytical expectation, so the Monte Carlo sampler
would have nothing to be checked against — and an unvalidated sampler is exactly what
this project is trying not to build.

### Suppression is a birth–death chain

Per-unit state `S ∈ {Free, Suppressed, Pinned}`, stepping up on near-misses at rate `β`
and decaying at rate `μ`:

$$
\text{Free} \rightleftharpoons \text{Suppressed} \rightleftharpoons \text{Pinned}
\qquad \pi_k \propto (\beta/\mu)^k, \quad k = 0,1,2
$$

The birth–death structure hands over its stationary distribution for free, and the mean
time Pinned → Free is `2/μ` — two exponential steps in series. Suppression is a *state*
rather than a scalar multiplier because it gates behaviour discontinuously: Pinned units
neither fire nor move, which is what lets fires shape manoeuvre without killing anybody.

### Attrition against Lanchester

Fire volume scales with a unit's surviving elements, so an aimed-fire duel obeys

$$
\frac{dA}{dt} = -\beta B, \quad \frac{dB}{dt} = -\alpha A
\qquad\Longrightarrow\qquad \alpha\big(A_0^2 - A^2\big) = \beta\big(B_0^2 - B^2\big)
$$

Lanchester's **square law**: combat power goes as the square of numbers under aimed
fire. Reproducing it is the strongest single check on the attrition chain, because it
is an emergent property of the whole loop — element counts, rate of fire, hit
probability, removal — rather than of any one function.

### Air defence: same target, two distributions

$$
\text{gun:} \quad \text{TTK} \sim \mathrm{Exp}(\lambda_k)
\quad \Rightarrow \quad \mathbb{E}[\text{TTK}] = \frac{1}{\lambda_k}
$$

$$
\text{missile:} \quad \text{shots} \sim \mathrm{Geometric}(p)
\quad \Rightarrow \quad \mathbb{E}[\text{TTK}] = \frac{t_f}{p} + \left(\frac{1}{p} - 1\right) t_r
$$

A gun grinds continuously; a missile is discrete shoot-look-shoot with flight time `t_f`
and reload `t_r`. The distributions differ in shape, not just in mean, so guns and
missiles fail differently against a saturating raid — which is the whole reason to model
both rather than tune one "effectiveness" number.

*Lives in* [`sensing.rs`](../crates/sim_core/src/sensing.rs), [`fires.rs`](../crates/sim_core/src/fires.rs),
[`suppression.rs`](../crates/sim_core/src/suppression.rs), [`air_defence.rs`](../crates/sim_core/src/air_defence.rs)
*·  gates V14–V24, V28–V31, V48–V49*

## 4. Game theory — sensing against counter-sensing

Where to put a sensor has no best answer, only a best answer *against a thinking
opponent*. That is a zero-sum matrix game. Blue mixes over positions `x ∈ Δ_m`, Red over
routes `y ∈ Δ_n`, and von Neumann's minimax theorem says the game has a value:

$$
v = \max_{x} \min_{y} x^{\mathsf{T}} A y = \min_{y} \max_{x} x^{\mathsf{T}} A y
$$

Solved by **fictitious play** — each side repeatedly best-responds to the opponent's
empirical distribution of past play:

$$
i^* = \arg\max_i \sum_j A_{ij} N^{\text{col}}_j
\qquad
j^* = \arg\min_j \sum_i A_{ij} N^{\text{row}}_i
$$

The time-averaged strategies converge to equilibrium for zero-sum games (Robinson, 1951)
with no LP dependency, and convergence is self-certifying: the value is bracketed by

$$
v_{\text{low}} = \min_j \big(x^{\mathsf{T}}A\big)_j \le v \le \max_i \big(Ay\big)_i = v_{\text{high}}
$$

and the gap `v_high − v_low` shrinks to zero. You can watch the bracket close.

The payoff `A[b][r]` is expected Red attrition when Red traverses route `r` while Blue
observes and bombards from position `b`, estimated by short headless Monte Carlo
battles. So the matrix is built by the simulation itself, and the equilibrium is a
statement about *this* terrain and *these* sensors — not an abstract game.

*Lives in* [`game.rs`](../crates/sim_core/src/game.rs) *·  gates V32–V39*

## 5. Partial observability — belief, and the value of not seeing

Once electronic warfare degrades sensing, the tracked quantity is no longer enemy
position but a **belief** over it: `b_t(s) = P(enemy at s | z_{1:t})`, maintained by the
two standard steps,

$$
\text{update:} \quad b_t(s) \propto P(z_t \mid s) b_{t-1}(s)
$$

$$
\text{predict:} \quad b^{-}_t(s') = \sum_s T(s' \mid s) b_{t-1}(s)
$$

The load-bearing observation is the **negative** one. Not seeing something is evidence:

$$
P(\text{no detection} \mid \text{enemy at } s) = \exp\big(-\lambda(s) \Delta t\big)
$$

A cell your sensor covers well has a low likelihood of producing no detection, so
belief drains out of it; dead ground and jammed cells sit near 1 and keep their mass.
Stare at open ground long enough and the belief mass migrates, unprompted, into the
folds and the woodline — which is what a competent staff officer does with the same
information, and it falls straight out of Bayes rather than being scripted.

EW enters the *rate*, not the geometry:

$$
\lambda_{\text{eff}} = \lambda \cdot \prod_j g_j(\text{target}),
\qquad g_j = 1 - \text{power}_j\left(1 - \frac{d}{\text{radius}_j}\right)
$$

so a jammer raises belief entropy `H(b) = −Σ_s b(s) log b(s)` without moving anything.
With no jammers every `g_j = 1` and the product is exactly 1, so EW-off reduces to the
sensing model bit-for-bit (V40) — an identity, not an approximation.

Entropy is also the control objective for the sensor-tasking layer: point each steerable
sensor at the facing maximising the *expected* reduction in `H(b)`. For a candidate
facing, either something is detected — collapsing belief to a point of zero entropy — or
nothing is, and belief becomes `b'(c) ∝ b(c)(1 − p(c))`, so

$$
\text{gain} = H(b) - \left(1 - \sum_c b(c) p(c)\right) H(b')
$$

That closes the loop **sensing → belief → decision → action**. Measured: three
narrow-arc observers searching for five dispersed units find **2 of 5** staring where
they were placed, and **5 of 5** when tasked by belief — and nothing about the resulting
sweep is scripted. Each sensor drains its own belief out of ground it has cleared, so the
best-information facing moves on by itself.

*Lives in* [`ew.rs`](../crates/sim_core/src/ew.rs), [`pomdp.rs`](../crates/sim_core/src/pomdp.rs),
[`sim/tasking.rs`](../crates/sim_core/src/sim/tasking.rs) *·  gates V40–V43, V57*

## 6. Combinatorial optimisation — who shoots what

Which shooter engages which target is an assignment problem, and solving it side-wide
rather than shooter-by-shooter is the difference between three tanks all firing at the
nearest enemy and three tanks covering three enemies. For shooter `i` and slot `k` of
target `j`,

$$
\text{payoff}\big[i\big]\big[(j,k)\big] = q(i,j)\cdot \text{value}(j)\cdot \big(1 - \bar{q}(j)\big)^{k}
$$

`q` is the fraction of the target destroyed this epoch, straight from the fires model.
The geometric term is the diminishing return on piling on: the (k+1)-th shooter only
helps if the `k` before it all failed. Expressing it as extra columns is what keeps this
a plain linear assignment — solved optimally by Kuhn–Munkres — instead of a submodular
problem needing a bespoke solver.

A greedy allocator ships alongside, and it earns its place by settling what the optimal
solver is actually worth. On a scenario built to present a real choice, 500 seeds compared
paired: **coordinating is worth −11.3 ± 0.5 s (~15%)** off the time to destroy the enemy,
unambiguously. **Optimality is worth nothing measurable** — greedy and Hungarian differ by
0.12 s against a standard error of ~0.5, and agree outright on most seeds. At this scale
greedy's myopia costs it essentially nothing.

That is the useful kind of negative result, and it only exists because the baseline was
kept. (An earlier version of this claimed greedy *beat* the optimal solver; it did not —
that came from unpaired means with no error bars. The experiment now reports a standard
error on every figure.)

*Lives in* [`allocation.rs`](../crates/sim_core/src/allocation.rs) *·  gate V56*
