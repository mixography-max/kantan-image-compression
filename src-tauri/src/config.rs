use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Timestamp (B-6: human-readable ISO 8601 approximation)
// ---------------------------------------------------------------------------

pub fn chrono_now() -> String {
    match SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) {
        Ok(d) => {
            let secs = d.as_secs();
            // Simple UTC timestamp formatting (no external crate needed)
            // Format: YYYY-MM-DD HH:MM:SS (UTC)
            let days = secs / 86400;
            let time_of_day = secs % 86400;
            let hours = time_of_day / 3600;
            let minutes = (time_of_day % 3600) / 60;
            let seconds = time_of_day % 60;

            // Calculate date from days since epoch (1970-01-01)
            let (year, month, day) = days_to_ymd(days);

            format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
                year, month, day, hours, minutes, seconds)
        }
        Err(_) => "unknown".to_string(),
    }
}

/// Convert days since Unix epoch to (year, month, day).
fn days_to_ymd(days: u64) -> (u64, u64, u64) {
    // Algorithm from http://howardhinnant.github.io/date_algorithms.html
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

// ---------------------------------------------------------------------------
// Output directory
// ---------------------------------------------------------------------------

pub fn default_output_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    let home = std::env::var("USERPROFILE").unwrap_or_else(|_| "C:\\".to_string());
    #[cfg(not(target_os = "windows"))]
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());

    PathBuf::from(&home).join("Desktop").join("圧縮済み")
}

pub fn output_dir_from(custom: Option<&str>) -> PathBuf {
    let dir = match custom {
        Some(d) if !d.is_empty() => PathBuf::from(d),
        _ => default_output_dir(),
    };
    let _ = fs::create_dir_all(&dir);
    dir
}

// ---------------------------------------------------------------------------
// Config persistence
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct AppConfig {
    pub output_dir: Option<String>,
}

pub fn config_path() -> PathBuf {
    #[cfg(target_os = "windows")]
    let home = std::env::var("APPDATA").unwrap_or_else(|_| {
        std::env::var("USERPROFILE").unwrap_or_else(|_| "C:\\".to_string())
    });
    #[cfg(target_os = "macos")]
    let home = {
        let h = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        format!("{}/Library/Application Support", h)
    };
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let home = {
        let h = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        format!("{}/.config", h)
    };

    let dir = PathBuf::from(&home).join("kantan-image-compression");
    let _ = fs::create_dir_all(&dir);
    dir.join("config.json")
}

pub fn load_config() -> AppConfig {
    let path = config_path();
    if let Ok(data) = fs::read_to_string(&path) {
        serde_json::from_str(&data).unwrap_or_default()
    } else {
        AppConfig::default()
    }
}

pub fn save_config(config: &AppConfig) {
    let path = config_path();
    if let Ok(json) = serde_json::to_string_pretty(config) {
        if let Err(e) = fs::write(&path, json) {
            eprintln!("設定保存失敗: {}", e); // B-5: log instead of silent ignore
        }
    }
}

// ---------------------------------------------------------------------------
// Compression history
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub filename: String,
    pub original_size: u64,
    pub compressed_size: u64,
    pub reduction: f64,
    pub output_path: String,
    pub timestamp: String,
}

fn history_path() -> PathBuf {
    let dir = config_path().parent().unwrap_or(Path::new("/tmp")).to_path_buf();
    let _ = fs::create_dir_all(&dir);
    dir.join("history.json")
}

pub fn load_history() -> Vec<HistoryEntry> {
    let path = history_path();
    if let Ok(data) = fs::read_to_string(&path) {
        serde_json::from_str(&data).unwrap_or_default()
    } else {
        Vec::new()
    }
}

pub fn save_history(history: &[HistoryEntry]) {
    let path = history_path();
    if let Ok(json) = serde_json::to_string_pretty(history) {
        let _ = fs::write(&path, json);
    }
}

pub fn append_history(entries: &[HistoryEntry]) {
    let mut history = load_history();
    history.extend_from_slice(entries);
    save_history(&history);
}
