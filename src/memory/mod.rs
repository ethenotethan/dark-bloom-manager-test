//! System memory monitoring (macOS)

use anyhow::Result;
use sysinfo::System;

/// Memory information
#[derive(Debug, Clone)]
pub struct MemoryInfo {
    pub total_gb: f64,
    pub available_gb: f64,
    pub used_gb: f64,
}

/// Get current system memory information
pub fn get_memory_info() -> Result<MemoryInfo> {
    let mut sys = System::new();
    sys.refresh_memory();

    let total = sys.total_memory();
    let available = sys.available_memory();
    let used = total.saturating_sub(available);

    Ok(MemoryInfo {
        total_gb: total as f64 / 1_073_741_824.0, // 1024^3
        available_gb: available as f64 / 1_073_741_824.0,
        used_gb: used as f64 / 1_073_741_824.0,
    })
}

/// Check if there's enough memory available for a given requirement
pub fn has_available_memory(required_gb: f64) -> Result<bool> {
    let info = get_memory_info()?;
    Ok(info.available_gb >= required_gb)
}

/// Wait for memory to be available (with timeout)
pub async fn wait_for_memory(required_gb: f64, timeout_secs: u64) -> Result<bool> {
    use std::time::{Duration, Instant};
    use tokio::time::sleep;

    let deadline = Instant::now() + Duration::from_secs(timeout_secs);

    while Instant::now() < deadline {
        if has_available_memory(required_gb)? {
            return Ok(true);
        }
        sleep(Duration::from_secs(2)).await;
    }

    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_memory_info() {
        let info = get_memory_info().unwrap();
        assert!(info.total_gb > 0.0);
        assert!(info.available_gb > 0.0);
        assert!(info.available_gb <= info.total_gb);
    }
}
