//! SQLite-backed storage for feedback sessions and weight history.
//!
//! Enabled via the `feedback` Cargo feature flag. Uses `rusqlite` with the
//! bundled SQLite amalgamation so there is no system dependency.

use std::path::Path;

use rusqlite::{Connection, params};

use crate::error::OptimError;

// ── Data types ────────────────────────────────────────────────────────────────

/// A feedback session record.
///
/// A session corresponds to a single call to `pack_files` and groups
/// the per-file [`FeedbackRecord`]s that were produced during that run.
///
/// # Examples
///
/// ```
/// use ctx_optim::feedback::store::Session;
///
/// let s = Session {
///     id: "sess-abc".to_string(),
///     repo: "/home/user/project".to_string(),
///     budget: 128_000,
///     created_at: 1_700_000_000,
/// };
/// assert_eq!(s.budget, 128_000);
/// ```
#[derive(Debug, Clone)]
pub struct Session {
    /// Unique session identifier (UUID or similar).
    pub id: String,
    /// Absolute path to the repository root.
    pub repo: String,
    /// Token budget used for this session.
    pub budget: usize,
    /// Unix timestamp (seconds) when the session was created.
    pub created_at: i64,
}

/// Per-file feedback record within a session.
///
/// Stores the scoring and selection outcome for a single file. The
/// `utilization` field is populated later by the utilization scorer.
///
/// # Examples
///
/// ```
/// use ctx_optim::feedback::store::FeedbackRecord;
///
/// let r = FeedbackRecord {
///     file_path: "src/main.rs".to_string(),
///     token_count: 512,
///     composite_score: 0.85,
///     was_selected: true,
///     utilization: Some(0.6),
/// };
/// assert!(r.was_selected);
/// ```
#[derive(Debug, Clone)]
pub struct FeedbackRecord {
    /// Relative or absolute path to the file.
    pub file_path: String,
    /// Number of tokens in the file.
    pub token_count: usize,
    /// Composite score assigned at pack time (in `[0.0, 1.0]`).
    pub composite_score: f32,
    /// Whether the file was included in the final selection.
    pub was_selected: bool,
    /// Fraction of the file's identifiers that were actually used by the LLM.
    /// `None` if utilization has not yet been computed.
    pub utilization: Option<f32>,
}

/// Stored weight snapshot from the learning algorithm.
///
/// Captured whenever the EMA-based weight updater produces a new weight
/// vector. The `avg_utilization` field records the mean utilization across
/// all training records at snapshot time.
///
/// # Examples
///
/// ```
/// use ctx_optim::feedback::store::WeightSnapshot;
///
/// let w = WeightSnapshot {
///     recency: 0.4,
///     size: 0.2,
///     proximity: 0.3,
///     dependency: 0.1,
///     avg_utilization: 0.55,
///     created_at: 1_700_001_000,
/// };
/// assert!((w.recency + w.size + w.proximity + w.dependency - 1.0).abs() < 1e-6);
/// ```
#[derive(Debug, Clone)]
pub struct WeightSnapshot {
    /// Recency signal weight.
    pub recency: f32,
    /// Size signal weight.
    pub size: f32,
    /// Proximity signal weight.
    pub proximity: f32,
    /// Dependency signal weight.
    pub dependency: f32,
    /// Mean utilization across the training window.
    pub avg_utilization: f32,
    /// Unix timestamp (seconds) when the snapshot was saved.
    pub created_at: i64,
}

// ── Store ─────────────────────────────────────────────────────────────────────

/// SQLite-backed store for feedback sessions, per-file records, and weight history.
///
/// Wrap in an `Arc<Mutex<FeedbackStore>>` when sharing across async tasks.
///
/// # Examples
///
/// ```no_run
/// use ctx_optim::feedback::store::FeedbackStore;
///
/// let store = FeedbackStore::open_in_memory().unwrap();
/// assert_eq!(store.session_count().unwrap(), 0);
/// ```
pub struct FeedbackStore {
    conn: Connection,
}

impl FeedbackStore {
    /// Open (or create) the SQLite database at `path`.
    ///
    /// Parent directories are created if they do not exist. The schema is
    /// created on first open via [`Self::create_schema`].
    ///
    /// # Errors
    ///
    /// Returns [`OptimError::Feedback`] if the directory cannot be created or
    /// the connection cannot be opened.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ctx_optim::feedback::store::FeedbackStore;
    ///
    /// let store = FeedbackStore::open("/tmp/ctx-optim/feedback.db").unwrap();
    /// ```
    pub fn open(path: impl AsRef<Path>) -> Result<Self, OptimError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| OptimError::Feedback(format!("create dirs: {e}")))?;
        }
        let conn =
            Connection::open(path).map_err(|e| OptimError::Feedback(format!("open db: {e}")))?;
        let store = Self { conn };
        store.create_schema()?;
        Ok(store)
    }

    /// Open an in-memory SQLite database for testing.
    ///
    /// The schema is created automatically.
    ///
    /// # Errors
    ///
    /// Returns [`OptimError::Feedback`] if the connection cannot be created.
    ///
    /// # Examples
    ///
    /// ```
    /// use ctx_optim::feedback::store::FeedbackStore;
    ///
    /// let store = FeedbackStore::open_in_memory().unwrap();
    /// assert_eq!(store.session_count().unwrap(), 0);
    /// ```
    pub fn open_in_memory() -> Result<Self, OptimError> {
        let conn = Connection::open_in_memory()
            .map_err(|e| OptimError::Feedback(format!("open in-memory db: {e}")))?;
        let store = Self { conn };
        store.create_schema()?;
        Ok(store)
    }

    /// Create all tables if they do not already exist.
    ///
    /// Idempotent — safe to call multiple times.
    ///
    /// # Errors
    ///
    /// Returns [`OptimError::Feedback`] if any DDL statement fails.
    ///
    /// # Examples
    ///
    /// ```
    /// use ctx_optim::feedback::store::FeedbackStore;
    ///
    /// let store = FeedbackStore::open_in_memory().unwrap();
    /// // calling again is a no-op
    /// store.create_schema().unwrap();
    /// ```
    pub fn create_schema(&self) -> Result<(), OptimError> {
        self.conn
            .execute_batch(
                "
                CREATE TABLE IF NOT EXISTS sessions (
                    id          TEXT PRIMARY KEY,
                    repo        TEXT NOT NULL,
                    budget      INTEGER NOT NULL,
                    created_at  INTEGER NOT NULL
                );

                CREATE TABLE IF NOT EXISTS feedback (
                    id              INTEGER PRIMARY KEY AUTOINCREMENT,
                    session_id      TEXT NOT NULL REFERENCES sessions(id),
                    file_path       TEXT NOT NULL,
                    token_count     INTEGER NOT NULL,
                    composite_score REAL NOT NULL,
                    was_selected    INTEGER NOT NULL,
                    utilization     REAL,
                    UNIQUE(session_id, file_path)
                );

                CREATE TABLE IF NOT EXISTS weight_history (
                    id              INTEGER PRIMARY KEY AUTOINCREMENT,
                    recency         REAL NOT NULL,
                    size            REAL NOT NULL,
                    proximity       REAL NOT NULL,
                    dependency      REAL NOT NULL,
                    avg_utilization REAL NOT NULL,
                    created_at      INTEGER NOT NULL
                );
                ",
            )
            .map_err(|e| OptimError::Feedback(format!("create schema: {e}")))
    }

    /// Persist a new [`Session`] record.
    ///
    /// # Errors
    ///
    /// Returns [`OptimError::Feedback`] on a constraint violation or I/O error.
    ///
    /// # Examples
    ///
    /// ```
    /// use ctx_optim::feedback::store::{FeedbackStore, Session};
    ///
    /// let store = FeedbackStore::open_in_memory().unwrap();
    /// let session = Session {
    ///     id: "s1".to_string(),
    ///     repo: "/repo".to_string(),
    ///     budget: 64_000,
    ///     created_at: 0,
    /// };
    /// store.create_session(&session).unwrap();
    /// assert_eq!(store.session_count().unwrap(), 1);
    /// ```
    pub fn create_session(&self, session: &Session) -> Result<(), OptimError> {
        self.conn
            .execute(
                "INSERT INTO sessions (id, repo, budget, created_at) VALUES (?1, ?2, ?3, ?4)",
                params![
                    session.id,
                    session.repo,
                    session.budget as i64,
                    session.created_at,
                ],
            )
            .map_err(|e| OptimError::Feedback(format!("create_session: {e}")))?;
        Ok(())
    }

    /// Retrieve a [`Session`] by its ID.
    ///
    /// Returns `None` if no session with `session_id` exists.
    ///
    /// # Errors
    ///
    /// Returns [`OptimError::Feedback`] on a database error.
    ///
    /// # Examples
    ///
    /// ```
    /// use ctx_optim::feedback::store::{FeedbackStore, Session};
    ///
    /// let store = FeedbackStore::open_in_memory().unwrap();
    /// assert!(store.get_session("missing").unwrap().is_none());
    /// ```
    pub fn get_session(&self, session_id: &str) -> Result<Option<Session>, OptimError> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, repo, budget, created_at FROM sessions WHERE id = ?1")
            .map_err(|e| OptimError::Feedback(format!("get_session prepare: {e}")))?;

        let mut rows = stmt
            .query(params![session_id])
            .map_err(|e| OptimError::Feedback(format!("get_session query: {e}")))?;

        if let Some(row) = rows
            .next()
            .map_err(|e| OptimError::Feedback(format!("get_session row: {e}")))?
        {
            Ok(Some(Session {
                id: row
                    .get(0)
                    .map_err(|e| OptimError::Feedback(format!("get_session col id: {e}")))?,
                repo: row
                    .get(1)
                    .map_err(|e| OptimError::Feedback(format!("get_session col repo: {e}")))?,
                budget: row
                    .get::<_, i64>(2)
                    .map_err(|e| OptimError::Feedback(format!("get_session col budget: {e}")))?
                    as usize,
                created_at: row
                    .get(3)
                    .map_err(|e| OptimError::Feedback(format!("get_session col ts: {e}")))?,
            }))
        } else {
            Ok(None)
        }
    }

    /// Persist a batch of [`FeedbackRecord`]s for a session.
    ///
    /// Uses `INSERT OR REPLACE` (upsert on `(session_id, file_path)`) and
    /// wraps the entire batch in a single transaction for performance.
    ///
    /// # Errors
    ///
    /// Returns [`OptimError::Feedback`] if the transaction or any insert fails.
    ///
    /// # Examples
    ///
    /// ```
    /// use ctx_optim::feedback::store::{FeedbackStore, FeedbackRecord, Session};
    ///
    /// let store = FeedbackStore::open_in_memory().unwrap();
    /// let session = Session { id: "s1".to_string(), repo: "/r".to_string(), budget: 1000, created_at: 0 };
    /// store.create_session(&session).unwrap();
    ///
    /// let records = vec![FeedbackRecord {
    ///     file_path: "src/lib.rs".to_string(),
    ///     token_count: 200,
    ///     composite_score: 0.9,
    ///     was_selected: true,
    ///     utilization: Some(0.7),
    /// }];
    /// store.record_feedback("s1", &records).unwrap();
    /// ```
    pub fn record_feedback(
        &self,
        session_id: &str,
        records: &[FeedbackRecord],
    ) -> Result<(), OptimError> {
        // Safety: unchecked_transaction is safe here — we hold the only
        // connection and we will commit or roll back explicitly.
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| OptimError::Feedback(format!("record_feedback begin tx: {e}")))?;

        for rec in records {
            tx.execute(
                "INSERT OR REPLACE INTO feedback
                    (session_id, file_path, token_count, composite_score, was_selected, utilization)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    session_id,
                    rec.file_path,
                    rec.token_count as i64,
                    rec.composite_score as f64,
                    rec.was_selected as i64,
                    rec.utilization.map(|u| u as f64),
                ],
            )
            .map_err(|e| OptimError::Feedback(format!("record_feedback insert: {e}")))?;
        }

        tx.commit()
            .map_err(|e| OptimError::Feedback(format!("record_feedback commit: {e}")))?;
        Ok(())
    }

    /// Retrieve all [`FeedbackRecord`]s for a session, ordered by composite score descending.
    ///
    /// # Errors
    ///
    /// Returns [`OptimError::Feedback`] on a database error.
    ///
    /// # Examples
    ///
    /// ```
    /// use ctx_optim::feedback::store::{FeedbackStore, Session};
    ///
    /// let store = FeedbackStore::open_in_memory().unwrap();
    /// let session = Session { id: "s1".to_string(), repo: "/r".to_string(), budget: 1000, created_at: 0 };
    /// store.create_session(&session).unwrap();
    ///
    /// let records = store.get_session_feedback("s1").unwrap();
    /// assert!(records.is_empty());
    /// ```
    pub fn get_session_feedback(
        &self,
        session_id: &str,
    ) -> Result<Vec<FeedbackRecord>, OptimError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT file_path, token_count, composite_score, was_selected, utilization
                 FROM feedback
                 WHERE session_id = ?1
                 ORDER BY composite_score DESC",
            )
            .map_err(|e| OptimError::Feedback(format!("get_session_feedback prepare: {e}")))?;

        let rows = stmt
            .query_map(params![session_id], |row| {
                let utilization: Option<f64> = row.get(4)?;
                Ok(FeedbackRecord {
                    file_path: row.get(0)?,
                    token_count: row.get::<_, i64>(1)? as usize,
                    composite_score: row.get::<_, f64>(2)? as f32,
                    was_selected: row.get::<_, i64>(3)? != 0,
                    utilization: utilization.map(|u| u as f32),
                })
            })
            .map_err(|e| OptimError::Feedback(format!("get_session_feedback query: {e}")))?;

        let mut records = Vec::new();
        for row in rows {
            records.push(
                row.map_err(|e| OptimError::Feedback(format!("get_session_feedback row: {e}")))?,
            );
        }
        Ok(records)
    }

    /// Persist a [`WeightSnapshot`] to the weight history table.
    ///
    /// # Errors
    ///
    /// Returns [`OptimError::Feedback`] on a database error.
    ///
    /// # Examples
    ///
    /// ```
    /// use ctx_optim::feedback::store::{FeedbackStore, WeightSnapshot};
    ///
    /// let store = FeedbackStore::open_in_memory().unwrap();
    /// let snap = WeightSnapshot {
    ///     recency: 0.4, size: 0.2, proximity: 0.3, dependency: 0.1,
    ///     avg_utilization: 0.5, created_at: 0,
    /// };
    /// store.save_weights(&snap).unwrap();
    /// ```
    pub fn save_weights(&self, snap: &WeightSnapshot) -> Result<(), OptimError> {
        self.conn
            .execute(
                "INSERT INTO weight_history
                    (recency, size, proximity, dependency, avg_utilization, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    snap.recency as f64,
                    snap.size as f64,
                    snap.proximity as f64,
                    snap.dependency as f64,
                    snap.avg_utilization as f64,
                    snap.created_at,
                ],
            )
            .map_err(|e| OptimError::Feedback(format!("save_weights: {e}")))?;
        Ok(())
    }

    /// Retrieve the most recently saved [`WeightSnapshot`].
    ///
    /// Returns `None` if the weight history is empty.
    ///
    /// # Errors
    ///
    /// Returns [`OptimError::Feedback`] on a database error.
    ///
    /// # Examples
    ///
    /// ```
    /// use ctx_optim::feedback::store::FeedbackStore;
    ///
    /// let store = FeedbackStore::open_in_memory().unwrap();
    /// assert!(store.latest_weights().unwrap().is_none());
    /// ```
    pub fn latest_weights(&self) -> Result<Option<WeightSnapshot>, OptimError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT recency, size, proximity, dependency, avg_utilization, created_at
                 FROM weight_history
                 ORDER BY created_at DESC
                 LIMIT 1",
            )
            .map_err(|e| OptimError::Feedback(format!("latest_weights prepare: {e}")))?;

        let mut rows = stmt
            .query(params![])
            .map_err(|e| OptimError::Feedback(format!("latest_weights query: {e}")))?;

        if let Some(row) = rows
            .next()
            .map_err(|e| OptimError::Feedback(format!("latest_weights row: {e}")))?
        {
            Ok(Some(WeightSnapshot {
                recency: row
                    .get::<_, f64>(0)
                    .map_err(|e| OptimError::Feedback(format!("latest_weights col recency: {e}")))?
                    as f32,
                size: row
                    .get::<_, f64>(1)
                    .map_err(|e| OptimError::Feedback(format!("latest_weights col size: {e}")))?
                    as f32,
                proximity: row.get::<_, f64>(2).map_err(|e| {
                    OptimError::Feedback(format!("latest_weights col proximity: {e}"))
                })? as f32,
                dependency: row.get::<_, f64>(3).map_err(|e| {
                    OptimError::Feedback(format!("latest_weights col dependency: {e}"))
                })? as f32,
                avg_utilization: row.get::<_, f64>(4).map_err(|e| {
                    OptimError::Feedback(format!("latest_weights col avg_util: {e}"))
                })? as f32,
                created_at: row
                    .get(5)
                    .map_err(|e| OptimError::Feedback(format!("latest_weights col ts: {e}")))?,
            }))
        } else {
            Ok(None)
        }
    }

    /// Return the total number of sessions stored.
    ///
    /// # Errors
    ///
    /// Returns [`OptimError::Feedback`] on a database error.
    ///
    /// # Examples
    ///
    /// ```
    /// use ctx_optim::feedback::store::FeedbackStore;
    ///
    /// let store = FeedbackStore::open_in_memory().unwrap();
    /// assert_eq!(store.session_count().unwrap(), 0);
    /// ```
    pub fn session_count(&self) -> Result<usize, OptimError> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM sessions", params![], |row| row.get(0))
            .map_err(|e| OptimError::Feedback(format!("session_count: {e}")))?;
        Ok(count as usize)
    }

    /// Return all [`FeedbackRecord`]s that have a non-NULL utilization value.
    ///
    /// Used by the learning algorithm to build the training set.
    ///
    /// # Errors
    ///
    /// Returns [`OptimError::Feedback`] on a database error.
    ///
    /// # Examples
    ///
    /// ```
    /// use ctx_optim::feedback::store::FeedbackStore;
    ///
    /// let store = FeedbackStore::open_in_memory().unwrap();
    /// assert!(store.all_feedback_with_utilization().unwrap().is_empty());
    /// ```
    pub fn all_feedback_with_utilization(&self) -> Result<Vec<FeedbackRecord>, OptimError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT file_path, token_count, composite_score, was_selected, utilization
                 FROM feedback
                 WHERE utilization IS NOT NULL",
            )
            .map_err(|e| {
                OptimError::Feedback(format!("all_feedback_with_utilization prepare: {e}"))
            })?;

        let rows = stmt
            .query_map(params![], |row| {
                let utilization: Option<f64> = row.get(4)?;
                Ok(FeedbackRecord {
                    file_path: row.get(0)?,
                    token_count: row.get::<_, i64>(1)? as usize,
                    composite_score: row.get::<_, f64>(2)? as f32,
                    was_selected: row.get::<_, i64>(3)? != 0,
                    utilization: utilization.map(|u| u as f32),
                })
            })
            .map_err(|e| {
                OptimError::Feedback(format!("all_feedback_with_utilization query: {e}"))
            })?;

        let mut records = Vec::new();
        for row in rows {
            records.push(row.map_err(|e| {
                OptimError::Feedback(format!("all_feedback_with_utilization row: {e}"))
            })?);
        }
        Ok(records)
    }

    /// Return the total number of feedback records stored across all sessions.
    ///
    /// # Errors
    ///
    /// Returns [`OptimError::Feedback`] on a database error.
    ///
    /// # Examples
    ///
    /// ```
    /// use ctx_optim::feedback::store::FeedbackStore;
    ///
    /// let store = FeedbackStore::open_in_memory().unwrap();
    /// assert_eq!(store.feedback_record_count().unwrap(), 0);
    /// ```
    pub fn feedback_record_count(&self) -> Result<usize, OptimError> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM feedback", params![], |row| row.get(0))
            .map_err(|e| OptimError::Feedback(format!("feedback_record_count: {e}")))?;
        Ok(count as usize)
    }

    /// Return the average utilization across all records that have a non-NULL value.
    ///
    /// Returns `None` if no records have utilization data.
    ///
    /// # Errors
    ///
    /// Returns [`OptimError::Feedback`] on a database error.
    ///
    /// # Examples
    ///
    /// ```
    /// use ctx_optim::feedback::store::FeedbackStore;
    ///
    /// let store = FeedbackStore::open_in_memory().unwrap();
    /// assert!(store.avg_utilization().unwrap().is_none());
    /// ```
    pub fn avg_utilization(&self) -> Result<Option<f32>, OptimError> {
        let result: Option<f64> = self
            .conn
            .query_row(
                "SELECT AVG(utilization) FROM feedback WHERE utilization IS NOT NULL",
                params![],
                |row| row.get(0),
            )
            .map_err(|e| OptimError::Feedback(format!("avg_utilization: {e}")))?;
        Ok(result.map(|v| v as f32))
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_session(id: &str) -> Session {
        Session {
            id: id.to_string(),
            repo: "/test/repo".to_string(),
            budget: 128_000,
            created_at: 1_700_000_000,
        }
    }

    fn make_record(path: &str, score: f32, selected: bool, util: Option<f32>) -> FeedbackRecord {
        FeedbackRecord {
            file_path: path.to_string(),
            token_count: 256,
            composite_score: score,
            was_selected: selected,
            utilization: util,
        }
    }

    #[test]
    fn test_open_creates_schema() {
        let store = FeedbackStore::open_in_memory().unwrap();
        // Verify all three user-defined tables exist in sqlite_master.
        // Exclude sqlite_sequence, which SQLite creates automatically for
        // AUTOINCREMENT columns.
        let tables: Vec<String> = {
            let mut stmt = store
                .conn
                .prepare(
                    "SELECT name FROM sqlite_master \
                     WHERE type='table' AND name NOT LIKE 'sqlite_%' \
                     ORDER BY name",
                )
                .unwrap();
            stmt.query_map(params![], |row| row.get(0))
                .unwrap()
                .map(|r| r.unwrap())
                .collect()
        };
        assert!(
            tables.contains(&"sessions".to_string()),
            "missing sessions table: {tables:?}"
        );
        assert!(
            tables.contains(&"feedback".to_string()),
            "missing feedback table: {tables:?}"
        );
        assert!(
            tables.contains(&"weight_history".to_string()),
            "missing weight_history table: {tables:?}"
        );
        assert_eq!(tables.len(), 3);
    }

    #[test]
    fn test_create_and_get_session() {
        let store = FeedbackStore::open_in_memory().unwrap();
        let session = make_session("sess-001");
        store.create_session(&session).unwrap();

        let retrieved = store
            .get_session("sess-001")
            .unwrap()
            .expect("should exist");
        assert_eq!(retrieved.id, "sess-001");
        assert_eq!(retrieved.repo, "/test/repo");
        assert_eq!(retrieved.budget, 128_000);
        assert_eq!(retrieved.created_at, 1_700_000_000);
    }

    #[test]
    fn test_get_session_not_found() {
        let store = FeedbackStore::open_in_memory().unwrap();
        let result = store.get_session("nonexistent").unwrap();
        assert!(result.is_none(), "expected None for missing session");
    }

    #[test]
    fn test_record_and_get_feedback() {
        let store = FeedbackStore::open_in_memory().unwrap();
        store.create_session(&make_session("s1")).unwrap();

        let records = vec![
            make_record("src/main.rs", 0.9, true, Some(0.75)),
            make_record("src/lib.rs", 0.5, false, None),
            make_record("tests/integration.rs", 0.7, true, Some(0.4)),
        ];
        store.record_feedback("s1", &records).unwrap();

        let fetched = store.get_session_feedback("s1").unwrap();
        assert_eq!(fetched.len(), 3);

        // Should be ordered by composite_score DESC: 0.9, 0.7, 0.5
        assert!(
            (fetched[0].composite_score - 0.9).abs() < 1e-4,
            "first score should be 0.9"
        );
        assert!(
            (fetched[1].composite_score - 0.7).abs() < 1e-4,
            "second score should be 0.7"
        );
        assert!(
            (fetched[2].composite_score - 0.5).abs() < 1e-4,
            "third score should be 0.5"
        );

        // Check utilization round-trip
        assert!(
            fetched[0]
                .utilization
                .is_some_and(|u| (u - 0.75).abs() < 1e-4),
            "utilization mismatch for first record"
        );
        assert!(
            fetched[2].utilization.is_none(),
            "lib.rs should have no utilization"
        );

        // Check was_selected
        assert!(fetched[0].was_selected);
        assert!(!fetched[2].was_selected);
    }

    #[test]
    fn test_save_and_get_latest_weights() {
        let store = FeedbackStore::open_in_memory().unwrap();

        let snap = WeightSnapshot {
            recency: 0.4,
            size: 0.2,
            proximity: 0.3,
            dependency: 0.1,
            avg_utilization: 0.55,
            created_at: 1_700_001_000,
        };
        store.save_weights(&snap).unwrap();

        let latest = store
            .latest_weights()
            .unwrap()
            .expect("should have weights");
        assert!((latest.recency - 0.4).abs() < 1e-4);
        assert!((latest.size - 0.2).abs() < 1e-4);
        assert!((latest.proximity - 0.3).abs() < 1e-4);
        assert!((latest.dependency - 0.1).abs() < 1e-4);
        assert!((latest.avg_utilization - 0.55).abs() < 1e-4);
        assert_eq!(latest.created_at, 1_700_001_000);
    }

    #[test]
    fn test_latest_weights_none_when_empty() {
        let store = FeedbackStore::open_in_memory().unwrap();
        let result = store.latest_weights().unwrap();
        assert!(
            result.is_none(),
            "expected None when weight_history is empty"
        );
    }

    #[test]
    fn test_session_count() {
        let store = FeedbackStore::open_in_memory().unwrap();
        assert_eq!(store.session_count().unwrap(), 0);

        store.create_session(&make_session("s1")).unwrap();
        store.create_session(&make_session("s2")).unwrap();
        store.create_session(&make_session("s3")).unwrap();

        assert_eq!(store.session_count().unwrap(), 3);
    }

    #[test]
    fn test_all_feedback_with_utilization() {
        let store = FeedbackStore::open_in_memory().unwrap();
        store.create_session(&make_session("s1")).unwrap();

        let records = vec![
            make_record("a.rs", 0.8, true, Some(0.6)),
            make_record("b.rs", 0.6, true, None),
            make_record("c.rs", 0.4, false, Some(0.2)),
        ];
        store.record_feedback("s1", &records).unwrap();

        let with_util = store.all_feedback_with_utilization().unwrap();
        assert_eq!(
            with_util.len(),
            2,
            "only records with utilization should be returned"
        );

        let paths: Vec<&str> = with_util.iter().map(|r| r.file_path.as_str()).collect();
        assert!(paths.contains(&"a.rs"));
        assert!(paths.contains(&"c.rs"));
        assert!(!paths.contains(&"b.rs"), "b.rs has no utilization");
    }
}
