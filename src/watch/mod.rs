//! File watcher for incremental re-indexing.
//!
//! Behind the `watch` Cargo feature flag.
//! Uses the `notify` crate to detect filesystem changes and trigger
//! re-discovery of modified files.
