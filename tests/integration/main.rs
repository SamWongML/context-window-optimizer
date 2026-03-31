//! Integration tests for the ctx-optim CLI and library.

mod cli;
mod edge_cases;
mod mcp_efficiency;
mod pack_pipeline;

#[cfg(feature = "feedback")]
mod feedback;

#[cfg(feature = "watch")]
mod watch;
