//! Feedback loop: session tracking, utilization scoring, and weight learning.
//!
//! Behind the `feedback` Cargo feature flag.

pub mod learning;
pub mod store;
pub mod utilization;
