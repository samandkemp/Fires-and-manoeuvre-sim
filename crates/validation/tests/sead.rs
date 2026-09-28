//! SEAD and the emission seam: killing what shoots back, anti-radiation homing,
//! counter-battery, and EMCON (V60, V64, V65, V69).
//!
//! One test binary over several suites: each `tests/*.rs` is its own link unit, so
//! the files under `tests/sead/` are modules of this one rather than binaries of their
//! own. `#[path]` is required because a test binary is a crate root - a bare `mod x;`
//! would look for `tests/x.rs` and become a separate binary again.

#[path = "sead/arm.rs"]
mod arm;
#[path = "sead/autonomy.rs"]
mod autonomy;
#[path = "sead/counter_battery.rs"]
mod counter_battery;
#[path = "sead/emcon.rs"]
mod emcon;
#[path = "sead/strike.rs"]
mod strike;
