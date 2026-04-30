use std::fs;
use std::path::{Path, PathBuf};

use crate::tools::{resolve_tool, tool_exists, run_tool, run_tool_with_cwd, bundled_tools_dir, gs_lib_debug};

// ---------------------------------------------------------------------------
// Supported extensions (C-2: single source of truth)
// ---------------------------------------------------------------------------

pub const SUPPORTED_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "pdf", "docx", "xlsx", "pptx"];

// ---------------------------------------------------------------------------
// Helper: safe path to &str
// ---------------------------------------------------------------------------

fn path_str(p: &Path) -> Result<&str, String> {
    p.to_str().ok_or_else(|| format!("無効なファイルパス: {}", p.display()))
}

// ---------------------------------------------------------------------------
// JPEG compression
// ---------------------------------------------------------------------------

pub fn compress_jpeg(src: &Path, dst: &Path, quality: u32, progressive: bool, _strip_metadata: bool) -> Result<(), String> {
    let cjpegli = resolve_tool("cjpegli");
    let src_s = path_str(src)?;
    let dst_s = path_str(dst)?;

    if tool_exists(&cjpegli) {
        let mut args = vec![
            src_s.to_string(),
            dst_s.to_string(),
            "-q".to_string(),
            quality.to_string(),
        ];
        if progressive {
            args.push("-p".to_string());
            args.push("2".to_string());
        }
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        run_tool(&cjpegli, &arg_refs)
    } else {
        // Fallback: sips (macOS only)
        #[cfg(target_os = "macos")]
        {
            run_tool("/usr/bin/sips", &[
                "-s", "format", "jpeg",
                "-s", "formatOptions", &quality.to_string(),
                src_s,
                "--out", dst_s,
            ])
        }
        #[cfg(target_os = "windows")]
        {
            let magick = resolve_tool("magick");
            if tool_exists(&magick) {
                run_tool(&magick, &[
                    "convert",
                    src_s,
                    "-quality", &quality.to_string(),
                    dst_s,
                ])
            } else {
                fs::copy(src, dst).map_err(|e| format!("コピー失敗: {}", e))?;
                Ok(())
            }
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            fs::copy(src, dst).map_err(|e| format!("コピー失敗: {}", e))?;
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// PNG compression (A-5: reduced intermediate copies)
// ---------------------------------------------------------------------------

pub fn compress_png(src: &Path, dst: &Path, colors: u32) -> Result<(), String> {
    let pngquant = resolve_tool("pngquant");
    let src_s = path_str(src)?;
    let dst_s = path_str(dst)?;

    if tool_exists(&pngquant) {
        // pngquant: read from src, write to dst directly (A-5: no intermediate copy)
        let pq_result = run_tool(&pngquant, &[
            "--force", "--quality=60-95",
            &colors.to_string(),
            "--output", dst_s,
            "--", src_s,
        ]);

        if pq_result.is_err() {
            // Exit code 99 = quality too low. Retry without constraint.
            let retry = run_tool(&pngquant, &[
                "--force", "--quality=0-100",
                &colors.to_string(),
                "--output", dst_s,
                "--", src_s,
            ]);
            if retry.is_err() {
                // pngquant completely failed — copy source as-is
                // (Oxipng will still optimize losslessly below)
                fs::copy(src, dst).map_err(|e| format!("コピー失敗: {}", e))?;
            }
        }

        // Second pass: Oxipng (strip metadata + filter optimization)
        let oxipng_opts = oxipng::Options {
            strip: oxipng::StripChunks::Safe,
            optimize_alpha: true,
            ..oxipng::Options::from_preset(4)
        };
        let _ = oxipng::optimize(
            &oxipng::InFile::Path(dst.to_path_buf()),
            &oxipng::OutFile::from_path(dst.to_path_buf()),
            &oxipng_opts,
        );

        // Third pass: ECT for lossless optimization
        let ect = resolve_tool("ect");
        if tool_exists(&ect) {
            let _ = run_tool(&ect, &["-9", "--strict", dst_s]);
        }
        Ok(())
    } else {
        fs::copy(src, dst).map_err(|e| format!("コピー失敗: {}", e))?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// PDF compression
// ---------------------------------------------------------------------------

pub fn compress_pdf(src: &Path, dst: &Path, dpi: u32, jpeg_q: u32) -> Result<(), String> {
    let gs = resolve_tool("gs");

    #[cfg(target_os = "windows")]
    let gs = if !tool_exists(&gs) {
        let gswin = resolve_tool("gswin64c");
        if tool_exists(&gswin) { gswin } else { gs }
    } else { gs };

    if !tool_exists(&gs) {
        return Err(format!("Ghostscript (gs) が見つかりません。\n検索パス: {}\nbundled_tools_dir: {:?}",
            gs, bundled_tools_dir()));
    }

    let src_s = path_str(src)?;
    let dpi_str = dpi.to_string();
    let jpeg_q_str = jpeg_q.to_string();

    let tmp_out = tempfile::Builder::new()
        .suffix(".pdf")
        .tempfile()
        .map_err(|e| format!("一時ファイル作成失敗: {}", e))?;
    let tmp_path = tmp_out.path().to_string_lossy().to_string();
    drop(tmp_out);

    let output_arg = format!("-sOutputFile={}", tmp_path);
    let result = run_tool(&gs, &[
        "-sDEVICE=pdfwrite", "-dCompatibilityLevel=1.5",
        "-dNOPAUSE", "-dBATCH", "-dQUIET",
        "-dDownsampleColorImages=true", "-dDownsampleGrayImages=true",
        "-dDownsampleMonoImages=true",
        &format!("-dColorImageResolution={}", dpi_str),
        &format!("-dGrayImageResolution={}", dpi_str),
        "-dMonoImageResolution=300",
        "-dColorImageDownsampleThreshold=1.0",
        "-dGrayImageDownsampleThreshold=1.0",
        "-dColorImageDownsampleType=/Bicubic",
        "-dGrayImageDownsampleType=/Bicubic",
        "-dAutoFilterColorImages=false", "-dAutoFilterGrayImages=false",
        "-dColorImageFilter=/DCTEncode", "-dGrayImageFilter=/DCTEncode",
        &format!("-dJPEGQ={}", jpeg_q_str),
        "-dCompressFonts=true", "-dSubsetFonts=true",
        "-dDetectDuplicateImages=true", "-dDoThumbnails=false",
        "-dOptimize=true",
        &output_arg,
        src_s,
    ]);

    match result {
        Ok(()) => {
            fs::copy(&tmp_path, dst).map_err(|e| format!("出力ファイルのコピー失敗: {}", e))?;
            let _ = fs::remove_file(&tmp_path);
            Ok(())
        }
        Err(e) => {
            let _ = fs::remove_file(&tmp_path);
            Err(format!("PDF圧縮エラー (gs={}):\n{}", gs, e))
        }
    }
}

pub fn compress_pdf_to_size(src: &Path, dst: &Path, target_kb: u64) -> Result<(), String> {
    let target_bytes = target_kb * 1024;
    let original_size = fs::metadata(src).map(|m| m.len()).unwrap_or(0);

    if original_size <= target_bytes {
        fs::copy(src, dst).map_err(|e| format!("コピー失敗: {}", e))?;
        return Ok(());
    }

    let presets: Vec<(u32, u32)> = vec![
        (235, 82), (200, 80), (175, 78), (150, 75),
        (130, 72), (110, 70), (100, 65), (90, 60),
        (80, 55), (72, 50), (72, 40), (72, 30),
    ];

    let mut best_result: Option<PathBuf> = None;

    for (dpi, jpeg_q) in &presets {
        let tmp = tempfile::Builder::new()
            .suffix(".pdf")
            .tempfile()
            .map_err(|e| format!("一時ファイル作成失敗: {}", e))?;
        let tmp_path = tmp.path().to_path_buf();
        drop(tmp);

        if compress_pdf(src, &tmp_path, *dpi, *jpeg_q).is_ok() {
            let size = fs::metadata(&tmp_path).map(|m| m.len()).unwrap_or(u64::MAX);
            if size <= target_bytes {
                if let Some(ref prev) = best_result {
                    let _ = fs::remove_file(prev);
                }
                best_result = Some(tmp_path);
                break;
            } else {
                if let Some(ref prev) = best_result {
                    let _ = fs::remove_file(prev);
                }
                best_result = Some(tmp_path);
            }
        } else {
            let _ = fs::remove_file(&tmp_path);
        }
    }

    match best_result {
        Some(ref result_path) => {
            fs::copy(result_path, dst).map_err(|e| format!("コピー失敗: {}", e))?;
            let _ = fs::remove_file(result_path);
            let final_size = fs::metadata(dst).map(|m| m.len()).unwrap_or(0);
            if final_size > target_bytes {
                eprintln!("Warning: target {}KB not met, result is {}KB",
                    target_kb, final_size / 1024);
            }
            Ok(())
        }
        None => Err("PDF サイズ目標圧縮に失敗しました".to_string()),
    }
}

// ---------------------------------------------------------------------------
// Format conversion
// ---------------------------------------------------------------------------

pub fn convert_to_webp(src: &Path, dst: &Path, quality: u32) -> Result<(), String> {
    let cwebp = resolve_tool("cwebp");
    let src_s = path_str(src)?;
    let dst_s = path_str(dst)?;
    if tool_exists(&cwebp) {
        run_tool(&cwebp, &["-q", &quality.to_string(), src_s, "-o", dst_s])
    } else {
        Err("cwebp が見つかりません。WebP 変換はスキップされます。".to_string())
    }
}

pub fn convert_to_jxl(src: &Path, dst: &Path, quality: u32, lossless: bool) -> Result<(), String> {
    let cjxl = resolve_tool("cjxl");
    let src_s = path_str(src)?;
    let dst_s = path_str(dst)?;
    if tool_exists(&cjxl) {
        let src_ext = src.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .unwrap_or_default();
        if lossless && (src_ext == "jpg" || src_ext == "jpeg") {
            run_tool(&cjxl, &[src_s, dst_s])
        } else {
            run_tool(&cjxl, &[src_s, dst_s, "-q", &quality.to_string()])
        }
    } else {
        Err("cjxl が見つかりません。JPEG XL 変換はスキップされます。".to_string())
    }
}

pub fn convert_to_avif(src: &Path, dst: &Path, quality: u32) -> Result<(), String> {
    let avifenc = resolve_tool("avifenc");
    let src_s = path_str(src)?;
    let dst_s = path_str(dst)?;
    if tool_exists(&avifenc) {
        run_tool(&avifenc, &[src_s, dst_s, "-q", &quality.to_string(), "-s", "6"])
    } else {
        Err("avifenc が見つかりません。AVIF 変換はスキップされます。".to_string())
    }
}

// ---------------------------------------------------------------------------
// Office file compression
// ---------------------------------------------------------------------------

pub fn compress_office(src: &Path, dst: &Path, jpeg_quality: u32, png_colors: u32, file_type: &str) -> Result<(), String> {
    let media_prefix = match file_type {
        "docx" => "word/media",
        "xlsx" => "xl/media",
        _ => "ppt/media",
    };

    let src_s = path_str(src)?;
    let dst_s = path_str(dst)?;

    let tmp_dir = tempfile::tempdir().map_err(|e| format!("一時ディレクトリ作成失敗: {}", e))?;
    let extract_dir = tmp_dir.path().join("extracted");
    fs::create_dir_all(&extract_dir).map_err(|e| format!("ディレクトリ作成失敗: {}", e))?;
    let extract_s = extract_dir.to_str()
        .ok_or_else(|| "一時ディレクトリのパスが無効です".to_string())?;

    // Unzip – cross-platform
    #[cfg(target_os = "macos")]
    run_tool("/usr/bin/unzip", &["-o", "-q", src_s, "-d", extract_s])?;

    #[cfg(target_os = "windows")]
    {
        run_tool("powershell", &[
            "-NoProfile", "-Command",
            &format!("Expand-Archive -Path '{}' -DestinationPath '{}' -Force", src_s, extract_s),
        ])?;
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    run_tool("unzip", &["-o", "-q", src_s, "-d", extract_s])?;

    // Compress media images
    let media_dir = extract_dir.join(media_prefix);
    if media_dir.exists() {
        if let Ok(entries) = fs::read_dir(&media_dir) {
            let cjpegli = resolve_tool("cjpegli");
            let pngquant = resolve_tool("pngquant");
            for entry in entries.flatten() {
                let path = entry.path();
                let ext = path.extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.to_lowercase())
                    .unwrap_or_default();

                if ext == "jpg" || ext == "jpeg" {
                    if tool_exists(&cjpegli) {
                        let tmp = path.with_extension("tmp.jpg");
                        if let (Ok(p_s), Ok(t_s)) = (path_str(&path), path_str(&tmp)) {
                            if run_tool(&cjpegli, &[p_s, t_s, "-q", &jpeg_quality.to_string()]).is_ok() {
                                let orig_size = fs::metadata(&path).map(|m| m.len()).unwrap_or(u64::MAX);
                                let new_size = fs::metadata(&tmp).map(|m| m.len()).unwrap_or(u64::MAX);
                                if new_size < orig_size {
                                    let _ = fs::remove_file(&path);
                                    let _ = fs::rename(&tmp, &path);
                                } else {
                                    let _ = fs::remove_file(&tmp);
                                }
                            }
                        }
                    }
                } else if ext == "png" {
                    if tool_exists(&pngquant) {
                        let tmp = path.with_extension("tmp.png");
                        if let (Ok(p_s), Ok(t_s)) = (path_str(&path), path_str(&tmp)) {
                            if run_tool(&pngquant, &[
                                "--force", "--quality=60-95",
                                &png_colors.to_string(),
                                "--output", t_s,
                                "--", p_s,
                            ]).is_ok() {
                                let orig_size = fs::metadata(&path).map(|m| m.len()).unwrap_or(u64::MAX);
                                let new_size = fs::metadata(&tmp).map(|m| m.len()).unwrap_or(u64::MAX);
                                if new_size < orig_size {
                                    let _ = fs::remove_file(&path);
                                    let _ = fs::rename(&tmp, &path);
                                } else {
                                    let _ = fs::remove_file(&tmp);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Re-zip – cross-platform
    let _ = fs::remove_file(dst);

    #[cfg(target_os = "macos")]
    run_tool_with_cwd("/usr/bin/zip", &["-r", "-q", dst_s, "."], extract_s)?;

    #[cfg(target_os = "windows")]
    {
        run_tool("powershell", &[
            "-NoProfile", "-Command",
            &format!("Compress-Archive -Path '{}\\*' -DestinationPath '{}' -Force", extract_s, dst_s),
        ])?;
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    run_tool_with_cwd("zip", &["-r", "-q", dst_s, "."], extract_s)?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Resize helper (B-2: no more mem::forget)
// ---------------------------------------------------------------------------

pub fn resize_if_needed(src: &Path, max_w: u32, max_h: u32) -> Result<Option<PathBuf>, String> {
    if max_w == 0 && max_h == 0 {
        return Ok(None);
    }
    let ext = src.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();
    if ext != "jpg" && ext != "jpeg" && ext != "png" {
        return Ok(None);
    }

    let img = image::open(src).map_err(|e| format!("画像読み込み失敗: {}", e))?;
    let (orig_w, orig_h) = (img.width(), img.height());

    let target_w = if max_w > 0 { max_w } else { orig_w };
    let target_h = if max_h > 0 { max_h } else { orig_h };

    if orig_w <= target_w && orig_h <= target_h {
        return Ok(None);
    }

    let ratio_w = target_w as f64 / orig_w as f64;
    let ratio_h = target_h as f64 / orig_h as f64;
    let ratio = ratio_w.min(ratio_h);
    let new_w = (orig_w as f64 * ratio).round() as u32;
    let new_h = (orig_h as f64 * ratio).round() as u32;

    let resized = img.resize(new_w, new_h, image::imageops::FilterType::Lanczos3);

    // B-2 fix: use persist() instead of mem::forget()
    let tmp = tempfile::Builder::new()
        .suffix(&format!(".{}", ext))
        .tempfile()
        .map_err(|e| format!("一時ファイル作成失敗: {}", e))?;

    let persist_path = tmp.path().to_path_buf();
    resized.save(&persist_path).map_err(|e| format!("リサイズ画像保存失敗: {}", e))?;

    // Keep the tempfile handle alive to prevent deletion, then persist it
    let persisted = tmp.into_temp_path()
        .keep()
        .map_err(|e| format!("一時ファイル永続化失敗: {}", e))?;

    Ok(Some(persisted))
}

// ---------------------------------------------------------------------------
// Folder expansion (C-2: uses SUPPORTED_EXTENSIONS)
// ---------------------------------------------------------------------------

pub fn expand_inputs(inputs: &[String]) -> Vec<String> {
    let mut result = Vec::new();
    for input in inputs {
        let p = Path::new(input);
        if p.is_dir() {
            collect_files_recursive(p, &mut result);
        } else {
            result.push(input.clone());
        }
    }
    result
}

fn collect_files_recursive(dir: &Path, out: &mut Vec<String>) {
    if let Ok(entries) = fs::read_dir(dir) {
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|e| e.path());
        for entry in entries {
            let path = entry.path();
            if path.is_dir() {
                collect_files_recursive(&path, out);
            } else {
                let ext = path.extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.to_lowercase())
                    .unwrap_or_default();
                if SUPPORTED_EXTENSIONS.contains(&ext.as_str()) {
                    out.push(path.to_string_lossy().to_string());
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Error report generator (C-4)
// ---------------------------------------------------------------------------

pub fn generate_error_report(
    src: &Path,
    input: &str,
    out_dir: &Path,
    error: &str,
    chrono_now_fn: impl Fn() -> String,
) -> String {
    format!(
        "=== はむはむ画像圧縮くん エラーレポート ===\n\
        日時: {}\n\
        ファイル: {}\n\
        入力パス: {}\n\
        出力先: {}\n\n\
        --- エラー詳細 ---\n{}\n\n\
        --- 環境情報 ---\n\
        OS: {}\n\
        ARCH: {}\n\
        gs パス: {}\n\
        pngquant パス: {}\n\
        cjpegli パス: {}\n\
        cjxl パス: {}\n\
        cwebp パス: {}\n\
        avifenc パス: {}\n\
        ect パス: {}\n\
        bundled_tools_dir: {:?}\n\
        gs 存在: {}\n\n\
        --- GS_LIB 確認 ---\n{}\n",
        chrono_now_fn(),
        src.file_name().unwrap_or_default().to_string_lossy(),
        input,
        out_dir.display(),
        error,
        std::env::consts::OS,
        std::env::consts::ARCH,
        resolve_tool("gs"),
        resolve_tool("pngquant"),
        resolve_tool("cjpegli"),
        resolve_tool("cjxl"),
        resolve_tool("cwebp"),
        resolve_tool("avifenc"),
        resolve_tool("ect"),
        bundled_tools_dir(),
        tool_exists(&resolve_tool("gs")),
        gs_lib_debug(),
    )
}
