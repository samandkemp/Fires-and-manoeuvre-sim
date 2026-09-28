//! Command and control: coordination as a placed asset, the link that degrades before
//! it dies, and fires that can be made to need the net (V59, V62, V63).
//!
//! One test binary over several suites: each `tests/*.rs` is its own link unit, so
//! the files under `tests/c2/` are modules of this one rather than binaries of their
//! own. `#[path]` is required because a test binary is a crate root - a bare `mod x;`
//! would look for `tests/x.rs` and become a separate binary again.

#[path = "c2/coordination.rs"]
mod coordination;
#[path = "c2/fires_net.rs"]
mod fires_net;
#[path = "c2/link.rs"]
mod link;
