//! Integration tests for the feedback loop: pack → feedback → learning.

use ctx_optim::{
    config::Config,
    feedback::{
        learning::run_learning_cycle,
        store::{FeedbackRecord, FeedbackStore},
        utilization::utilization_score,
    },
    pack_files,
    types::Budget,
};
use tempfile::TempDir;

fn make_repo_with_files() -> TempDir {
    let dir = TempDir::new().unwrap();
    let root = dir.path();

    // Create source files with distinct identifiers
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/scoring.rs"),
        "pub fn compute_score(entry: &FileEntry, weights: &ScoringWeights) -> f32 {\n    \
         entry.recency * weights.recency + entry.size * weights.size\n}\n",
    )
    .unwrap();
    std::fs::write(
        root.join("src/discovery.rs"),
        "pub fn discover_files(root: &Path, config: &Config) -> Vec<FileEntry> {\n    \
         walk_directory(root).filter(|f| !f.is_ignored()).collect()\n}\n",
    )
    .unwrap();
    std::fs::write(
        root.join("src/knapsack.rs"),
        "pub fn greedy_select(items: &[ScoredEntry], budget: usize) -> Vec<ScoredEntry> {\n    \
         items.sort_by_efficiency().take_while(|i| total <= budget).collect()\n}\n",
    )
    .unwrap();

    dir
}

#[test]
fn test_feedback_loop_end_to_end() {
    let repo = make_repo_with_files();
    let config = Config::default();
    let budget = Budget::standard(128_000);

    // Step 1: Pack context
    let result = pack_files(repo.path(), &budget, &[], &config).unwrap();
    assert!(!result.session_id.is_empty(), "session_id should be set");
    assert!(!result.selected.is_empty(), "should select files");

    // Step 2: Open feedback store
    let db_path = repo.path().join(".ctx-optim/feedback.db");
    let store = FeedbackStore::open(&db_path).unwrap();

    // Verify session was auto-stored
    let session = store.get_session(&result.session_id).unwrap();
    assert!(session.is_some(), "session should be stored");

    // Step 3: Simulate LLM response that references scoring.rs identifiers
    let llm_response = "I used compute_score with the ScoringWeights to calculate \
                        the FileEntry recency and size signals.";

    // Step 4: Compute utilization for each selected file
    let feedback_records: Vec<FeedbackRecord> = result
        .selected
        .iter()
        .map(|s| {
            let content = std::fs::read_to_string(&s.entry.path).unwrap_or_default();
            let util = utilization_score(&content, llm_response);
            FeedbackRecord {
                file_path: s.entry.path.to_string_lossy().to_string(),
                token_count: s.entry.token_count,
                composite_score: s.composite_score,
                was_selected: true,
                utilization: Some(util),
            }
        })
        .collect();

    store
        .record_feedback(&result.session_id, &feedback_records)
        .unwrap();

    // Verify: scoring.rs should have higher utilization than others
    let stored = store.get_session_feedback(&result.session_id).unwrap();
    let scoring_util = stored
        .iter()
        .find(|r| r.file_path.contains("scoring"))
        .and_then(|r| r.utilization);
    let discovery_util = stored
        .iter()
        .find(|r| r.file_path.contains("discovery"))
        .and_then(|r| r.utilization);

    if let (Some(s), Some(d)) = (scoring_util, discovery_util) {
        assert!(
            s > d,
            "scoring.rs utilization ({s:.3}) should be > discovery.rs ({d:.3})"
        );
    }
}

#[test]
fn test_weight_learning_after_multiple_sessions() {
    let repo = make_repo_with_files();
    let config = Config::default();
    let budget = Budget::standard(128_000);
    let db_path = repo.path().join(".ctx-optim/feedback.db");
    let store = FeedbackStore::open(&db_path).unwrap();

    // Run multiple pack + feedback cycles
    for i in 0..5 {
        let result = pack_files(repo.path(), &budget, &[], &config).unwrap();

        let response = if i % 2 == 0 {
            "compute_score FileEntry ScoringWeights recency size"
        } else {
            "discover_files walk_directory Config FileEntry is_ignored"
        };

        let records: Vec<FeedbackRecord> = result
            .selected
            .iter()
            .map(|s| {
                let content = std::fs::read_to_string(&s.entry.path).unwrap_or_default();
                FeedbackRecord {
                    file_path: s.entry.path.to_string_lossy().to_string(),
                    token_count: s.entry.token_count,
                    composite_score: s.composite_score,
                    was_selected: true,
                    utilization: Some(utilization_score(&content, response)),
                }
            })
            .collect();

        // Use a unique session ID for each cycle (the auto-generated one may collide in fast loops)
        let session_id = format!("test-session-{i}");
        let session = ctx_optim::feedback::store::Session {
            id: session_id.clone(),
            repo: repo.path().to_string_lossy().to_string(),
            budget: budget.total_tokens,
            created_at: ctx_optim::feedback::unix_now() + i as i64,
        };
        store.create_session(&session).unwrap();
        store.record_feedback(&session_id, &records).unwrap();
    }

    // Run weight learning
    let updated = run_learning_cycle(&store, &config.weights, 0.1).unwrap();
    assert!(updated.is_some(), "should produce updated weights");

    let w = updated.unwrap();
    let sum = w.recency + w.size + w.proximity + w.dependency;
    assert!(
        (sum - 1.0).abs() < 1e-3,
        "weights should sum to ~1.0, got {sum}"
    );

    // Verify weights were stored
    let stored = store.latest_weights().unwrap();
    assert!(stored.is_some(), "weights should be persisted");
}
