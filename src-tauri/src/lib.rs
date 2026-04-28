use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Settings & Result types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CompressSettings {
    jpeg_quality: Option<u32>,
    png_colors: Option<u32>,
    pdf_dpi: Option<u32>,
    pdf_jpeg_q: Option<u32>,
    office_quality: Option<u32>,
    output_dir: Option<String>,
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
                // Collect all subdirectories that might contain gs init/resource files
                let mut gs_lib_paths: Vec<String> = Vec::new();
                // macOS: gsresource/<version>/ contains Resource, lib
                // Windows: gsresource/Resource, gsresource/lib
                if let Ok(entries) = fs::read_dir(&gs_res) {
                    for entry in entries.flatten() {
                        let p = entry.path();
                        if p.is_dir() {
                            // Add the directory itself and common subdirs
                            let resource = p.join("Resource");
                            let lib = p.join("lib");
                            let init = p.join("Resource").join("Init");
                            if resource.exists() { gs_lib_paths.push(resource.to_string_lossy().to_string()); }
                            if lib.exists() { gs_lib_paths.push(lib.to_string_lossy().to_string()); }
                            if init.exists() { gs_lib_paths.push(init.to_string_lossy().to_string()); }
                            gs_lib_paths.push(p.to_string_lossy().to_string());
                        }
                    }
                }
                if !gs_lib_paths.is_empty() {
                    let sep = if cfg!(windows) { ";" } else { ":" };
                    cmd.env("GS_LIB", gs_lib_paths.join(sep));
                }
            }
        }
    }
    cmd
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
        return Err(format!("{} が失敗: {}", exe, stderr));
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

fn compress_jpeg(src: &Path, dst: &Path, quality: u32) -> Result<(), String> {
    let cjpegli = resolve_tool("cjpegli");
    if tool_exists(&cjpegli) {
        run_tool(&cjpegli, &[
            src.to_str().unwrap(),
            dst.to_str().unwrap(),
            "-q", &quality.to_string(),
        ])
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
        ])
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
        return Err("Ghostscript (gs) が見つかりません。インストールしてください。".to_string());
    }

    let dpi_str = dpi.to_string();
    let jpeg_q_str = jpeg_q.to_string();
    let output_arg = format!("-sOutputFile={}", dst.to_str().unwrap());
    run_tool(&gs, &[
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
    ])
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

    let supported = ["jpg", "jpeg", "png", "pdf", "docx", "xlsx", "pptx"];
    let mut results = Vec::new();

    for input in &inputs {
        let src = Path::new(input);
        let ext = src.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .unwrap_or_default();

        if !supported.contains(&ext.as_str()) {
            results.push(CompressResult {
                filename: src.file_name().unwrap_or_default().to_string_lossy().to_string(),
                output_filename: String::new(),
                output_path: String::new(),
                original_size: 0,
                compressed_size: 0,
                reduction: 0.0,
                is_error: true,
                error_message: Some(format!("未対応のファイル形式: .{}", ext)),
            });
            continue;
        }

        let original_size = fs::metadata(src).map(|m| m.len()).unwrap_or(0);
        let stem = src.file_stem().unwrap_or_default().to_string_lossy();
        let out_name = format!("{}_compressed.{}", stem, ext);
        let dst = out_dir.join(&out_name);

        let compress_result = match ext.as_str() {
            "jpg" | "jpeg" => compress_jpeg(src, &dst, jpeg_q),
            "png" => compress_png(src, &dst, png_c),
            "pdf" => compress_pdf(src, &dst, pdf_dpi, pdf_jq),
            "docx" | "xlsx" | "pptx" => compress_office(src, &dst, office_q, png_c, &ext),
            _ => Err("未対応".to_string()),
        };

        match compress_result {
            Ok(()) => {
                let compressed_size = fs::metadata(&dst).map(|m| m.len()).unwrap_or(0);
                let reduction = if original_size > 0 {
                    (1.0 - compressed_size as f64 / original_size as f64) * 100.0
                } else {
                    0.0
                };
                results.push(CompressResult {
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
                results.push(CompressResult {
                    filename: src.file_name().unwrap_or_default().to_string_lossy().to_string(),
                    output_filename: String::new(),
                    output_path: String::new(),
                    original_size,
                    compressed_size: 0,
                    reduction: 0.0,
                    is_error: true,
                    error_message: Some(e),
                });
            }
        }
    }

    results
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
