//! Air and counter-air: flight, air defence, and the sensor-to-shooter timeline
//! (V44-V52).
//!
//! One test binary over several suites: each `tests/*.rs` is its own link unit, so
//! the files under `tests/air/` are modules of this one rather than binaries of their
//! own. `#[path]` is required because a test binary is a crate root - a bare `mod x;`
//! would look for `tests/x.rs` and become a separate binary again.

#[path = "air/counter_air.rs"]
mod counter_air;
#[path = "air/flight.rs"]
mod flight;
