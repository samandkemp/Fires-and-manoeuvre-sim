//! SEAD and the emission seam: killing what shoots back, anti-radiation homing,
//! counter-battery, and EMCON (V60, V64, V65, V69).
//!
//! One test binary over several suites. Cargo builds every `tests/*.rs` as its own
//! binary linking `sim_core` afresh; files under `tests/sead/` are modules of this
//! one instead, which is why the suite relinks once here rather than 4 times.
//! Each module below is the file it always was, moved rather than rewritten.
//!
//! The `#[path]` attributes are not decoration: a test binary is a *crate root*, so a
//! bare `mod strike;` would look for `tests/strike.rs` -- back at the top level, where it
//! would become its own binary again and undo the grouping.

#[path = "sead/arm.rs"]
mod arm;
#[path = "sead/counter_battery.rs"]
mod counter_battery;
#[path = "sead/emcon.rs"]
mod emcon;
#[path = "sead/strike.rs"]
mod strike;
