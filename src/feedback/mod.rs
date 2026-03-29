//! Feedback loop: session tracking, utilization scoring, and weight learning.
//!
//! Behind the `feedback` Cargo feature flag.

pub mod learning;
pub mod store;
pub mod utilization;

use md5::{Digest, Md5};
use std::time::{SystemTime, UNIX_EPOCH};

/// Generate a unique session ID from the repo path and current time.
///
/// Format: hex-encoded MD5 of `"{repo}:{unix_timestamp_nanos}"`, truncated to 16 chars.
///
/// # Examples
///
/// ```
/// use ctx_optim::feedback::generate_session_id;
/// let id = generate_session_id("/tmp/my-repo");
/// assert_eq!(id.len(), 16);
/// assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
/// ```
pub fn generate_session_id(repo_path: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let input = format!("{repo_path}:{nanos}");
    let digest = Md5::digest(input.as_bytes());
    format!("{:032x}", u128::from_be_bytes(digest.into()))[..16].to_string()
}

/// Get the current Unix timestamp in seconds.
///
/// # Examples
///
/// ```
/// use ctx_optim::feedback::unix_now;
/// assert!(unix_now() > 0);
/// ```
pub fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_session_id_length() {
        let id = generate_session_id("/some/repo");
        assert_eq!(id.len(), 16, "session ID should be 16 hex chars: {id}");
    }

    #[test]
    fn test_generate_session_id_hex_chars() {
        let id = generate_session_id("/repo");
        assert!(
            id.chars().all(|c| c.is_ascii_hexdigit()),
            "session ID should be hex: {id}"
        );
    }

    #[test]
    fn test_unix_now_positive() {
        assert!(unix_now() > 0);
    }
}
