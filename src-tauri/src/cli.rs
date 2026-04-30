use clap::Parser;
use kan_tan_image_compression_kun_lib::{CompressSettings, StderrLogger, compress_files};

/// 🐹 はむはむ画像圧縮くん CLI — 高品質画像・PDF圧縮ツール
#[derive(Parser, Debug)]
#[command(name = "hamham-compress", version, about, long_about = None)]
struct Cli {
    /// 入力ファイルまたはフォルダ（複数指定可）
    #[arg(required = true)]
    files: Vec<String>,

    /// JPEG/WebP/AVIF 品質 (10-100)
    #[arg(short = 'q', long = "quality", default_value_t = 85)]
    quality: u32,

    /// PNG 色数 (2-256)
    #[arg(short = 'c', long = "png-colors", default_value_t = 256)]
    png_colors: u32,

    /// PDF 解像度 (DPI)
    #[arg(long = "pdf-dpi", default_value_t = 235)]
    pdf_dpi: u32,

    /// PDF 内 JPEG 品質
    #[arg(long = "pdf-quality", default_value_t = 82)]
    pdf_quality: u32,

    /// 出力ディレクトリ
    #[arg(short = 'o', long = "output")]
    output: Option<String>,

    /// WebP 変換モード
    #[arg(long = "webp")]
    webp: bool,

    /// AVIF 変換モード
    #[arg(long = "avif")]
    avif: bool,

    /// JPEG XL 変換モード
    #[arg(long = "jxl")]
    jxl: bool,

    /// JXL ロスレスモード (JPEG→JXL のみ)
    #[arg(long = "jxl-lossless", default_value_t = true)]
    jxl_lossless: bool,

    /// SSIM 自動品質（おまかせモード）
    #[arg(short = 'a', long = "auto-quality")]
    auto_quality: bool,

    /// 目標ファイルサイズ (KB, PDF用)
    #[arg(short = 's', long = "target-size", default_value_t = 0)]
    target_size: u64,

    /// 最大幅 (0=無制限)
    #[arg(short = 'W', long = "max-width", default_value_t = 0)]
    max_width: u32,

    /// 最大高さ (0=無制限)
    #[arg(short = 'H', long = "max-height", default_value_t = 0)]
    max_height: u32,

    /// プログレッシブ JPEG を無効化
    #[arg(long = "no-progressive")]
    no_progressive: bool,

    /// メタデータ削除を無効化
    #[arg(long = "keep-metadata")]
    keep_metadata: bool,
}

fn format_size(bytes: u64) -> String {
    if bytes >= 1_048_576 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.0} KB", bytes as f64 / 1024.0)
    } else {
        format!("{} B", bytes)
    }
}

fn main() {
    let cli = Cli::parse();

    eprintln!("🐹 はむはむ画像圧縮くん CLI v{}\n", env!("CARGO_PKG_VERSION"));

    let settings = CompressSettings {
        jpeg_quality: Some(cli.quality),
        png_colors: Some(cli.png_colors),
        pdf_dpi: Some(cli.pdf_dpi),
        pdf_jpeg_q: Some(cli.pdf_quality),
        office_quality: Some(cli.quality),
        output_dir: cli.output,
        strip_metadata: Some(!cli.keep_metadata),
        progressive_jpeg: Some(!cli.no_progressive),
        max_width: Some(cli.max_width),
        max_height: Some(cli.max_height),
        convert_webp: Some(cli.webp),
        target_size_kb: Some(cli.target_size),
        convert_jxl: Some(cli.jxl),
        jxl_lossless: Some(cli.jxl_lossless),
        convert_avif: Some(cli.avif),
        auto_quality: Some(cli.auto_quality),
    };

    let logger = StderrLogger;
    let results = compress_files(&cli.files, &settings, &logger);

    // Print results
    let mut success_count = 0u32;
    let mut fail_count = 0u32;
    let mut total_original: u64 = 0;
    let mut total_compressed: u64 = 0;

    eprintln!();

    for r in &results {
        if r.is_error {
            fail_count += 1;
            eprintln!(
                "⚠️  {} — {}",
                r.filename,
                r.error_message.as_deref().unwrap_or("不明なエラー")
            );
        } else {
            success_count += 1;
            total_original += r.original_size;
            total_compressed += r.compressed_size;
            println!(
                "✅ {} → {} ({} → {}, {:.1}%)",
                r.filename,
                r.output_filename,
                format_size(r.original_size),
                format_size(r.compressed_size),
                r.reduction
            );
        }
    }

    // Summary
    let total = success_count + fail_count;
    let total_reduction = if total_original > 0 {
        (1.0 - total_compressed as f64 / total_original as f64) * 100.0
    } else {
        0.0
    };

    eprintln!();
    eprintln!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    eprintln!(
        "  処理: {} ファイル | 成功: {} | 失敗: {}",
        total, success_count, fail_count
    );
    if success_count > 0 {
        eprintln!(
            "  総削減: {} → {} ({:.1}%)",
            format_size(total_original),
            format_size(total_compressed),
            total_reduction
        );
    }
    eprintln!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    if fail_count > 0 {
        std::process::exit(1);
    }
}
