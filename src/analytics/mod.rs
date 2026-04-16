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
                darkbloom_memory_gb REAL DEFAULT 0,
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

            -- Earnings snapshots (periodic tracking)
            CREATE TABLE IF NOT EXISTS earnings_snapshots (
                id INTEGER PRIMARY KEY,
                timestamp TEXT NOT NULL,
                total_usd REAL DEFAULT 0,
                today_usd REAL DEFAULT 0,
                pending_usd REAL DEFAULT 0,
                requests_served INTEGER DEFAULT 0,
                session_earnings_usd REAL DEFAULT 0
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
        darkbloom_memory_gb: f64,
        system_memory_available_gb: f64,
    ) -> Result<()> {
        let timestamp = Utc::now().to_rfc3339();
        let state_str = format!("{:?}", state);
        let models_json = serde_json::to_string(omlx_loaded_models)?;

        self.conn.execute(
            "INSERT INTO snapshots (timestamp, state, omlx_loaded_models, omlx_memory_gb, darkbloom_connected, darkbloom_memory_gb, system_memory_available_gb) 
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![timestamp, state_str, models_json, omlx_memory_gb, darkbloom_connected as i32, darkbloom_memory_gb, system_memory_available_gb],
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

/// A memory snapshot for time-series charts
#[derive(Debug, Clone, serde::Serialize)]
pub struct MemorySnapshot {
    pub timestamp: String,
    pub state: String,
    pub omlx_memory_gb: f64,
    pub darkbloom_memory_gb: f64,
    pub system_available_gb: f64,
}

impl Store {
    /// Get memory history for charting
    pub fn get_memory_history(&self, hours: u32) -> Result<Vec<MemorySnapshot>> {
        let since = (Utc::now() - Duration::hours(hours as i64)).to_rfc3339();

        let mut stmt = self.conn.prepare(
            "SELECT timestamp, state, omlx_memory_gb, COALESCE(darkbloom_memory_gb, 0), system_memory_available_gb 
             FROM snapshots 
             WHERE timestamp > ?1 
             ORDER BY timestamp ASC",
        )?;

        let snapshots = stmt
            .query_map([&since], |row| {
                Ok(MemorySnapshot {
                    timestamp: row.get(0)?,
                    state: row.get(1)?,
                    omlx_memory_gb: row.get(2)?,
                    darkbloom_memory_gb: row.get(3)?,
                    system_available_gb: row.get(4)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(snapshots)
    }

    /// Get state timeline for activity chart
    pub fn get_state_timeline(&self, hours: u32) -> Result<Vec<StateTimelineEntry>> {
        let since = (Utc::now() - Duration::hours(hours as i64)).to_rfc3339();

        let mut stmt = self.conn.prepare(
            "SELECT timestamp, state FROM snapshots 
             WHERE timestamp > ?1 
             ORDER BY timestamp ASC",
        )?;

        let entries = stmt
            .query_map([&since], |row| {
                Ok(StateTimelineEntry {
                    timestamp: row.get(0)?,
                    state: row.get(1)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(entries)
    }
}

/// State timeline entry for activity visualization
#[derive(Debug, Clone, serde::Serialize)]
pub struct StateTimelineEntry {
    pub timestamp: String,
    pub state: String,
}

/// Earnings snapshot for tracking over time
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EarningsSnapshot {
    pub timestamp: String,
    pub total_usd: f64,
    pub today_usd: f64,
    pub pending_usd: f64,
    pub requests_served: u64,
    pub session_earnings_usd: f64,
}

/// Earnings summary
#[derive(Debug, Clone, serde::Serialize)]
pub struct EarningsSummary {
    pub total_usd: f64,
    pub today_usd: f64,
    pub this_week_usd: f64,
    pub this_month_usd: f64,
    pub pending_usd: f64,
    pub total_requests: u64,
    pub avg_per_request_usd: f64,
    pub estimated_hourly_usd: f64,
}

impl Store {
    /// Record an earnings snapshot
    pub fn record_earnings_snapshot(
        &self,
        total_usd: f64,
        today_usd: f64,
        pending_usd: f64,
        requests_served: u64,
        session_earnings_usd: f64,
    ) -> Result<()> {
        let timestamp = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO earnings_snapshots (timestamp, total_usd, today_usd, pending_usd, requests_served, session_earnings_usd) 
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![timestamp, total_usd, today_usd, pending_usd, requests_served as i64, session_earnings_usd],
        )?;
        Ok(())
    }

    /// Get earnings history for charting
    pub fn get_earnings_history(&self, hours: u32) -> Result<Vec<EarningsSnapshot>> {
        let since = (Utc::now() - Duration::hours(hours as i64)).to_rfc3339();

        let mut stmt = self.conn.prepare(
            "SELECT timestamp, total_usd, today_usd, pending_usd, requests_served, session_earnings_usd 
             FROM earnings_snapshots 
             WHERE timestamp > ?1 
             ORDER BY timestamp ASC"
        )?;

        let snapshots = stmt
            .query_map([&since], |row| {
                Ok(EarningsSnapshot {
                    timestamp: row.get(0)?,
                    total_usd: row.get(1)?,
                    today_usd: row.get(2)?,
                    pending_usd: row.get(3)?,
                    requests_served: row.get::<_, i64>(4)? as u64,
                    session_earnings_usd: row.get(5)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(snapshots)
    }

    /// Get earnings summary
    pub fn get_earnings_summary(&self) -> Result<EarningsSummary> {
        // Get latest snapshot
        let latest: Option<(f64, f64, f64, i64)> = self
            .conn
            .query_row(
                "SELECT total_usd, today_usd, pending_usd, requests_served 
             FROM earnings_snapshots 
             ORDER BY timestamp DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .ok();

        let (total_usd, today_usd, pending_usd, total_requests) =
            latest.unwrap_or((0.0, 0.0, 0.0, 0));

        // Calculate this week's earnings (sum of session_earnings in last 7 days)
        let week_ago = (Utc::now() - Duration::days(7)).to_rfc3339();
        let this_week_usd: f64 = self.conn.query_row(
            "SELECT COALESCE(SUM(session_earnings_usd), 0) FROM earnings_snapshots WHERE timestamp > ?1",
            [&week_ago],
            |row| row.get(0),
        ).unwrap_or(0.0);

        // Calculate this month's earnings
        let month_ago = (Utc::now() - Duration::days(30)).to_rfc3339();
        let this_month_usd: f64 = self.conn.query_row(
            "SELECT COALESCE(SUM(session_earnings_usd), 0) FROM earnings_snapshots WHERE timestamp > ?1",
            [&month_ago],
            |row| row.get(0),
        ).unwrap_or(0.0);

        // Calculate average per request
        let avg_per_request = if total_requests > 0 {
            total_usd / total_requests as f64
        } else {
            0.0
        };

        // Estimate hourly rate based on last 24 hours of Darkbloom activity
        let day_ago = (Utc::now() - Duration::hours(24)).to_rfc3339();
        let (day_earnings, darkbloom_hours): (f64, f64) = self.conn.query_row(
            "SELECT 
                COALESCE(SUM(e.session_earnings_usd), 0),
                COALESCE(COUNT(DISTINCT s.id) * 1.0 / 60, 0)
             FROM earnings_snapshots e
             LEFT JOIN snapshots s ON s.timestamp > ?1 AND s.state IN ('DarkbloomActive', 'DARKBLOOM_ACTIVE')
             WHERE e.timestamp > ?1",
            [&day_ago],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap_or((0.0, 0.0));

        let estimated_hourly = if darkbloom_hours > 0.0 {
            day_earnings / darkbloom_hours
        } else {
            0.0
        };

        Ok(EarningsSummary {
            total_usd,
            today_usd,
            this_week_usd,
            this_month_usd,
            pending_usd,
            total_requests: total_requests as u64,
            avg_per_request_usd: avg_per_request,
            estimated_hourly_usd: estimated_hourly,
        })
    }

    /// Start a new Darkbloom session
    pub fn start_darkbloom_session(&self, model: &str) -> Result<i64> {
        let timestamp = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO darkbloom_sessions (start_time, model) VALUES (?1, ?2)",
            params![timestamp, model],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// End a Darkbloom session
    pub fn end_darkbloom_session(
        &self,
        session_id: i64,
        requests_served: u64,
        earnings_usd: f64,
    ) -> Result<()> {
        let timestamp = Utc::now().to_rfc3339();
        self.conn.execute(
            "UPDATE darkbloom_sessions SET end_time = ?1, requests_served = ?2, earnings_usd = ?3 WHERE id = ?4",
            params![timestamp, requests_served as i64, earnings_usd, session_id],
        )?;
        Ok(())
    }

    /// Get recent Darkbloom sessions
    pub fn get_recent_sessions(&self, limit: usize) -> Result<Vec<DarkbloomSession>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, start_time, end_time, model, requests_served, earnings_usd 
             FROM darkbloom_sessions 
             ORDER BY start_time DESC 
             LIMIT ?1",
        )?;

        let sessions = stmt
            .query_map([limit as i64], |row| {
                Ok(DarkbloomSession {
                    id: row.get(0)?,
                    start_time: row.get(1)?,
                    end_time: row.get(2)?,
                    model: row.get(3)?,
                    requests_served: row.get::<_, i64>(4)? as u64,
                    earnings_usd: row.get(5)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(sessions)
    }
}

/// A Darkbloom session record
#[derive(Debug, Clone, serde::Serialize)]
pub struct DarkbloomSession {
    pub id: i64,
    pub start_time: String,
    pub end_time: Option<String>,
    pub model: Option<String>,
    pub requests_served: u64,
    pub earnings_usd: f64,
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
