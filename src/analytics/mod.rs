//! Analytics storage and aggregation

use anyhow::Result;
use chrono::{Duration, Utc};
use rusqlite::{params, Connection};
use std::path::Path;
use tracing::debug;

use crate::config::Config;
use crate::{AnalyticsSummary, SystemState, TimePeriod};

/// SQLite-backed analytics store
pub struct Store {
    conn: Connection,
}

impl Store {
    /// Open or create the analytics database
    pub fn open(config: &Config) -> Result<Self> {
        let db_path = config.database_path()?;
        Self::open_path(&db_path)
    }

    /// Open database at a specific path
    pub fn open_path(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        let store = Self { conn };
        store.initialize()?;
        Ok(store)
    }

    /// Initialize database schema
    fn initialize(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            -- State transitions
            CREATE TABLE IF NOT EXISTS transitions (
                id INTEGER PRIMARY KEY,
                timestamp TEXT NOT NULL,
                from_state TEXT NOT NULL,
                to_state TEXT NOT NULL,
                trigger TEXT NOT NULL,
                duration_ms INTEGER,
                success INTEGER NOT NULL
            );

            -- Periodic snapshots
            CREATE TABLE IF NOT EXISTS snapshots (
                id INTEGER PRIMARY KEY,
                timestamp TEXT NOT NULL,
                state TEXT NOT NULL,
                omlx_loaded_models TEXT,
                omlx_memory_gb REAL,
                darkbloom_connected INTEGER,
                system_memory_available_gb REAL
            );

            -- OMLX request tracking (sampled)
            CREATE TABLE IF NOT EXISTS omlx_requests (
                id INTEGER PRIMARY KEY,
                timestamp TEXT NOT NULL,
                model TEXT,
                tokens_in INTEGER,
                tokens_out INTEGER,
                duration_ms INTEGER
            );

            -- Darkbloom sessions
            CREATE TABLE IF NOT EXISTS darkbloom_sessions (
                id INTEGER PRIMARY KEY,
                start_time TEXT NOT NULL,
                end_time TEXT,
                model TEXT,
                requests_served INTEGER DEFAULT 0,
                earnings_usd REAL DEFAULT 0
            );

            -- Indexes
            CREATE INDEX IF NOT EXISTS idx_transitions_timestamp ON transitions(timestamp);
            CREATE INDEX IF NOT EXISTS idx_snapshots_timestamp ON snapshots(timestamp);
            CREATE INDEX IF NOT EXISTS idx_omlx_requests_timestamp ON omlx_requests(timestamp);
            "#,
        )?;

        Ok(())
    }

    /// Record a state transition
    pub fn record_transition(
        &self,
        from_state: &str,
        to_state: &str,
        trigger: &str,
        duration_ms: u64,
        success: bool,
    ) -> Result<()> {
        let timestamp = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO transitions (timestamp, from_state, to_state, trigger, duration_ms, success) 
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![timestamp, from_state, to_state, trigger, duration_ms, success as i32],
        )?;
        debug!(
            "Recorded transition: {} -> {} ({})",
            from_state, to_state, trigger
        );
        Ok(())
    }

    /// Record a periodic snapshot
    pub fn record_snapshot(
        &self,
        state: SystemState,
        omlx_loaded_models: &[String],
        omlx_memory_gb: f64,
        darkbloom_connected: bool,
        system_memory_available_gb: f64,
    ) -> Result<()> {
        let timestamp = Utc::now().to_rfc3339();
        let state_str = format!("{:?}", state);
        let models_json = serde_json::to_string(omlx_loaded_models)?;

        self.conn.execute(
            "INSERT INTO snapshots (timestamp, state, omlx_loaded_models, omlx_memory_gb, darkbloom_connected, system_memory_available_gb) 
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![timestamp, state_str, models_json, omlx_memory_gb, darkbloom_connected as i32, system_memory_available_gb],
        )?;
        Ok(())
    }

    /// Get analytics summary for a time period
    pub fn get_summary(&self, period: TimePeriod) -> Result<AnalyticsSummary> {
        let duration = match period {
            TimePeriod::Hour => Duration::hours(1),
            TimePeriod::Day => Duration::days(1),
            TimePeriod::Week => Duration::weeks(1),
            TimePeriod::Month => Duration::days(30),
        };

        let since = (Utc::now() - duration).to_rfc3339();

        // Count snapshots by state
        let mut stmt = self
            .conn
            .prepare("SELECT state, COUNT(*) FROM snapshots WHERE timestamp > ?1 GROUP BY state")?;

        let state_counts: std::collections::HashMap<String, i64> = stmt
            .query_map([&since], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })?
            .filter_map(|r| r.ok())
            .collect();

        let total_snapshots: i64 = state_counts.values().sum();
        let total_snapshots = total_snapshots.max(1) as f64;

        let omlx_active = *state_counts.get("OmlxActive").unwrap_or(&0) as f64;
        let darkbloom_active = *state_counts.get("DarkbloomActive").unwrap_or(&0) as f64;
        let omlx_idle = *state_counts.get("OmlxIdle").unwrap_or(&0) as f64;
        let transitioning = (total_snapshots - omlx_active - darkbloom_active - omlx_idle).max(0.0);

        // Count transitions
        let transitions_count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM transitions WHERE timestamp > ?1 AND success = 1",
                [&since],
                |row| row.get(0),
            )
            .unwrap_or(0);

        // Average transition duration
        let avg_duration: f64 = self.conn.query_row(
            "SELECT COALESCE(AVG(duration_ms), 0) FROM transitions WHERE timestamp > ?1 AND success = 1",
            [&since],
            |row| row.get(0),
        ).unwrap_or(0.0);

        // Memory stats
        let (peak_memory, avg_memory): (f64, f64) = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(omlx_memory_gb), 0), COALESCE(AVG(omlx_memory_gb), 0) 
             FROM snapshots WHERE timestamp > ?1",
                [&since],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap_or((0.0, 0.0));

        Ok(AnalyticsSummary {
            period,
            omlx_active_pct: (omlx_active / total_snapshots) * 100.0,
            darkbloom_active_pct: (darkbloom_active / total_snapshots) * 100.0,
            idle_pct: (omlx_idle / total_snapshots) * 100.0,
            transitioning_pct: (transitioning / total_snapshots) * 100.0,
            omlx_requests: 0,             // TODO: implement request tracking
            darkbloom_requests_served: 0, // TODO: pull from Darkbloom API
            darkbloom_earnings_usd: 0.0,  // TODO: pull from Darkbloom API
            transitions_count: transitions_count as u32,
            avg_transition_duration_ms: avg_duration as u64,
            peak_memory_gb: peak_memory,
            avg_memory_gb: avg_memory,
        })
    }

    /// Get recent transitions
    pub fn get_recent_transitions(&self, limit: usize) -> Result<Vec<TransitionRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT timestamp, from_state, to_state, trigger, duration_ms, success 
             FROM transitions ORDER BY timestamp DESC LIMIT ?1",
        )?;

        let records = stmt
            .query_map([limit as i64], |row| {
                Ok(TransitionRecord {
                    timestamp: row.get(0)?,
                    from_state: row.get(1)?,
                    to_state: row.get(2)?,
                    trigger: row.get(3)?,
                    duration_ms: row.get(4)?,
                    success: row.get::<_, i32>(5)? != 0,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(records)
    }

    /// Cleanup old data based on retention policy
    pub fn cleanup(&self, retention_days: u32) -> Result<usize> {
        let cutoff = (Utc::now() - Duration::days(retention_days as i64)).to_rfc3339();

        let mut deleted = 0;
        deleted += self
            .conn
            .execute("DELETE FROM snapshots WHERE timestamp < ?1", [&cutoff])?;
        deleted += self
            .conn
            .execute("DELETE FROM transitions WHERE timestamp < ?1", [&cutoff])?;
        deleted += self
            .conn
            .execute("DELETE FROM omlx_requests WHERE timestamp < ?1", [&cutoff])?;

        debug!("Cleaned up {} old records", deleted);
        Ok(deleted)
    }
}

/// A recorded transition
#[derive(Debug, Clone, serde::Serialize)]
pub struct TransitionRecord {
    pub timestamp: String,
    pub from_state: String,
    pub to_state: String,
    pub trigger: String,
    pub duration_ms: i64,
    pub success: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn test_store_creation() {
        let tmp = NamedTempFile::new().unwrap();
        let store = Store::open_path(tmp.path()).unwrap();

        // Should be able to record and query
        store
            .record_transition("OMLX", "DARKBLOOM", "idle", 1000, true)
            .unwrap();
        let summary = store.get_summary(TimePeriod::Hour).unwrap();
        assert_eq!(summary.transitions_count, 1);
    }
}
