// src/App.tsx
import React, { useState, useEffect } from 'react';
import DropZone from './DropZone';
import SettingsPanel from './SettingsPanel';
import ResultCard from './ResultCard';
import HistoryPanel from './HistoryPanel';
import { CompressionResult, Settings } from './utils';
import { invoke } from '@tauri-apps/api/core';
import { revealItemInDir, openPath } from '@tauri-apps/plugin-opener';

const SETTINGS_STORAGE_KEY = 'hamham_settings';

const DEFAULT_SETTINGS: Settings = {
  jpegQuality: 85,
  pngColors: 256,
  pdfDpi: 235,
  pdfJpegQ: 82,
  officeQuality: 80,
  group: false,
  progressiveJpeg: true,
  stripMetadata: true,
  maxWidth: 0,
  maxHeight: 0,
  convertWebp: false,
  targetSizeKb: 0,
  convertJxl: false,
  jxlLossless: true,
  convertAvif: false,
  autoQuality: false,
  openFolderOnComplete: true,
};

const App: React.FC = () => {
  const [results, setResults] = useState<CompressionResult[]>([]);
  const [outputDir, setOutputDir] = useState('');
  const [historyRefresh, setHistoryRefresh] = useState(0);
  const [settings, setSettings] = useState<Settings>(() => {
    try {
      const saved = localStorage.getItem(SETTINGS_STORAGE_KEY);
      if (saved) {
        return { ...DEFAULT_SETTINGS, ...JSON.parse(saved) };
      }
    } catch {}
    return DEFAULT_SETTINGS;
  });

  // Load saved output directory on mount
  useEffect(() => {
    invoke<string>('get_output_dir').then((dir) => {
      setOutputDir(dir);
    }).catch(() => {});
  }, []);

  const handleCompression = async (newResults: CompressionResult[]) => {
    setResults(prev => [...newResults, ...prev]);
    setHistoryRefresh(prev => prev + 1); // trigger history reload

    // 保存が成功したら、保存先のフォルダを開く
    if (settings.openFolderOnComplete !== false && newResults.length > 0) {
      const validPaths = newResults.map(r => r.outputPath).filter(Boolean);
      if (validPaths.length > 0) {
        try {
          await revealItemInDir(validPaths.length === 1 ? validPaths[0] : validPaths);
        } catch {
          try {
            const first = validPaths[0];
            const lastSep = Math.max(first.lastIndexOf('/'), first.lastIndexOf('\\'));
            if (lastSep > 0) {
              await openPath(first.substring(0, lastSep));
            }
          } catch (e) {
            console.error('Failed to open destination folder:', e);
          }
        }
      }
    }
  };

  const handleSettingsChange = (newSettings: Settings) => {
    setSettings(newSettings);
    try {
      localStorage.setItem(SETTINGS_STORAGE_KEY, JSON.stringify(newSettings));
    } catch {}
  };

  const handleOutputDirChange = (dir: string) => {
    setOutputDir(dir);
    // Persist to config file via Rust
    invoke('set_output_dir', { path: dir }).catch(() => {});
  };

  return (
    <div className="app">
      <header>
        <h1>🐹 はむはむ画像圧縮くん</h1>
        <span className="sub">ひまわりの種みたいにギュッと小さくするよ！🌻</span>
      </header>
      <div className="main">
        <div className="left">
          <DropZone settings={settings} outputDir={outputDir} onComplete={handleCompression} />
          <div className="algo-info">
            <h3>🔧 圧縮アルゴリズム</h3>
            <dl>
              <dt>📸 JPEG</dt>
              <dd><strong>Jpegli</strong>（Google開発）— XYB色空間 + プログレッシブスキャンで高品質・高圧縮を実現</dd>
              <dt>🎨 PNG</dt>
              <dd><strong>pngquant</strong>（減色）+ <strong>ECT</strong>（ロスレス再圧縮）の2段階最適化で最高水準のPNG圧縮</dd>
              <dt>📄 PDF</dt>
              <dd><strong>Ghostscript</strong> — 画像のダウンサンプリングとJPEG再圧縮でPDFを軽量化</dd>
              <dt>🖼️ JPEG XL</dt>
              <dd><strong>cjxl</strong>（libjxl）— JPEGの正式な後継規格。ロスレスJPEG変換で完全復元可能</dd>
              <dt>🌟 AVIF</dt>
              <dd><strong>avifenc</strong>（libavif）— AV1ベースの次世代画像フォーマット。Web利用可能な形式で最高の圧縮率</dd>
            </dl>
          </div>
          {results.map((r, i) => (
            <ResultCard key={r.outputPath || i} result={r} />
          ))}
          <HistoryPanel refreshTrigger={historyRefresh} />
        </div>
        <div className="right">
          <SettingsPanel
            settings={settings}
            onChange={handleSettingsChange}
            outputDir={outputDir}
            onOutputDirChange={handleOutputDirChange}
          />
        </div>
      </div>
    </div>
  );
};

export default App;
