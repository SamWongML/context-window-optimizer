//! Integration tests for the ctx-optim CLI and library.

#[path = "../fixtures/mod.rs"]
mod fixtures;

mod cli;
mod pack_pipeline;
mod realistic;

#[cfg(feature = "feedback")]
mod feedback;

#[cfg(feature = "watch")]
mod watch;
