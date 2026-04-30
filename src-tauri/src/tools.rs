use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

// ---------------------------------------------------------------------------
// Bundled tools directory (cached)
// ---------------------------------------------------------------------------

static BUNDLED_TOOLS_DIR: OnceLock<Option<PathBuf>> = OnceLock::new();

/// Returns the directory where bundled tools are placed at runtime.
/// Result is cached after the first call.
pub fn bundled_tools_dir() -> Option<&'static PathBuf> {
    BUNDLED_TOOLS_DIR.get_or_init(|| {
        let exe = std::env::current_exe().ok()?;
        let exe_dir = exe.parent()?;
        #[cfg(target_os = "macos")]
        {
            let dir = exe_dir.parent()?.join("Resources").join("resources").join("tools");
            if dir.exists() { return Some(dir); }
            Some(exe_dir.parent()?.join("Resources").join("tools"))
        }
        #[cfg(not(target_os = "macos"))]
        {
            let dir = exe_dir.join("resources").join("tools");
            if dir.exists() { return Some(dir); }
            Some(exe_dir.join("tools"))
        }
    }).as_ref()
}

// ---------------------------------------------------------------------------
// Tool resolution (cached)
// ---------------------------------------------------------------------------

static RESOLVED_TOOLS: OnceLock<HashMap<String, String>> = OnceLock::new();

fn resolve_tool_uncached(name: &str) -> String {
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

/// Resolve a tool path with caching. After first resolution, returns cached value.
pub fn resolve_tool(name: &str) -> String {
    let cache = RESOLVED_TOOLS.get_or_init(|| {
        let tool_names = ["cjpegli", "pngquant", "ect", "gs", "gswin64c",
                         "cwebp", "cjxl", "avifenc", "magick",
                         "/usr/bin/sips", "/usr/bin/unzip", "/usr/bin/zip"];
        let mut map = HashMap::new();
        for &t in &tool_names {
            map.insert(t.to_string(), resolve_tool_uncached(t));
        }
        map
    });
    cache.get(name).cloned().unwrap_or_else(|| resolve_tool_uncached(name))
}

pub fn tool_exists(path: &str) -> bool {
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
pub fn is_bundled(path: &str) -> bool {
    if let Some(tools_dir) = bundled_tools_dir() {
        Path::new(path).starts_with(tools_dir)
    } else {
        false
    }
}

/// Build a Command, adding library paths and GS_LIB for bundled tools.
pub fn make_tool_command(exe: &str) -> Command {
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
                    let path = std::env::var("PATH").unwrap_or_default();
                    cmd.env("PATH", format!("{};{}", lib_dir.to_string_lossy(), path));
                }
            }

            // Set GS_LIB for Ghostscript resource files
            let gs_res = tools_dir.join("gsresource");
            if gs_res.exists() {
                let mut gs_lib_paths: Vec<String> = Vec::new();
                gs_lib_paths.push(gs_res.to_string_lossy().to_string());
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

pub fn run_tool(exe: &str, args: &[&str]) -> Result<(), String> {
    run_tool_impl(exe, args, None)
}

pub fn run_tool_with_cwd(exe: &str, args: &[&str], cwd: &str) -> Result<(), String> {
    run_tool_impl(exe, args, Some(cwd))
}

fn run_tool_impl(exe: &str, args: &[&str], cwd: Option<&str>) -> Result<(), String> {
    let mut cmd = make_tool_command(exe);
    cmd.args(args);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let output = cmd.output()
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

/// Debug info about GS_LIB resolution
pub fn gs_lib_debug() -> String {
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
