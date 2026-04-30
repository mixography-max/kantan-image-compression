mod tools;
mod quality;
pub mod compress;
mod config;

use std::fs;
use std::path::Path;
use std::sync::Mutex;
use serde::{Deserialize, Serialize};
use rayon::prelude::*;
use tauri::{AppHandle, Emitter};

// Re-exports for CLI
pub use compress::SUPPORTED_EXTENSIONS;
pub use config::chrono_now;

// ---------------------------------------------------------------------------
// Logger abstraction (GUI vs CLI)
// ---------------------------------------------------------------------------

pub trait Logger: Send + Sync {
    fn log(&self, message: &str);
}

/// Logger for Tauri GUI — emits events to the frontend
struct TauriLogger {
    app: AppHandle,
}

impl Logger for TauriLogger {
    fn log(&self, message: &str) {
        let _ = self.app.emit("auto-quality-log", message.to_string());
    }
}

/// Logger for CLI — prints to stderr
pub struct StderrLogger;

impl Logger for StderrLogger {
    fn log(&self, message: &str) {
        eprintln!("{}", message);
    }
}

// ---------------------------------------------------------------------------
// Settings & Result types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompressSettings {
    pub jpeg_quality: Option<u32>,
    pub png_colors: Option<u32>,
    pub pdf_dpi: Option<u32>,
    pub pdf_jpeg_q: Option<u32>,
    pub office_quality: Option<u32>,
    pub output_dir: Option<String>,
    pub strip_metadata: Option<bool>,
    pub progressive_jpeg: Option<bool>,
    pub max_width: Option<u32>,
    pub max_height: Option<u32>,
    pub convert_webp: Option<bool>,
    pub target_size_kb: Option<u64>,
    pub convert_jxl: Option<bool>,
    pub jxl_lossless: Option<bool>,
    pub convert_avif: Option<bool>,
    pub auto_quality: Option<bool>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompressResult {
    pub filename: String,
    pub output_filename: String,
    pub output_path: String,
    pub original_size: u64,
    pub compressed_size: u64,
    pub reduction: f64,
    pub is_error: bool,
    pub error_message: Option<String>,
}

// ---------------------------------------------------------------------------
// Core compression logic (shared by GUI and CLI)
// ---------------------------------------------------------------------------

pub fn compress_files(inputs: &[String], settings: &CompressSettings, logger: &dyn Logger) -> Vec<CompressResult> {
    let out_dir = config::output_dir_from(settings.output_dir.as_deref());
    let jpeg_q = settings.jpeg_quality.unwrap_or(85);
    let png_c = settings.png_colors.unwrap_or(256);
    let pdf_dpi = settings.pdf_dpi.unwrap_or(235);
    let pdf_jq = settings.pdf_jpeg_q.unwrap_or(82);
    let office_q = settings.office_quality.unwrap_or(80);
    let progressive = settings.progressive_jpeg.unwrap_or(true);
    let strip_meta = settings.strip_metadata.unwrap_or(true);
    let max_w = settings.max_width.unwrap_or(0);
    let max_h = settings.max_height.unwrap_or(0);
    let webp_mode = settings.convert_webp.unwrap_or(false);
    let target_kb = settings.target_size_kb.unwrap_or(0);
    let jxl_mode = settings.convert_jxl.unwrap_or(false);
    let jxl_lossless = settings.jxl_lossless.unwrap_or(true);
    let avif_mode = settings.convert_avif.unwrap_or(false);
    let auto_q = settings.auto_quality.unwrap_or(false);

    let all_inputs = compress::expand_inputs(inputs);
    let results = Mutex::new(Vec::new());

    all_inputs.par_iter().for_each(|input| {
        let src = Path::new(input);
        let ext = src.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .unwrap_or_default();

        if !SUPPORTED_EXTENSIONS.contains(&ext.as_str()) {
            results.lock().unwrap().push(CompressResult {
                filename: src.file_name().unwrap_or_default().to_string_lossy().to_string(),
                output_filename: String::new(),
                output_path: String::new(),
                original_size: 0,
                compressed_size: 0,
                reduction: 0.0,
                is_error: true,
                error_message: Some(format!("未対応のファイル形式: .{}", ext)),
            });
            return;
        }

        let original_size = fs::metadata(src).map(|m| m.len()).unwrap_or(0);
        let stem = src.file_stem().unwrap_or_default().to_string_lossy();

        let is_image = matches!(ext.as_str(), "jpg" | "jpeg" | "png");
        let out_ext = if jxl_mode && is_image {
            "jxl".to_string()
        } else if avif_mode && is_image {
            "avif".to_string()
        } else if webp_mode && is_image {
            "webp".to_string()
        } else {
            ext.clone()
        };
        let out_name = format!("{}_compressed.{}", stem, out_ext);
        let dst = out_dir.join(&out_name);

        // Resize if needed
        let skip_resize = jxl_mode && jxl_lossless && matches!(ext.as_str(), "jpg" | "jpeg");
        let resized_path = if skip_resize {
            None
        } else {
            compress::resize_if_needed(src, max_w, max_h).ok().flatten()
        };
        let actual_src = resized_path.as_deref().unwrap_or(src);

        let compress_result = if jxl_mode && is_image {
            compress::convert_to_jxl(actual_src, &dst, jpeg_q, jxl_lossless)
        } else if avif_mode && is_image {
            compress::convert_to_avif(actual_src, &dst, jpeg_q)
        } else if webp_mode && is_image {
            compress::convert_to_webp(actual_src, &dst, jpeg_q)
        } else if auto_q && matches!(ext.as_str(), "jpg" | "jpeg") {
            quality::auto_quality_jpeg(
                actual_src, &dst, progressive, strip_meta, logger,
                &src.file_name().unwrap_or_default().to_string_lossy(),
            ).map(|_| ())
        } else if auto_q && ext == "png" {
            quality::auto_quality_png(
                actual_src, &dst, logger,
                &src.file_name().unwrap_or_default().to_string_lossy(),
            ).map(|_| ())
        } else {
            match ext.as_str() {
                "jpg" | "jpeg" => compress::compress_jpeg(actual_src, &dst, jpeg_q, progressive, strip_meta),
                "png" => compress::compress_png(actual_src, &dst, png_c),
                "pdf" => {
                    if target_kb > 0 {
                        compress::compress_pdf_to_size(src, &dst, target_kb)
                    } else {
                        compress::compress_pdf(src, &dst, pdf_dpi, pdf_jq)
                    }
                }
                "docx" | "xlsx" | "pptx" => compress::compress_office(src, &dst, office_q, png_c, &ext),
                _ => Err("未対応".to_string()),
            }
        };

        // Clean up resized temp file
        if let Some(ref rp) = resized_path {
            let _ = fs::remove_file(rp);
        }

        match compress_result {
            Ok(()) => {
                let compressed_size = fs::metadata(&dst).map(|m| m.len()).unwrap_or(0);
                let reduction = if original_size > 0 {
                    (1.0 - compressed_size as f64 / original_size as f64) * 100.0
                } else {
                    0.0
                };
                results.lock().unwrap().push(CompressResult {
                    filename: src.file_name().unwrap_or_default().to_string_lossy().to_string(),
                    output_filename: out_name,
                    output_path: dst.to_string_lossy().to_string(),
                    original_size,
                    compressed_size,
                    reduction,
                    is_error: false,
                    error_message: None,
                });
            }
            Err(e) => {
                let report = compress::generate_error_report(
                    src, input, &out_dir, &e, config::chrono_now,
                );
                let report_path = out_dir.join(format!(
                    "error_report_{}.txt",
                    src.file_stem().unwrap_or_default().to_string_lossy()
                ));
                let _ = fs::write(&report_path, &report);

                results.lock().unwrap().push(CompressResult {
                    filename: src.file_name().unwrap_or_default().to_string_lossy().to_string(),
                    output_filename: String::new(),
                    output_path: String::new(),
                    original_size,
                    compressed_size: 0,
                    reduction: 0.0,
                    is_error: true,
                    error_message: Some(format!("{}\n\nエラーレポート: {}", e, report_path.display())),
                });
            }
        }
    });

    let final_results = results.into_inner().unwrap();

    // Save successful results to history
    let history_entries: Vec<config::HistoryEntry> = final_results.iter()
        .filter(|r| !r.is_error)
        .map(|r| config::HistoryEntry {
            filename: r.filename.clone(),
            original_size: r.original_size,
            compressed_size: r.compressed_size,
            reduction: r.reduction,
            output_path: r.output_path.clone(),
            timestamp: config::chrono_now(),
        })
        .collect();
    if !history_entries.is_empty() {
        config::append_history(&history_entries);
    }

    final_results
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[tauri::command]
fn compress(app: AppHandle, inputs: Vec<String>, settings: CompressSettings) -> Vec<CompressResult> {
    let logger = TauriLogger { app };
    compress_files(&inputs, &settings, &logger)
}

#[tauri::command]
fn get_output_dir() -> String {
    let cfg = config::load_config();
    match cfg.output_dir {
        Some(ref d) if !d.is_empty() => d.clone(),
        _ => config::default_output_dir().to_string_lossy().to_string(),
    }
}

#[tauri::command]
fn set_output_dir(path: String) {
    let mut cfg = config::load_config();
    cfg.output_dir = Some(path);
    config::save_config(&cfg);
}

#[tauri::command]
fn get_history() -> Vec<config::HistoryEntry> {
    config::load_history()
}

#[tauri::command]
fn clear_history() {
    config::save_history(&[]);
}

#[tauri::command]
fn delete_history_entries(indices: Vec<usize>) -> Vec<config::HistoryEntry> {
    let mut history = config::load_history();
    let mut sorted = indices;
    sorted.sort_unstable();
    sorted.dedup();
    for &i in sorted.iter().rev() {
        if i < history.len() {
            history.remove(i);
        }
    }
    config::save_history(&history);
    history
}

#[tauri::command]
fn export_history_csv() -> Result<String, String> {
    let history = config::load_history();
    let out_dir = config::default_output_dir();
    let _ = fs::create_dir_all(&out_dir);
    let csv_path = out_dir.join("compression_history.csv");

    let mut csv = String::from("日時,ファイル名,元サイズ(bytes),圧縮後サイズ(bytes),削減率(%),出力パス\n");
    for entry in &history {
        csv.push_str(&format!(
            "{},{},{},{},{:.1},{}\n",
            entry.timestamp,
            entry.filename.replace(',', "_"),
            entry.original_size,
            entry.compressed_size,
            entry.reduction,
            entry.output_path.replace(',', "_"),
        ));
    }
    fs::write(&csv_path, &csv).map_err(|e| format!("CSV書き出し失敗: {}", e))?;
    Ok(csv_path.to_string_lossy().to_string())
}

// ---------------------------------------------------------------------------
// App entry
// ---------------------------------------------------------------------------

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            compress,
            get_output_dir,
            set_output_dir,
            get_history,
            clear_history,
            delete_history_entries,
            export_history_csv,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
