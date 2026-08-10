//! Command and control: coordination as a placed asset, the link that degrades before
//! it dies, and fires that can be made to need the net (V59, V62, V63).
//!
//! One test binary over several suites. Cargo builds every `tests/*.rs` as its own
//! binary linking `sim_core` afresh; files under `tests/c2/` are modules of this
//! one instead, which is why the suite relinks once here rather than 3 times.
//! Each module below is the file it always was, moved rather than rewritten.
//!
//! The `#[path]` attributes are not decoration: a test binary is a *crate root*, so a
//! bare `mod coordination;` would look for `tests/coordination.rs` -- back at the top level, where it
//! would become its own binary again and undo the grouping.

#[path = "c2/coordination.rs"]
mod coordination;
#[path = "c2/fires_net.rs"]
mod fires_net;
#[path = "c2/link.rs"]
mod link;
