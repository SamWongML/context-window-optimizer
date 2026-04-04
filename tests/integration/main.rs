//! Integration tests for the ctx-optim CLI and library.

#[allow(dead_code, unused_imports)]
#[path = "../fixtures/mod.rs"]
mod fixtures;

mod cli;
mod pack_pipeline;
mod realistic;

#[cfg(feature = "feedback")]
mod feedback;

#[cfg(feature = "watch")]
mod watch;
