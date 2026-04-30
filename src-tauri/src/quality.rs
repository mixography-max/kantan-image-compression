use std::fs;
use std::path::Path;
use image::{GrayImage, RgbaImage};

use crate::Logger;
use crate::compress::{compress_jpeg, compress_png};

// ---------------------------------------------------------------------------
// SSIM calculation (luminance-based structural similarity)
// ---------------------------------------------------------------------------

/// Compute SSIM from pre-loaded grayscale images (avoids redundant I/O).
fn ssim_from_images(img_a: &GrayImage, img_b: &GrayImage) -> f64 {
    let (w_a, h_a) = img_a.dimensions();
    let (w_b, h_b) = img_b.dimensions();

    let img_b_resized;
    let img_b_ref = if w_a != w_b || h_a != h_b {
        img_b_resized = image::imageops::resize(img_b, w_a, h_a, image::imageops::FilterType::Lanczos3);
        &img_b_resized
    } else {
        img_b
    };

    let (width, height) = (w_a as usize, h_a as usize);
    let pixels_a: &[u8] = img_a.as_raw();
    let pixels_b: &[u8] = img_b_ref.as_raw();

    // SSIM constants (for 8-bit images)
    let c1: f64 = (0.01 * 255.0) * (0.01 * 255.0); // 6.5025
    let c2: f64 = (0.03 * 255.0) * (0.03 * 255.0); // 58.5225

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
        return 1.0;
    }
    ssim_sum / block_count as f64
}

/// Compute SSIM between two image files.
#[allow(dead_code)]
pub fn compute_ssim(original: &Path, compressed: &Path) -> Result<f64, String> {
    let img_a = image::open(original)
        .map_err(|e| format!("元画像の読み込み失敗: {}", e))?
        .to_luma8();
    let img_b = image::open(compressed)
        .map_err(|e| format!("圧縮画像の読み込み失敗: {}", e))?
        .to_luma8();
    Ok(ssim_from_images(&img_a, &img_b))
}

// ---------------------------------------------------------------------------
// CIEDE2000 Color Difference (ΔE)
// ---------------------------------------------------------------------------

fn srgb_to_linear(v: f64) -> f64 {
    let s = v / 255.0;
    if s <= 0.04045 {
        s / 12.92
    } else {
        ((s + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_rgb_to_xyz(r: f64, g: f64, b: f64) -> (f64, f64, f64) {
    let x = 0.4124564 * r + 0.3575761 * g + 0.1804375 * b;
    let y = 0.2126729 * r + 0.7151522 * g + 0.0721750 * b;
    let z = 0.0193339 * r + 0.1191920 * g + 0.9503041 * b;
    (x, y, z)
}

fn xyz_to_lab(x: f64, y: f64, z: f64) -> (f64, f64, f64) {
    const XN: f64 = 0.95047;
    const YN: f64 = 1.00000;
    const ZN: f64 = 1.08883;

    fn f(t: f64) -> f64 {
        const DELTA: f64 = 6.0 / 29.0;
        if t > DELTA * DELTA * DELTA {
            t.cbrt()
        } else {
            t / (3.0 * DELTA * DELTA) + 4.0 / 29.0
        }
    }

    let fx = f(x / XN);
    let fy = f(y / YN);
    let fz = f(z / ZN);

    let l = 116.0 * fy - 16.0;
    let a = 500.0 * (fx - fy);
    let b = 200.0 * (fy - fz);
    (l, a, b)
}

fn srgb_to_lab(r: u8, g: u8, b: u8) -> (f64, f64, f64) {
    let rl = srgb_to_linear(r as f64);
    let gl = srgb_to_linear(g as f64);
    let bl = srgb_to_linear(b as f64);
    let (x, y, z) = linear_rgb_to_xyz(rl, gl, bl);
    xyz_to_lab(x, y, z)
}

fn ciede2000(l1: f64, a1: f64, b1: f64, l2: f64, a2: f64, b2: f64) -> f64 {
    use std::f64::consts::PI;

    let c1_ab = (a1 * a1 + b1 * b1).sqrt();
    let c2_ab = (a2 * a2 + b2 * b2).sqrt();
    let c_ab_mean = (c1_ab + c2_ab) / 2.0;

    let c_ab_mean_pow7 = c_ab_mean.powi(7);
    let g = 0.5 * (1.0 - (c_ab_mean_pow7 / (c_ab_mean_pow7 + 25.0_f64.powi(7))).sqrt());

    let a1_prime = a1 * (1.0 + g);
    let a2_prime = a2 * (1.0 + g);

    let c1_prime = (a1_prime * a1_prime + b1 * b1).sqrt();
    let c2_prime = (a2_prime * a2_prime + b2 * b2).sqrt();

    let h1_prime = b1.atan2(a1_prime).to_degrees();
    let h1_prime = if h1_prime < 0.0 { h1_prime + 360.0 } else { h1_prime };
    let h2_prime = b2.atan2(a2_prime).to_degrees();
    let h2_prime = if h2_prime < 0.0 { h2_prime + 360.0 } else { h2_prime };

    let dl_prime = l2 - l1;
    let dc_prime = c2_prime - c1_prime;

    let dh_prime_raw = if c1_prime * c2_prime == 0.0 {
        0.0
    } else if (h2_prime - h1_prime).abs() <= 180.0 {
        h2_prime - h1_prime
    } else if h2_prime - h1_prime > 180.0 {
        h2_prime - h1_prime - 360.0
    } else {
        h2_prime - h1_prime + 360.0
    };
    let dh_prime = 2.0 * (c1_prime * c2_prime).sqrt() * (dh_prime_raw / 2.0 * PI / 180.0).sin();

    let l_prime_mean = (l1 + l2) / 2.0;
    let c_prime_mean = (c1_prime + c2_prime) / 2.0;

    let h_prime_mean = if c1_prime * c2_prime == 0.0 {
        h1_prime + h2_prime
    } else if (h1_prime - h2_prime).abs() <= 180.0 {
        (h1_prime + h2_prime) / 2.0
    } else if h1_prime + h2_prime < 360.0 {
        (h1_prime + h2_prime + 360.0) / 2.0
    } else {
        (h1_prime + h2_prime - 360.0) / 2.0
    };

    let t = 1.0
        - 0.17 * ((h_prime_mean - 30.0) * PI / 180.0).cos()
        + 0.24 * ((2.0 * h_prime_mean) * PI / 180.0).cos()
        + 0.32 * ((3.0 * h_prime_mean + 6.0) * PI / 180.0).cos()
        - 0.20 * ((4.0 * h_prime_mean - 63.0) * PI / 180.0).cos();

    let sl = 1.0 + 0.015 * (l_prime_mean - 50.0).powi(2) / (20.0 + (l_prime_mean - 50.0).powi(2)).sqrt();
    let sc = 1.0 + 0.045 * c_prime_mean;
    let sh = 1.0 + 0.015 * c_prime_mean * t;

    let c_prime_mean_pow7 = c_prime_mean.powi(7);
    let rt_term = -2.0 * (c_prime_mean_pow7 / (c_prime_mean_pow7 + 25.0_f64.powi(7))).sqrt()
        * (60.0 * (-((h_prime_mean - 275.0) / 25.0).powi(2)).exp() * PI / 180.0).sin();

    let result = ((dl_prime / sl).powi(2)
        + (dc_prime / sc).powi(2)
        + (dh_prime / sh).powi(2)
        + rt_term * (dc_prime / sc) * (dh_prime / sh))
        .sqrt();

    if result.is_nan() { 0.0 } else { result }
}

/// Compute ΔE from pre-loaded RGBA images (avoids redundant I/O).
fn delta_e_from_images(img_a: &RgbaImage, img_b: &RgbaImage) -> (f64, f64) {
    let (w_a, h_a) = img_a.dimensions();
    let (w_b, h_b) = img_b.dimensions();

    let img_b_resized;
    let img_b_ref = if w_a != w_b || h_a != h_b {
        img_b_resized = image::imageops::resize(img_b, w_a, h_a, image::imageops::FilterType::Lanczos3);
        &img_b_resized
    } else {
        img_b
    };

    let pixels_a = img_a.as_raw();
    let pixels_b = img_b_ref.as_raw();
    let total_pixels = (w_a * h_a) as usize;

    const SAMPLE_STEP: usize = 4;
    let mut sum_de = 0.0f64;
    let mut max_de = 0.0f64;
    let mut count = 0u64;

    for i in (0..total_pixels).step_by(SAMPLE_STEP) {
        let idx = i * 4;
        if idx + 3 >= pixels_a.len() || idx + 3 >= pixels_b.len() {
            break;
        }

        let (l1, a1, b1) = srgb_to_lab(pixels_a[idx], pixels_a[idx + 1], pixels_a[idx + 2]);
        let (l2, a2, b2) = srgb_to_lab(pixels_b[idx], pixels_b[idx + 1], pixels_b[idx + 2]);

        let de = ciede2000(l1, a1, b1, l2, a2, b2);
        sum_de += de;
        if de > max_de {
            max_de = de;
        }
        count += 1;
    }

    if count == 0 {
        return (0.0, 0.0);
    }
    (sum_de / count as f64, max_de)
}

/// Compute average and max CIEDE2000 ΔE between two image files.
#[allow(dead_code)]
pub fn compute_delta_e(original: &Path, compressed: &Path) -> Result<(f64, f64), String> {
    let img_a = image::open(original)
        .map_err(|e| format!("元画像の読み込み失敗: {}", e))?
        .to_rgba8();
    let img_b = image::open(compressed)
        .map_err(|e| format!("圧縮画像の読み込み失敗: {}", e))?
        .to_rgba8();
    Ok(delta_e_from_images(&img_a, &img_b))
}

// ---------------------------------------------------------------------------
// Auto-quality search (A-2: single image load per iteration)
// ---------------------------------------------------------------------------

/// Auto-quality search for JPEG using binary search + SSIM.
/// Target: SSIM >= 0.95 with minimum file size.
pub fn auto_quality_jpeg(
    src: &Path,
    dst: &Path,
    progressive: bool,
    strip_meta: bool,
    logger: &dyn Logger,
    filename: &str,
) -> Result<u32, String> {
    let target_ssim: f64 = 0.95;
    let mut lo: u32 = 30;
    let mut hi: u32 = 95;
    let mut best_q: u32 = 85;
    let mut iteration = 0;

    // Pre-load original image once (A-2 optimization)
    let orig_img = image::open(src)
        .map_err(|e| format!("元画像の読み込み失敗: {}", e))?;
    let orig_gray = orig_img.to_luma8();

    logger.log(&format!(
        "🔍 {} — SSIM自動品質探索開始 (目標: SSIM ≥ {:.2})", filename, target_ssim
    ));

    while lo <= hi {
        let mid = (lo + hi) / 2;
        iteration += 1;

        compress_jpeg(src, dst, mid, progressive, strip_meta)?;

        // Load compressed image once per iteration
        let comp_img = image::open(dst)
            .map_err(|e| format!("圧縮画像の読み込み失敗: {}", e))?;
        let comp_gray = comp_img.to_luma8();
        let ssim = ssim_from_images(&orig_gray, &comp_gray);

        let size = fs::metadata(dst).map(|m| m.len()).unwrap_or(0);
        let size_kb = size as f64 / 1024.0;

        let status = if ssim >= target_ssim { "✅" } else { "⚠️" };
        logger.log(&format!(
            "  #{} Q={:3} → SSIM={:.4} {} ({:.0}KB)",
            iteration, mid, ssim, status, size_kb
        ));

        if ssim >= target_ssim {
            best_q = mid;
            hi = mid.saturating_sub(1);
        } else {
            lo = mid + 1;
        }

        if lo > hi { break; }
    }

    // Final compress at best quality (skip if last iteration already used best_q — A-8)
    let needs_final = iteration == 0 || best_q != (lo + hi + 1) / 2;
    if needs_final {
        compress_jpeg(src, dst, best_q, progressive, strip_meta)?;
    }
    let final_ssim = {
        let comp = image::open(dst)
            .map_err(|e| format!("圧縮画像の読み込み失敗: {}", e))?
            .to_luma8();
        ssim_from_images(&orig_gray, &comp)
    };
    let final_size = fs::metadata(dst).map(|m| m.len()).unwrap_or(0);

    logger.log(&format!(
        "🎯 {} — 最適品質 Q={} (SSIM={:.4}, {:.0}KB)",
        filename, best_q, final_ssim, final_size as f64 / 1024.0
    ));

    Ok(best_q)
}

/// Auto-quality search for PNG using binary search over color count.
/// Uses composite evaluation: SSIM + CIEDE2000 ΔE.
/// Target: SSIM ≥ 0.98 AND avg ΔE ≤ 1.5 AND max ΔE ≤ 5.0
pub fn auto_quality_png(
    src: &Path,
    dst: &Path,
    logger: &dyn Logger,
    filename: &str,
) -> Result<u32, String> {
    let target_ssim: f64 = 0.98;
    let target_avg_de: f64 = 1.5;
    let target_max_de: f64 = 5.0;
    let mut lo: u32 = 32;
    let mut hi: u32 = 256;
    let mut best_c: u32 = 256;
    let mut iteration = 0;

    // Pre-load original image once (A-2 optimization)
    let orig_img = image::open(src)
        .map_err(|e| format!("元画像の読み込み失敗: {}", e))?;
    let orig_gray = orig_img.to_luma8();
    let orig_rgba = orig_img.to_rgba8();

    logger.log(&format!(
        "🔍 {} — PNG自動色数探索開始 (目標: SSIM≥{:.2}, ΔE avg≤{:.1}, max≤{:.1})",
        filename, target_ssim, target_avg_de, target_max_de
    ));

    while lo <= hi {
        let mid = (lo + hi) / 2;
        iteration += 1;

        compress_png(src, dst, mid)?;

        // Load compressed image once, compute both metrics (A-2 optimization)
        let comp_img = image::open(dst)
            .map_err(|e| format!("圧縮画像の読み込み失敗: {}", e))?;
        let comp_gray = comp_img.to_luma8();
        let comp_rgba = comp_img.to_rgba8();

        let ssim = ssim_from_images(&orig_gray, &comp_gray);
        let (avg_de, max_de) = delta_e_from_images(&orig_rgba, &comp_rgba);
        let size = fs::metadata(dst).map(|m| m.len()).unwrap_or(0);
        let size_kb = size as f64 / 1024.0;

        let passes_all = ssim >= target_ssim && avg_de <= target_avg_de && max_de <= target_max_de;

        let status = if passes_all { "✅" } else { "⚠️" };
        logger.log(&format!(
            "  #{} 色数={:3} → SSIM={:.4} ΔE(avg={:.2}, max={:.1}) {} ({:.0}KB)",
            iteration, mid, ssim, avg_de, max_de, status, size_kb
        ));

        if passes_all {
            best_c = mid;
            hi = mid.saturating_sub(1);
        } else {
            lo = mid + 1;
        }

        if lo > hi { break; }
    }

    // Final compress at best color count
    compress_png(src, dst, best_c)?;
    let comp_img = image::open(dst)
        .map_err(|e| format!("圧縮画像の読み込み失敗: {}", e))?;
    let final_ssim = ssim_from_images(&orig_gray, &comp_img.to_luma8());
    let (final_avg_de, final_max_de) = delta_e_from_images(&orig_rgba, &comp_img.to_rgba8());
    let final_size = fs::metadata(dst).map(|m| m.len()).unwrap_or(0);

    logger.log(&format!(
        "🎯 {} — 最適色数 {} (SSIM={:.4}, ΔE avg={:.2}/max={:.1}, {:.0}KB)",
        filename, best_c, final_ssim, final_avg_de, final_max_de, final_size as f64 / 1024.0
    ));

    Ok(best_c)
}
