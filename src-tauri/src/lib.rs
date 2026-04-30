use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use serde::{Deserialize, Serialize};
use rayon::prelude::*;

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

        // Second pass: ECT (Efficient Compression Tool) for lossless optimization
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
fn compress(inputs: Vec<String>, settings: CompressSettings) -> Vec<CompressResult> {
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
        let out_name = format!("{}_compressed.{}", stem, ext);
        let dst = out_dir.join(&out_name);

        // Resize if needed (images only)
        let resized_path = resize_if_needed(src, max_w, max_h).ok().flatten();
        let actual_src = resized_path.as_deref().unwrap_or(src);

        let compress_result = match ext.as_str() {
            "jpg" | "jpeg" => compress_jpeg(actual_src, &dst, jpeg_q, progressive, strip_meta),
            "png" => compress_png(actual_src, &dst, png_c),
            "pdf" => compress_pdf(src, &dst, pdf_dpi, pdf_jq),
            "docx" | "xlsx" | "pptx" => compress_office(src, &dst, office_q, png_c, &ext),
            _ => Err("未対応".to_string()),
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

    results.into_inner().unwrap()
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
// App entry
// ---------------------------------------------------------------------------

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![compress, get_output_dir, set_output_dir])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
