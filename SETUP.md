# Pre-Project Setup — Rust + Bevy in VSCode

Target: **Bevy 0.19** on **stable Rust**, in **VSCode**. Written for someone new to
both languages. Follow it in order; each step ends with a check so you know it worked.

> Bevy releases breaking changes roughly every 3 months. When a newer version lands,
> the *shape* of this guide holds, but re-check version numbers and any migration guide
> at bevy.org before bumping.

---

## 1. Install Rust (the toolchain)

Rust is installed via `rustup`, which manages compiler versions for you.


- **Windows** — download and run `rustup-init.exe` from https://rustup.rs.
  It will tell you it needs the **Visual Studio C++ Build Tools** (the MSVC linker).
  Let it guide you, or install "Desktop development with C++" from the Visual Studio
  Installer first. This is required — Rust links through the MSVC toolchain on Windows.

- **macOS / Linux** — run in a terminal:
  ```
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  ```
  Accept the defaults. Then restart the terminal.

Then make sure you set default to stable:
```
rustup default stable
```

**Check it worked:**
```
rustc --version
cargo --version
```
Both should print a version. `cargo` is Rust's build tool + package manager

---

## 2. VSCode extensions

Install these from the Extensions panel (Ctrl/Cmd+Shift+X):

- **rust-analyzer** (by rust-lang) — *essential.* The language server: autocomplete,
  inline types, go-to-definition, error highlighting. This is 90% of the experience.
- **CodeLLDB** (by Vadim Chugunov) — debugger with breakpoints. Cross-platform.
- **Even Better TOML** — syntax + validation for `Cargo.toml` and config files.
- **Dependi** — shows latest crate versions inline in `Cargo.toml` (the older "crates"
  extension is deprecated; Dependi replaces it).
- **Error Lens** *(optional but great for beginners)* — prints errors inline on the
  line instead of only underlining them.

**Recommended settings.** Open your workspace settings (Ctrl/Cmd+Shift+P →
"Preferences: Open Workspace Settings (JSON)") and add:
```json
{
  "rust-analyzer.check.command": "clippy",
  "editor.formatOnSave": true,
  "[rust]": { "editor.defaultFormatter": "rust-lang.rust-analyzer" }
}
```
This runs Clippy (Rust's linter — it teaches you idiomatic Rust as you go) on save, and
auto-formats with `rustfmt`.

**Check it worked:** you'll verify rust-analyzer properly in step 4, once there's code.

> Tip: always open the **project root folder** in VSCode (File → Open Folder), not a
> single file, so rust-analyzer can see the whole project.

---

## 3. First build — a Bevy window (verify the toolchain)

Before structuring the real project, prove the whole chain works.

```
cargo new hello_bevy
cd hello_bevy
code .
```

Open `Cargo.toml` and add Bevy under `[dependencies]`:
```toml
[dependencies]
bevy = { version = "0.19", features = ["dynamic_linking"] }
```

Replace `src/main.rs` with:
```rust
use bevy::prelude::*;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .run();
}
```

Run it:
```
cargo run
```

**Expect the first build to take several minutes** — you're compiling an entire game
engine from source. This happens once. Every build after is fast. When it finishes, a
blank window opens. That means Rust, Cargo, Bevy, and your GPU path all work.

> `dynamic_linking` massively speeds up your *iterative* rebuilds. It's a dev-only
> convenience: **remove it (or put it behind your own feature flag) before you make a
> release build**, since it requires shipping a dynamic library alongside the binary.

---

## 4. Enable fast compiles (for development)

Enabling fast compiles during development prevents loss of momentum, at a slight cost to optimisation

**(a) Optimise dependencies in debug builds.** In `Cargo.toml`, add:
```toml
[profile.dev]
opt-level = 1

[profile.dev.package."*"]
opt-level = 3
```
This compiles *your* code fast (unoptimised) but *dependencies* optimised, so the sim
still runs at a usable speed while you iterate.

**(b) Use a fast linker.** Create `.cargo/config.toml` in the project root with the
block for your OS:

- **Windows** — `rust-lld` ships with Rust, so just:
  ```toml
  [target.x86_64-pc-windows-msvc]
  linker = "rust-lld.exe"
  ```
- **Linux** — first `sudo apt install lld clang` (Debian/Ubuntu), then:
  ```toml
  [target.x86_64-unknown-linux-gnu]
  linker = "clang"
  rustflags = ["-C", "link-arg=-fuse-ld=lld"]
  ```
- **macOS** — the default linker is already as fast as the alternatives. **Do nothing
  here**; `dynamic_linking` from step 3 is enough. (Ignore old tutorials pushing `zld`.)

**Check it worked:** make a trivial edit to `main.rs` (e.g. add a comment) and
`cargo run` again — the rebuild should be seconds, not minutes.

---

## 5. Workspace Structure (core vs app)

If setting up a new workspace, it is suggested to follow the architecture in the 
README: a **headless OR core** independent of Bevy, and a **Bevy app** that visually 
renders the model.

Keeping the OR core and Bevy app seperate makes the underlying maths independently 
testable and allows the user to run batch experiments without a visual render.

Create this layout (rename `hello_bevy` or start fresh):
```
fires-sim/
├── Cargo.toml            # workspace manifest
├── .cargo/config.toml    # linker settings from step 4
└── crates/
    ├── sim_core/         # pure Rust: terrain, fires, sensing, DP, game theory. NO bevy.
    │   ├── Cargo.toml
    │   └── src/lib.rs
    └── app/              # Bevy front-end. Depends on sim_core.
        ├── Cargo.toml
        └── src/main.rs
```

> **Do not call it `core`.** A dependency named `core` shadows Rust's own built-in
> `core` crate inside anything that depends on it, which breaks every proc macro that
> emits `::core::` paths — `thiserror`'s derive stops compiling, with an error message
> that points nowhere near the real cause. `sim_core` costs five characters and avoids
> the whole problem. (Learned the hard way; the crate manifests still carry a note.)

Root `Cargo.toml`:
```toml
[workspace]
resolver = "2"
members = ["crates/sim_core", "crates/app"]

[profile.dev]
opt-level = 1
[profile.dev.package."*"]
opt-level = 3
```

`crates/sim_core/Cargo.toml` (no Bevy — this is the OR engine):
```toml
[package]
name = "sim_core"
version = "0.1.0"
edition = "2021"

[dependencies]
glam = "0.32"        # vectors/matrices; same lib Bevy uses, so types interop cleanly
ndarray = "0.16"     # terrain rasters / grids
rand = "0.9"         # seeded RNG
rand_chacha = "0.9"  # ChaCha8Rng: a stream that stays stable across rand versions
rand_distr = "0.5"   # normal/other distributions for dispersion & stochastic models
```

> **Match glam to Bevy's.** Bevy 0.19 resolves glam 0.32; pin the same version or the
> "types interop cleanly" promise breaks silently — you get two incompatible `Vec2`
> types and a wall of confusing errors. Check `Cargo.lock` before bumping either side.
>
> **Prefer `ChaCha8Rng` to `StdRng`.** `StdRng` explicitly does not promise a stable
> stream across `rand` versions, so an archived `(scenario, seed)` result would quietly
> stop reproducing after a routine dependency bump. ChaCha8 does promise it.

`crates/app/Cargo.toml` (the Bevy side):
```toml
[package]
name = "app"
version = "0.1.0"
edition = "2021"

[dependencies]
sim_core = { path = "../sim_core" }
bevy = { version = "0.19", features = ["dynamic_linking"] }
```

Run the app crate with:
```
cargo run -p app
```
Run only the core's tests (headless, no window) with:
```
cargo test -p sim_core
```

**Check it worked:** `cargo run -p app` opens the window; `cargo test -p sim_core` runs
(even with zero tests) and reports success.

---

## 6. Potential Optional Crates 

Add these with `cargo add <name> -p <crate>` when you reach the relevant phase. For any
Bevy-ecosystem crate, **check its README for the Bevy version it targets** — they pin to
specific Bevy releases and lag a week or two behind a new Bevy.

- **bevy_egui** (`~0.40`, targets Bevy 0.19) — immediate-mode control panels: dropdowns,
  sliders, toggles. Ideal for selecting a sensing asset, placing it, and tweaking unit
  stats live. This is your main tool-UI workhorse.
- **bevy_pancam** — click-drag pan and scroll-zoom for a 2D camera. Near-essential for a
  tactical map. (Check its Bevy-0.19 compatibility on crates.io.)
- **nalgebra** *(sim_core, if needed)* — heavier linear algebra than glam, for control/DP
  maths where you want matrix decompositions etc.
- **argmin** or **good_lp** *(sim_core, later)* — optimisation / LP solving for the
  game-theoretic equilibria. Not needed yet: the zero-sum solver uses fictitious play,
  which converges without an LP dependency.

---

## 7. Rust and Bevy Resources

Bevy is hard to learn *while* also learning Rust's ownership model. Spend a little time
on fundamentals alongside the early phases:

- **The Rust Book** — https://doc.rust-lang.org/book — read chapters 1–10 (esp. 4
  "Ownership" and 10 "Generics/Traits"). This is the single best resource.
- **Rustlings** — https://github.com/rust-lang/rustlings — small in-terminal exercises;
  the fastest way to make the concepts stick.
- **Bevy Quick Start** — https://bevy.org/learn/quick-start — the official intro.
- **Bevy examples** — the `examples/` folder in the Bevy GitHub repo (check out the tag
  matching your version) is the most reliable, always-current reference.
- **Unofficial Bevy Cheat Book** — https://bevy-cheatbook.github.io — great task-oriented
  recipes; may lag a version behind, so cross-check against examples.

---

## Setup checklist

- [ ] `rustc --version` and `cargo --version` both print
- [ ] rust-analyzer installed; hovering a variable shows its type
- [ ] `cargo run` on the hello window opens a blank window
- [ ] fast-compile config in place; a one-line edit rebuilds in seconds
- [ ] workspace builds: `cargo run -p app` (window) and `cargo test -p sim_core` (headless)

## Common first-time issues

- **First build feels frozen** — it isn't; compiling the engine just takes minutes once.
- **rust-analyzer shows no types / errors everywhere** — it's still indexing on first
  open (can take a few minutes and a chunk of RAM). Wait for the spinner to finish.
- **Windows link errors** — you're missing the MSVC C++ Build Tools; reinstall them.
- **An ecosystem crate won't compile** — version mismatch with Bevy. Check that crate's
  README for the exact Bevy version it supports.
- **Release build fails to run elsewhere** — you left `dynamic_linking` on. Disable it
  for release.
