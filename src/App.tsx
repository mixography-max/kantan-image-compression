// src/App.tsx
import React, { useState, useEffect } from 'react';
import DropZone from './DropZone';
import SettingsPanel from './SettingsPanel';
import ResultCard from './ResultCard';
import HistoryPanel from './HistoryPanel';
import { CompressionResult } from './utils';
import { invoke } from '@tauri-apps/api/core';

const App: React.FC = () => {
  const [results, setResults] = useState<CompressionResult[]>([]);
  const [outputDir, setOutputDir] = useState('');
  const [historyRefresh, setHistoryRefresh] = useState(0);
  const [settings, setSettings] = useState({
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
  });

  // Load saved output directory on mount
  useEffect(() => {
    invoke<string>('get_output_dir').then((dir) => {
      setOutputDir(dir);
    }).catch(() => {});
  }, []);

  const handleCompression = (newResults: CompressionResult[]) => {
    setResults(prev => [...newResults, ...prev]);
    setHistoryRefresh(prev => prev + 1); // trigger history reload
  };

  const handleSettingsChange = (newSettings: typeof settings) => {
    setSettings(newSettings);
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
            </dl>
          </div>
          {results.map((r, i) => (
            <ResultCard key={i} result={r} />
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
