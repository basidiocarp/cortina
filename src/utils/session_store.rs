use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;

const SESSIONS_DB_FILENAME: &str = "cortina-sessions.db";
const SESSIONS_DB_ENV_VAR: &str = "CORTINA_SESSIONS_DB_PATH";
#[allow(dead_code)]
const SESSION_ORPHAN_THRESHOLD_HOURS: i64 = 24;

pub struct SessionStore {
    conn: Connection,
}

impl SessionStore {
    /// Open a `SQLite` store for session state. Creates the database and schema if needed.
    ///
    /// # Errors
    ///
    /// Returns an error if the database cannot be opened or schema creation fails.
    pub fn open() -> Result<Self> {
        let db_path =
            spore::paths::db_path("cortina", SESSIONS_DB_FILENAME, SESSIONS_DB_ENV_VAR, None)
                .context("resolve cortina sessions database path")?;

        Self::open_at(&db_path)
    }

    /// Open a `SQLite` store at a specific path. Creates the database and schema if needed.
    ///
    /// # Errors
    ///
    /// Returns an error if the database cannot be opened or schema creation fails.
    pub(crate) fn open_at(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("open cortina sessions database at {}", path.display()))?;

        conn.execute_batch("PRAGMA journal_mode=WAL;")
            .context("set WAL mode for sessions database")?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS sessions (
                session_id   TEXT PRIMARY KEY,
                project      TEXT NOT NULL,
                worktree_id  TEXT,
                status       TEXT NOT NULL DEFAULT 'active',
                created_at   INTEGER NOT NULL,
                last_seen_at INTEGER NOT NULL
            );",
        )
        .context("create sessions table")?;

        Ok(Self { conn })
    }

    /// Insert a new active session.
    ///
    /// # Errors
    ///
    /// Returns an error if the database write fails.
    pub fn create(&self, session_id: &str, project: &str, worktree_id: Option<&str>) -> Result<()> {
        let now = now_ms();
        self.conn
            .execute(
                "INSERT INTO sessions (session_id, project, worktree_id, status, created_at, last_seen_at)
                 VALUES (?1, ?2, ?3, 'active', ?4, ?4)",
                params![session_id, project, worktree_id, now],
            )
            .context("insert session into database")?;
        Ok(())
    }

    /// Mark a session ended cleanly (hyphae write succeeded).
    ///
    /// # Errors
    ///
    /// Returns an error if the database write fails.
    pub fn end_clean(&self, session_id: &str) -> Result<()> {
        self.conn
            .execute(
                "UPDATE sessions SET status = 'ended', last_seen_at = ?1 WHERE session_id = ?2",
                params![now_ms(), session_id],
            )
            .context("update session status to ended")?;
        Ok(())
    }

    /// Mark a session orphaned (hyphae write failed, crash, etc.).
    ///
    /// # Errors
    ///
    /// Returns an error if the database write fails.
    pub fn end_orphaned(&self, session_id: &str) -> Result<()> {
        self.conn
            .execute(
                "UPDATE sessions SET status = 'orphaned', last_seen_at = ?1 WHERE session_id = ?2",
                params![now_ms(), session_id],
            )
            .context("update session status to orphaned")?;
        Ok(())
    }

    /// Update heartbeat so long-running sessions don't auto-expire.
    ///
    /// # Errors
    ///
    /// Returns an error if the database write fails.
    #[allow(dead_code)]
    pub fn heartbeat(&self, session_id: &str) -> Result<()> {
        self.conn
            .execute(
                "UPDATE sessions SET last_seen_at = ?1 WHERE session_id = ?2 AND status = 'active'",
                params![now_ms(), session_id],
            )
            .context("update session heartbeat")?;
        Ok(())
    }

    /// Find an active session for the given project/worktree, or None.
    ///
    /// Sessions older than `SESSION_ORPHAN_THRESHOLD_HOURS` are treated as orphaned
    /// and marked as such before the query.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    #[allow(dead_code)]
    pub fn find_active(&self, project: &str, worktree_id: Option<&str>) -> Result<Option<String>> {
        let threshold_ms = now_ms() - SESSION_ORPHAN_THRESHOLD_HOURS * 3_600_000;

        // Mark stale 'active' sessions as orphaned first
        self.conn
            .execute(
                "UPDATE sessions SET status = 'orphaned'
                 WHERE status = 'active' AND project = ?1 AND last_seen_at < ?2",
                params![project, threshold_ms],
            )
            .context("mark stale sessions as orphaned")?;

        // Find current active session
        let session_id: Option<String> = self
            .conn
            .query_row(
                "SELECT session_id FROM sessions
                 WHERE status = 'active' AND project = ?1 AND (worktree_id = ?2 OR (?2 IS NULL AND worktree_id IS NULL))
                 ORDER BY created_at DESC LIMIT 1",
                params![project, worktree_id],
                |row| row.get(0),
            )
            .optional()
            .context("query active session")?;

        Ok(session_id)
    }
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    #[allow(clippy::cast_possible_truncation)]
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    ms
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_create_and_find_active_session() -> Result<()> {
        let tmp = tempdir()?;
        let db_path = tmp.path().join("test.db");
        let store = SessionStore::open_at(&db_path)?;
        store.create("sess-1", "myproject", Some("wt-123"))?;

        let found = store.find_active("myproject", Some("wt-123"))?;
        assert_eq!(found, Some("sess-1".to_string()));

        Ok(())
    }

    #[test]
    fn test_end_clean_marks_session_ended() -> Result<()> {
        let tmp = tempdir()?;
        let db_path = tmp.path().join("test.db");
        let store = SessionStore::open_at(&db_path)?;
        store.create("sess-1", "myproject", Some("wt-123"))?;

        store.end_clean("sess-1")?;

        let found = store.find_active("myproject", Some("wt-123"))?;
        assert_eq!(found, None);

        Ok(())
    }

    #[test]
    fn test_end_orphaned_marks_session_orphaned() -> Result<()> {
        let tmp = tempdir()?;
        let db_path = tmp.path().join("test.db");
        let store = SessionStore::open_at(&db_path)?;
        store.create("sess-1", "myproject", Some("wt-123"))?;

        store.end_orphaned("sess-1")?;

        let found = store.find_active("myproject", Some("wt-123"))?;
        assert_eq!(found, None);

        Ok(())
    }

    #[test]
    fn test_heartbeat_keeps_session_active() -> Result<()> {
        let tmp = tempdir()?;
        let db_path = tmp.path().join("test.db");
        let store = SessionStore::open_at(&db_path)?;
        store.create("sess-1", "myproject", Some("wt-123"))?;

        // Heartbeat should keep it active
        store.heartbeat("sess-1")?;

        let found = store.find_active("myproject", Some("wt-123"))?;
        assert_eq!(found, Some("sess-1".to_string()));

        Ok(())
    }

    #[test]
    fn test_find_active_with_null_worktree() -> Result<()> {
        let tmp = tempdir()?;
        let db_path = tmp.path().join("test.db");
        let store = SessionStore::open_at(&db_path)?;
        store.create("sess-1", "myproject", None)?;

        let found = store.find_active("myproject", None)?;
        assert_eq!(found, Some("sess-1".to_string()));

        Ok(())
    }

    #[test]
    fn test_find_active_returns_none_for_different_project() -> Result<()> {
        let tmp = tempdir()?;
        let db_path = tmp.path().join("test.db");
        let store = SessionStore::open_at(&db_path)?;
        store.create("sess-1", "project-a", Some("wt-123"))?;

        let found = store.find_active("project-b", Some("wt-123"))?;
        assert_eq!(found, None);

        Ok(())
    }

    #[test]
    fn test_multiple_sessions_isolated() -> Result<()> {
        let tmp = tempdir()?;
        let db_path = tmp.path().join("test.db");
        let store = SessionStore::open_at(&db_path)?;

        // Create two active sessions for the same project, different worktrees
        store.create("sess-1", "myproject", Some("wt-123"))?;
        store.create("sess-2", "myproject", Some("wt-456"))?;

        // Each should find its own session
        let found1 = store.find_active("myproject", Some("wt-123"))?;
        assert_eq!(found1, Some("sess-1".to_string()));

        let found2 = store.find_active("myproject", Some("wt-456"))?;
        assert_eq!(found2, Some("sess-2".to_string()));

        // End one cleanly; the other should still be findable
        store.end_clean("sess-1")?;
        let found1_after = store.find_active("myproject", Some("wt-123"))?;
        assert_eq!(found1_after, None);

        let found2_after = store.find_active("myproject", Some("wt-456"))?;
        assert_eq!(found2_after, Some("sess-2".to_string()));

        Ok(())
    }

    #[test]
    fn test_latest_created_session_is_returned() -> Result<()> {
        let tmp = tempdir()?;
        let db_path = tmp.path().join("test.db");
        let store = SessionStore::open_at(&db_path)?;

        // Create first session, end it
        store.create("sess-1", "myproject", None)?;
        store.end_clean("sess-1")?;

        // Create second session for the same project/worktree
        store.create("sess-2", "myproject", None)?;

        // Should find the new active session
        let found = store.find_active("myproject", None)?;
        assert_eq!(found, Some("sess-2".to_string()));

        Ok(())
    }

    #[test]
    fn test_heartbeat_on_nonexistent_session_does_not_error() -> Result<()> {
        let tmp = tempdir()?;
        let db_path = tmp.path().join("test.db");
        let store = SessionStore::open_at(&db_path)?;

        // Heartbeat on a session that doesn't exist should succeed (no rows updated)
        store.heartbeat("nonexistent-sess")?;

        Ok(())
    }

    #[test]
    fn test_heartbeat_does_not_revive_ended_session() -> Result<()> {
        let tmp = tempdir()?;
        let db_path = tmp.path().join("test.db");
        let store = SessionStore::open_at(&db_path)?;

        store.create("sess-1", "myproject", Some("wt-123"))?;
        store.end_clean("sess-1")?;

        // Heartbeat on ended session should not revive it
        store.heartbeat("sess-1")?;

        let found = store.find_active("myproject", Some("wt-123"))?;
        assert_eq!(found, None);

        Ok(())
    }

    #[test]
    fn test_end_clean_idempotent() -> Result<()> {
        let tmp = tempdir()?;
        let db_path = tmp.path().join("test.db");
        let store = SessionStore::open_at(&db_path)?;

        store.create("sess-1", "myproject", Some("wt-123"))?;
        store.end_clean("sess-1")?;

        // Second end_clean should succeed without error
        store.end_clean("sess-1")?;

        let found = store.find_active("myproject", Some("wt-123"))?;
        assert_eq!(found, None);

        Ok(())
    }
}
