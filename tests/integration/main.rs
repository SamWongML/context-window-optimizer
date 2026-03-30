//! Integration tests for the ctx-optim CLI and library.

mod cli;
mod efficiency;
mod pack_pipeline;

#[cfg(feature = "feedback")]
mod feedback;

#[cfg(feature = "watch")]
mod watch;
