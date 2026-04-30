use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use serde::{Deserialize, Serialize};
use rayon::prelude::*;
use tauri::{AppHandle, Emitter};

// ---------------------------------------------------------------------------
// Settings & Result types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CompressSettings {
    jpeg_quality: Option<u32>,
    png_colors: Option<u32>,
    pdf_dpi: Option<u32>,
    pdf_jpeg_q: Option<u32>,
    office_quality: Option<u32>,
    output_dir: Option<String>,
    strip_metadata: Option<bool>,
    progressive_jpeg: Option<bool>,
    max_width: Option<u32>,
    max_height: Option<u32>,
    convert_webp: Option<bool>,
    target_size_kb: Option<u64>,
    convert_jxl: Option<bool>,
    jxl_lossless: Option<bool>,
    convert_avif: Option<bool>,
    auto_quality: Option<bool>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CompressResult {
    filename: String,
    output_filename: String,
    output_path: String,
    original_size: u64,
    compressed_size: u64,
    reduction: f64,
    is_error: bool,
    error_message: Option<String>,
}

// ---------------------------------------------------------------------------
// Tool resolution – cross-platform (bundled tools have priority)
// ---------------------------------------------------------------------------

/// Returns the directory where bundled tools are placed at runtime.
fn bundled_tools_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let exe_dir = exe.parent()?;
    #[cfg(target_os = "macos")]
    {
        // macOS: Contents/MacOS/exe → Contents/Resources/resources/tools/
        let dir = exe_dir.parent()?.join("Resources").join("resources").join("tools");
        if dir.exists() { return Some(dir); }
        // Fallback: map-style resources path
        Some(exe_dir.parent()?.join("Resources").join("tools"))
    }
    #[cfg(not(target_os = "macos"))]
    {
        // Windows: resources/tools/ next to exe
        let dir = exe_dir.join("resources").join("tools");
        if dir.exists() { return Some(dir); }
        Some(exe_dir.join("tools"))
    }
}

fn resolve_tool(name: &str) -> String {
    // 1. Check bundled tools first
    if let Some(tools_dir) = bundled_tools_dir() {
        let bundled = if cfg!(windows) {
            tools_dir.join(format!("{}.exe", name))
        } else {
            tools_dir.join(name)
        };
        if bundled.exists() {
            return bundled.to_string_lossy().to_string();
        }
    }
    // 2. Check platform-specific system paths
    #[cfg(target_os = "macos")]
    {
        let candidates = [
            format!("/opt/homebrew/bin/{}", name),
            format!("/usr/local/bin/{}", name),
            format!("{}/bin/{}", std::env::var("HOME").unwrap_or_default(), name),
        ];
        for c in &candidates {
            if Path::new(c).exists() {
                return c.clone();
            }
        }
    }
    #[cfg(target_os = "windows")]
    {
        let home = std::env::var("USERPROFILE").unwrap_or_default();
        let candidates = [
            format!("{}\\bin\\{}.exe", home, name),
            format!("{}\\scoop\\shims\\{}.exe", home, name),
            format!("C:\\tools\\{}.exe", name),
            format!("C:\\Program Files\\{name}\\{name}.exe", name = name),
        ];
        for c in &candidates {
            if Path::new(c).exists() {
                return c.clone();
            }
        }
        if let Ok(output) = Command::new("where").arg(name).output() {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout);
                if let Some(first_line) = path.lines().next() {
                    if !first_line.is_empty() {
                        return first_line.to_string();
                    }
                }
            }
        }
    }
    name.to_string()
}

fn tool_exists(path: &str) -> bool {
    if Path::new(path).exists() {
        return true;
    }
    #[cfg(target_os = "windows")]
    {
        if let Ok(output) = Command::new("where").arg(path).output() {
            return output.status.success();
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        if let Ok(output) = Command::new("which").arg(path).output() {
            return output.status.success();
        }
    }
    false
}

/// Check if the given tool path points to a bundled tool.
fn is_bundled(path: &str) -> bool {
    if let Some(tools_dir) = bundled_tools_dir() {
        Path::new(path).starts_with(&tools_dir)
    } else {
        false
    }
}

/// Build a Command, adding library paths and GS_LIB for bundled tools.
fn make_tool_command(exe: &str) -> Command {
    let mut cmd = Command::new(exe);
    if is_bundled(exe) {
        if let Some(tools_dir) = bundled_tools_dir() {
            // Set library loading path for bundled dylibs/DLLs
            let lib_dir = tools_dir.join("lib");
            #[cfg(target_os = "macos")]
            {
                if lib_dir.exists() {
                    cmd.env("DYLD_LIBRARY_PATH", &lib_dir);
                }
            }
            #[cfg(target_os = "windows")]
            {
                if lib_dir.exists() {
                    // Prepend lib dir to PATH for DLL loading
                    let path = std::env::var("PATH").unwrap_or_default();
                    cmd.env("PATH", format!("{};{}", lib_dir.to_string_lossy(), path));
                }
            }

            // Set GS_LIB for Ghostscript resource files
            let gs_res = tools_dir.join("gsresource");
            if gs_res.exists() {
                let mut gs_lib_paths: Vec<String> = Vec::new();
                // Add the gsresource dir itself
                gs_lib_paths.push(gs_res.to_string_lossy().to_string());
                // Traverse subdirectories to find Resource, lib, Init
                fn collect_gs_paths(dir: &Path, paths: &mut Vec<String>) {
                    if let Ok(entries) = fs::read_dir(dir) {
                        for entry in entries.flatten() {
                            let p = entry.path();
                            if p.is_dir() {
                                let name = p.file_name().unwrap_or_default().to_string_lossy().to_string();
                                paths.push(p.to_string_lossy().to_string());
                                if name == "Resource" || name == "lib" || name == "Init" {
                                    // Don't recurse further into these
                                } else {
                                    collect_gs_paths(&p, paths);
                                }
                            }
                        }
                    }
                }
                collect_gs_paths(&gs_res, &mut gs_lib_paths);
                if !gs_lib_paths.is_empty() {
                    let sep = if cfg!(windows) { ";" } else { ":" };
                    cmd.env("GS_LIB", gs_lib_paths.join(sep));
                }
            }
        }
    }
    cmd
}

/// Get current timestamp as string (without chrono crate)
fn chrono_now() -> String {
    use std::time::SystemTime;
    match SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) {
        Ok(d) => format!("unix:{}", d.as_secs()),
        Err(_) => "unknown".to_string(),
    }
}

/// Debug info about GS_LIB resolution
fn gs_lib_debug() -> String {
    let mut info = String::new();
    if let Some(tools_dir) = bundled_tools_dir() {
        let gs_res = tools_dir.join("gsresource");
        info.push_str(&format!("gsresource dir: {:?} (exists: {})\n", gs_res, gs_res.exists()));
        if gs_res.exists() {
            fn list_dir_recursive(dir: &Path, depth: usize, out: &mut String) {
                if depth > 3 { return; }
                if let Ok(entries) = fs::read_dir(dir) {
                    for entry in entries.flatten() {
                        let p = entry.path();
                        let indent = "  ".repeat(depth);
                        let name = p.file_name().unwrap_or_default().to_string_lossy().to_string();
                        if p.is_dir() {
                            out.push_str(&format!("{}{}/\n", indent, name));
                            list_dir_recursive(&p, depth + 1, out);
                        } else {
                            out.push_str(&format!("{}{}\n", indent, name));
                        }
                    }
                }
            }
            list_dir_recursive(&gs_res, 0, &mut info);
        }
    } else {
        info.push_str("bundled_tools_dir: None\n");
    }
    info
}

fn default_output_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    let home = std::env::var("USERPROFILE").unwrap_or_else(|_| "C:\\".to_string());
    #[cfg(not(target_os = "windows"))]
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());

    PathBuf::from(&home).join("Desktop").join("圧縮済み")
}

fn output_dir_from(custom: Option<&str>) -> PathBuf {
    let dir = match custom {
        Some(d) if !d.is_empty() => PathBuf::from(d),
        _ => default_output_dir(),
    };
    let _ = fs::create_dir_all(&dir);
    dir
}

// ---------------------------------------------------------------------------
// Config persistence (saves output_dir to a JSON file)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, Default)]
struct AppConfig {
    output_dir: Option<String>,
}

fn config_path() -> PathBuf {
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

fn load_config() -> AppConfig {
    let path = config_path();
    if let Ok(data) = fs::read_to_string(&path) {
        serde_json::from_str(&data).unwrap_or_default()
    } else {
        AppConfig::default()
    }
}

fn save_config(config: &AppConfig) {
    let path = config_path();
    if let Ok(json) = serde_json::to_string_pretty(config) {
        let _ = fs::write(&path, json);
    }
}

fn run_tool(exe: &str, args: &[&str]) -> Result<(), String> {
    let output = make_tool_command(exe)
        .args(args)
        .output()
        .map_err(|e| format!("{} の実行に失敗: {}", exe, e))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut msg = format!("{} が失敗 (exit {})", exe, output.status);
        if !stderr.is_empty() {
            msg.push_str(&format!("\nstderr: {}", stderr.chars().take(500).collect::<String>()));
        }
        if !stdout.is_empty() {
            msg.push_str(&format!("\nstdout: {}", stdout.chars().take(300).collect::<String>()));
        }
        return Err(msg);
    }
    Ok(())
}

fn run_tool_with_cwd(exe: &str, args: &[&str], cwd: &str) -> Result<(), String> {
    let output = make_tool_command(exe)
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|e| format!("{} の実行に失敗: {}", exe, e))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("{} が失敗: {}", exe, stderr));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// SSIM calculation (luminance-based structural similarity)
// ---------------------------------------------------------------------------

fn compute_ssim(original: &Path, compressed: &Path) -> Result<f64, String> {
    let img_a = image::open(original)
        .map_err(|e| format!("元画像の読み込み失敗: {}", e))?
        .to_luma8();
    let img_b = image::open(compressed)
        .map_err(|e| format!("圧縮画像の読み込み失敗: {}", e))?
        .to_luma8();

    let (w_a, h_a) = img_a.dimensions();
    let (w_b, h_b) = img_b.dimensions();

    // If sizes differ, resize img_b to match img_a
    let img_b = if w_a != w_b || h_a != h_b {
        image::imageops::resize(&img_b, w_a, h_a, image::imageops::FilterType::Lanczos3)
    } else {
        img_b
    };

    let (width, height) = (w_a as usize, h_a as usize);
    let pixels_a: &[u8] = img_a.as_raw();
    let pixels_b: &[u8] = img_b.as_raw();

    // SSIM constants (for 8-bit images)
    let c1: f64 = (0.01 * 255.0) * (0.01 * 255.0); // 6.5025
    let c2: f64 = (0.03 * 255.0) * (0.03 * 255.0); // 58.5225

    // Compute SSIM over 8x8 blocks
    let block_size = 8usize;
    let mut ssim_sum = 0.0f64;
    let mut block_count = 0u64;

    let bx_count = width / block_size;
    let by_count = height / block_size;

    for by in 0..by_count {
        for bx in 0..bx_count {
            let mut sum_a = 0.0f64;
            let mut sum_b = 0.0f64;
            let mut sum_a2 = 0.0f64;
            let mut sum_b2 = 0.0f64;
            let mut sum_ab = 0.0f64;
            let n = (block_size * block_size) as f64;

            for dy in 0..block_size {
                for dx in 0..block_size {
                    let y = by * block_size + dy;
                    let x = bx * block_size + dx;
                    let idx = y * width + x;
                    let a = pixels_a[idx] as f64;
                    let b = pixels_b[idx] as f64;
                    sum_a += a;
                    sum_b += b;
                    sum_a2 += a * a;
                    sum_b2 += b * b;
                    sum_ab += a * b;
                }
            }

            let mu_a = sum_a / n;
            let mu_b = sum_b / n;
            let sigma_a2 = sum_a2 / n - mu_a * mu_a;
            let sigma_b2 = sum_b2 / n - mu_b * mu_b;
            let sigma_ab = sum_ab / n - mu_a * mu_b;

            let numerator = (2.0 * mu_a * mu_b + c1) * (2.0 * sigma_ab + c2);
            let denominator = (mu_a * mu_a + mu_b * mu_b + c1) * (sigma_a2 + sigma_b2 + c2);
            ssim_sum += numerator / denominator;
            block_count += 1;
        }
    }

    if block_count == 0 {
        return Ok(1.0); // Trivially similar (very small image)
    }
    Ok(ssim_sum / block_count as f64)
}

/// Auto-quality search for JPEG using binary search + SSIM
/// Target: SSIM >= 0.95 with minimum file size
fn auto_quality_jpeg(
    src: &Path,
    dst: &Path,
    progressive: bool,
    strip_meta: bool,
    app: &AppHandle,
    filename: &str,
) -> Result<u32, String> {
    let target_ssim: f64 = 0.95;
    let mut lo: u32 = 30;
    let mut hi: u32 = 95;
    let mut best_q: u32 = 85;
    let mut iteration = 0;

    let _ = app.emit("auto-quality-log", format!(
        "🔍 {} — SSIM自動品質探索開始 (目標: SSIM ≥ {:.2})", filename, target_ssim
    ));

    while lo <= hi {
        let mid = (lo + hi) / 2;
        iteration += 1;

        // Compress at this quality
        compress_jpeg(src, dst, mid, progressive, strip_meta)?;
        let ssim = compute_ssim(src, dst)?;
        let size = fs::metadata(dst).map(|m| m.len()).unwrap_or(0);
        let size_kb = size as f64 / 1024.0;

        let status = if ssim >= target_ssim { "✅" } else { "⚠️" };
        let msg = format!(
            "  #{} Q={:3} → SSIM={:.4} {} ({:.0}KB)",
            iteration, mid, ssim, status, size_kb
        );
        let _ = app.emit("auto-quality-log", msg);

        if ssim >= target_ssim {
            best_q = mid;
            hi = mid.saturating_sub(1); // Try lower quality
        } else {
            lo = mid + 1; // Need higher quality
        }

        if lo > hi { break; }
    }

    // Final compress at best quality
    compress_jpeg(src, dst, best_q, progressive, strip_meta)?;
    let final_ssim = compute_ssim(src, dst)?;
    let final_size = fs::metadata(dst).map(|m| m.len()).unwrap_or(0);

    let _ = app.emit("auto-quality-log", format!(
        "🎯 {} — 最適品質 Q={} (SSIM={:.4}, {:.0}KB)",
        filename, best_q, final_ssim, final_size as f64 / 1024.0
    ));

    Ok(best_q)
}

/// Auto-quality search for PNG using binary search over color count + SSIM
/// Target: SSIM >= 0.95 with minimum file size
fn auto_quality_png(
    src: &Path,
    dst: &Path,
    app: &AppHandle,
    filename: &str,
) -> Result<u32, String> {
    let target_ssim: f64 = 0.95;
    // Search range: 8 to 256 colors (powers-of-2 aware steps)
    let mut lo: u32 = 8;
    let mut hi: u32 = 256;
    let mut best_c: u32 = 256;
    let mut iteration = 0;

    let _ = app.emit("auto-quality-log", format!(
        "🔍 {} — PNG自動色数探索開始 (目標: SSIM ≥ {:.2})", filename, target_ssim
    ));

    while lo <= hi {
        let mid = (lo + hi) / 2;
        iteration += 1;

        // Compress at this color count
        compress_png(src, dst, mid)?;
        let ssim = compute_ssim(src, dst)?;
        let size = fs::metadata(dst).map(|m| m.len()).unwrap_or(0);
        let size_kb = size as f64 / 1024.0;

        let status = if ssim >= target_ssim { "✅" } else { "⚠️" };
        let msg = format!(
            "  #{} 色数={:3} → SSIM={:.4} {} ({:.0}KB)",
            iteration, mid, ssim, status, size_kb
        );
        let _ = app.emit("auto-quality-log", msg);

        if ssim >= target_ssim {
            best_c = mid;
            hi = mid.saturating_sub(1); // Try fewer colors
        } else {
            lo = mid + 1; // Need more colors
        }

        if lo > hi { break; }
    }

    // Final compress at best color count
    compress_png(src, dst, best_c)?;
    let final_ssim = compute_ssim(src, dst)?;
    let final_size = fs::metadata(dst).map(|m| m.len()).unwrap_or(0);

    let _ = app.emit("auto-quality-log", format!(
        "🎯 {} — 最適色数 {} (SSIM={:.4}, {:.0}KB)",
        filename, best_c, final_ssim, final_size as f64 / 1024.0
    ));

    Ok(best_c)
}

// ---------------------------------------------------------------------------
// Compression functions
// ---------------------------------------------------------------------------

fn compress_jpeg(src: &Path, dst: &Path, quality: u32, progressive: bool, _strip_metadata: bool) -> Result<(), String> {
    let cjpegli = resolve_tool("cjpegli");
    if tool_exists(&cjpegli) {
        let mut args = vec![
            src.to_str().unwrap().to_string(),
            dst.to_str().unwrap().to_string(),
            "-q".to_string(),
            quality.to_string(),
        ];
        if progressive {
            args.push("-p".to_string());
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
                src.to_str().unwrap(),
                "--out", dst.to_str().unwrap(),
            ])
        }
        #[cfg(target_os = "windows")]
        {
            // Fallback: use magick (ImageMagick) if available
            let magick = resolve_tool("magick");
            if tool_exists(&magick) {
                run_tool(&magick, &[
                    "convert",
                    src.to_str().unwrap(),
                    "-quality", &quality.to_string(),
                    dst.to_str().unwrap(),
                ])
            } else {
                // Last resort: just copy
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

fn compress_png(src: &Path, dst: &Path, colors: u32) -> Result<(), String> {
    let pngquant = resolve_tool("pngquant");
    if tool_exists(&pngquant) {
        fs::copy(src, dst).map_err(|e| format!("コピー失敗: {}", e))?;
        run_tool(&pngquant, &[
            "--force", "--quality=60-95",
            &colors.to_string(),
            "--output", dst.to_str().unwrap(),
            "--", dst.to_str().unwrap(),
        ])?;

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

        // Third pass: ECT (Efficient Compression Tool) for lossless optimization
        let ect = resolve_tool("ect");
        if tool_exists(&ect) {
            // -9 = max compression, --strict = preserve correctness
            let _ = run_tool(&ect, &["-9", "--strict", dst.to_str().unwrap()]);
        }
        Ok(())
    } else {
        // Fallback: just copy
        fs::copy(src, dst).map_err(|e| format!("コピー失敗: {}", e))?;
        Ok(())
    }
}

fn compress_pdf(src: &Path, dst: &Path, dpi: u32, jpeg_q: u32) -> Result<(), String> {
    let gs = resolve_tool("gs");

    // On Windows, Ghostscript binary name is different
    #[cfg(target_os = "windows")]
    let gs = if !tool_exists(&gs) {
        let gswin = resolve_tool("gswin64c");
        if tool_exists(&gswin) { gswin } else { gs }
    } else { gs };

    if !tool_exists(&gs) {
        return Err(format!("Ghostscript (gs) が見つかりません。\n検索パス: {}\nbundled_tools_dir: {:?}",
            gs, bundled_tools_dir()));
    }

    let dpi_str = dpi.to_string();
    let jpeg_q_str = jpeg_q.to_string();

    // Use a temp file for output to avoid path encoding issues, then move
    let tmp_out = tempfile::Builder::new()
        .suffix(".pdf")
        .tempfile()
        .map_err(|e| format!("一時ファイル作成失敗: {}", e))?;
    let tmp_path = tmp_out.path().to_string_lossy().to_string();
    // Close the temp file handle so gs can write to it
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
        src.to_str().unwrap(),
    ]);

    match result {
        Ok(()) => {
            // Move temp output to final destination
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

fn convert_to_webp(src: &Path, dst: &Path, quality: u32) -> Result<(), String> {
    let cwebp = resolve_tool("cwebp");
    if tool_exists(&cwebp) {
        run_tool(&cwebp, &[
            "-q", &quality.to_string(),
            src.to_str().unwrap(),
            "-o", dst.to_str().unwrap(),
        ])
    } else {
        Err("cwebp が見つかりません。WebP 変換はスキップされます。".to_string())
    }
}

fn convert_to_jxl(src: &Path, dst: &Path, quality: u32, lossless: bool) -> Result<(), String> {
    let cjxl = resolve_tool("cjxl");
    if tool_exists(&cjxl) {
        let src_ext = src.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .unwrap_or_default();
        if lossless && (src_ext == "jpg" || src_ext == "jpeg") {
            // Lossless JPEG → JXL transcoding (bit-perfect roundtrip)
            run_tool(&cjxl, &[
                src.to_str().unwrap(),
                dst.to_str().unwrap(),
            ])
        } else {
            // Lossy conversion with quality parameter
            run_tool(&cjxl, &[
                src.to_str().unwrap(),
                dst.to_str().unwrap(),
                "-q", &quality.to_string(),
            ])
        }
    } else {
        Err("cjxl が見つかりません。JPEG XL 変換はスキップされます。".to_string())
    }
}

fn convert_to_avif(src: &Path, dst: &Path, quality: u32) -> Result<(), String> {
    let avifenc = resolve_tool("avifenc");
    if tool_exists(&avifenc) {
        // Map quality (0-100) to AVIF min/max quantizer
        // avifenc uses -q (quality, 0-100 where 100 is lossless)
        run_tool(&avifenc, &[
            src.to_str().unwrap(),
            dst.to_str().unwrap(),
            "-q", &quality.to_string(),
            "-s", "6",   // speed 6 (balanced speed/quality)
        ])
    } else {
        Err("avifenc が見つかりません。AVIF 変換はスキップされます。".to_string())
    }
}

fn compress_pdf_to_size(src: &Path, dst: &Path, target_kb: u64) -> Result<(), String> {
    let target_bytes = target_kb * 1024;
    let original_size = fs::metadata(src).map(|m| m.len()).unwrap_or(0);

    if original_size <= target_bytes {
        fs::copy(src, dst).map_err(|e| format!("コピー失敗: {}", e))?;
        return Ok(());
    }

    // Binary search: try different DPI/quality combinations
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
                // Found a setting that meets the target
                if let Some(ref prev) = best_result {
                    let _ = fs::remove_file(prev);
                }
                best_result = Some(tmp_path);
                break;
            } else {
                // Still too large, try lower quality
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
                // Could not meet target, but use best effort
                eprintln!("Warning: target {}KB not met, result is {}KB",
                    target_kb, final_size / 1024);
            }
            Ok(())
        }
        None => Err("PDF サイズ目標圧縮に失敗しました".to_string()),
    }
}

fn compress_office(src: &Path, dst: &Path, jpeg_quality: u32, png_colors: u32, file_type: &str) -> Result<(), String> {
    let media_prefix = match file_type {
        "docx" => "word/media",
        "xlsx" => "xl/media",
        _ => "ppt/media",
    };

    let tmp_dir = tempfile::tempdir().map_err(|e| format!("一時ディレクトリ作成失敗: {}", e))?;
    let extract_dir = tmp_dir.path().join("extracted");
    fs::create_dir_all(&extract_dir).map_err(|e| format!("ディレクトリ作成失敗: {}", e))?;

    // Unzip – cross-platform
    #[cfg(target_os = "macos")]
    run_tool("/usr/bin/unzip", &[
        "-o", "-q",
        src.to_str().unwrap(),
        "-d", extract_dir.to_str().unwrap(),
    ])?;

    #[cfg(target_os = "windows")]
    {
        // Use PowerShell Expand-Archive on Windows
        run_tool("powershell", &[
            "-NoProfile", "-Command",
            &format!(
                "Expand-Archive -Path '{}' -DestinationPath '{}' -Force",
                src.to_str().unwrap(),
                extract_dir.to_str().unwrap()
            ),
        ])?;
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    run_tool("unzip", &[
        "-o", "-q",
        src.to_str().unwrap(),
        "-d", extract_dir.to_str().unwrap(),
    ])?;

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
                        if run_tool(&cjpegli, &[
                            path.to_str().unwrap(),
                            tmp.to_str().unwrap(),
                            "-q", &jpeg_quality.to_string(),
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
                } else if ext == "png" {
                    if tool_exists(&pngquant) {
                        let tmp = path.with_extension("tmp.png");
                        if run_tool(&pngquant, &[
                            "--force", "--quality=60-95",
                            &png_colors.to_string(),
                            "--output", tmp.to_str().unwrap(),
                            "--", path.to_str().unwrap(),
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

    // Re-zip – cross-platform
    let _ = fs::remove_file(dst);

    #[cfg(target_os = "macos")]
    run_tool_with_cwd("/usr/bin/zip", &[
        "-r", "-q",
        dst.to_str().unwrap(),
        ".",
    ], extract_dir.to_str().unwrap())?;

    #[cfg(target_os = "windows")]
    {
        run_tool("powershell", &[
            "-NoProfile", "-Command",
            &format!(
                "Compress-Archive -Path '{}\\*' -DestinationPath '{}' -Force",
                extract_dir.to_str().unwrap(),
                dst.to_str().unwrap()
            ),
        ])?;
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    run_tool_with_cwd("zip", &[
        "-r", "-q",
        dst.to_str().unwrap(),
        ".",
    ], extract_dir.to_str().unwrap())?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Resize helper (using `image` crate)
// ---------------------------------------------------------------------------

fn resize_if_needed(src: &Path, max_w: u32, max_h: u32) -> Result<Option<PathBuf>, String> {
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
        return Ok(None); // Already within limits
    }

    let ratio_w = target_w as f64 / orig_w as f64;
    let ratio_h = target_h as f64 / orig_h as f64;
    let ratio = ratio_w.min(ratio_h);
    let new_w = (orig_w as f64 * ratio).round() as u32;
    let new_h = (orig_h as f64 * ratio).round() as u32;

    let resized = img.resize(new_w, new_h, image::imageops::FilterType::Lanczos3);

    let tmp = tempfile::Builder::new()
        .suffix(&format!(".{}", ext))
        .tempfile()
        .map_err(|e| format!("一時ファイル作成失敗: {}", e))?;
    let _tmp_path = tmp.path().to_path_buf();
    // Keep temp file alive by persisting
    let persist_path = tmp.into_temp_path();

    resized.save(&*persist_path).map_err(|e| format!("リサイズ画像保存失敗: {}", e))?;
    // Leak the TempPath so the file is not deleted
    let leaked = persist_path.to_path_buf();
    std::mem::forget(persist_path);
    Ok(Some(leaked))
}

// ---------------------------------------------------------------------------
// Folder expansion
// ---------------------------------------------------------------------------

fn expand_inputs(inputs: &[String]) -> Vec<String> {
    let supported = ["jpg", "jpeg", "png", "pdf", "docx", "xlsx", "pptx"];
    let mut result = Vec::new();
    for input in inputs {
        let p = Path::new(input);
        if p.is_dir() {
            collect_files_recursive(p, &supported, &mut result);
        } else {
            result.push(input.clone());
        }
    }
    result
}

fn collect_files_recursive(dir: &Path, extensions: &[&str], out: &mut Vec<String>) {
    if let Ok(entries) = fs::read_dir(dir) {
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|e| e.path());
        for entry in entries {
            let path = entry.path();
            if path.is_dir() {
                collect_files_recursive(&path, extensions, out);
            } else {
                let ext = path.extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.to_lowercase())
                    .unwrap_or_default();
                if extensions.contains(&ext.as_str()) {
                    out.push(path.to_string_lossy().to_string());
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tauri command
// ---------------------------------------------------------------------------

#[tauri::command]
fn compress(app: AppHandle, inputs: Vec<String>, settings: CompressSettings) -> Vec<CompressResult> {
    let out_dir = output_dir_from(settings.output_dir.as_deref());
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

    let supported = ["jpg", "jpeg", "png", "pdf", "docx", "xlsx", "pptx"];

    // Expand folders to individual files
    let all_inputs = expand_inputs(&inputs);

    let results = Mutex::new(Vec::new());

    all_inputs.par_iter().for_each(|input| {
        let src = Path::new(input);
        let ext = src.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .unwrap_or_default();

        if !supported.contains(&ext.as_str()) {
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

        // Determine output extension (WebP/JXL/AVIF conversion for images)
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

        // Resize if needed (images only, skip resize for lossless JXL)
        let skip_resize = jxl_mode && jxl_lossless && matches!(ext.as_str(), "jpg" | "jpeg");
        let resized_path = if skip_resize {
            None
        } else {
            resize_if_needed(src, max_w, max_h).ok().flatten()
        };
        let actual_src = resized_path.as_deref().unwrap_or(src);

        let compress_result = if jxl_mode && is_image {
            convert_to_jxl(actual_src, &dst, jpeg_q, jxl_lossless)
        } else if avif_mode && is_image {
            convert_to_avif(actual_src, &dst, jpeg_q)
        } else if webp_mode && is_image {
            convert_to_webp(actual_src, &dst, jpeg_q)
        } else if auto_q && matches!(ext.as_str(), "jpg" | "jpeg") {
            // SSIM auto-quality for JPEG
            auto_quality_jpeg(
                actual_src, &dst, progressive, strip_meta, &app,
                &src.file_name().unwrap_or_default().to_string_lossy(),
            ).map(|_| ())
        } else if auto_q && ext == "png" {
            // SSIM auto-quality for PNG (color count search)
            auto_quality_png(
                actual_src, &dst, &app,
                &src.file_name().unwrap_or_default().to_string_lossy(),
            ).map(|_| ())
        } else {
            match ext.as_str() {
                "jpg" | "jpeg" => compress_jpeg(actual_src, &dst, jpeg_q, progressive, strip_meta),
                "png" => compress_png(actual_src, &dst, png_c),
                "pdf" => {
                    if target_kb > 0 {
                        compress_pdf_to_size(src, &dst, target_kb)
                    } else {
                        compress_pdf(src, &dst, pdf_dpi, pdf_jq)
                    }
                }
                "docx" | "xlsx" | "pptx" => compress_office(src, &dst, office_q, png_c, &ext),
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
                // Save error report file
                let report = format!(
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
                    chrono_now(),
                    src.file_name().unwrap_or_default().to_string_lossy(),
                    input,
                    out_dir.display(),
                    e,
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
    let history_entries: Vec<HistoryEntry> = final_results.iter()
        .filter(|r| !r.is_error)
        .map(|r| HistoryEntry {
            filename: r.filename.clone(),
            original_size: r.original_size,
            compressed_size: r.compressed_size,
            reduction: r.reduction,
            output_path: r.output_path.clone(),
            timestamp: chrono_now(),
        })
        .collect();
    if !history_entries.is_empty() {
        append_history(&history_entries);
    }

    final_results
}

#[tauri::command]
fn get_output_dir() -> String {
    let config = load_config();
    match config.output_dir {
        Some(ref d) if !d.is_empty() => d.clone(),
        _ => default_output_dir().to_string_lossy().to_string(),
    }
}

#[tauri::command]
fn set_output_dir(path: String) {
    let mut config = load_config();
    config.output_dir = Some(path);
    save_config(&config);
}

// ---------------------------------------------------------------------------
// Compression history
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HistoryEntry {
    filename: String,
    original_size: u64,
    compressed_size: u64,
    reduction: f64,
    output_path: String,
    timestamp: String,
}

fn history_path() -> PathBuf {
    let dir = config_path().parent().unwrap_or(Path::new("/tmp")).to_path_buf();
    let _ = fs::create_dir_all(&dir);
    dir.join("history.json")
}

fn load_history() -> Vec<HistoryEntry> {
    let path = history_path();
    if let Ok(data) = fs::read_to_string(&path) {
        serde_json::from_str(&data).unwrap_or_default()
    } else {
        Vec::new()
    }
}

fn save_history(history: &[HistoryEntry]) {
    let path = history_path();
    if let Ok(json) = serde_json::to_string_pretty(history) {
        let _ = fs::write(&path, json);
    }
}

fn append_history(entries: &[HistoryEntry]) {
    let mut history = load_history();
    history.extend_from_slice(entries);
    save_history(&history);
}

#[tauri::command]
fn get_history() -> Vec<HistoryEntry> {
    load_history()
}

#[tauri::command]
fn clear_history() {
    save_history(&[]);
}

#[tauri::command]
fn delete_history_entries(indices: Vec<usize>) -> Vec<HistoryEntry> {
    let mut history = load_history();
    // Sort indices in reverse to remove from end first
    let mut sorted = indices;
    sorted.sort_unstable();
    sorted.dedup();
    for &i in sorted.iter().rev() {
        if i < history.len() {
            history.remove(i);
        }
    }
    save_history(&history);
    history
}

#[tauri::command]
fn export_history_csv() -> Result<String, String> {
    let history = load_history();
    let out_dir = default_output_dir();
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
