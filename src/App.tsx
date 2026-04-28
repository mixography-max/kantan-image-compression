// src/App.tsx
import React, { useState, useEffect } from 'react';
import DropZone from './DropZone';
import SettingsPanel from './SettingsPanel';
import ResultCard from './ResultCard';
import { CompressionResult } from './utils';
import { invoke } from '@tauri-apps/api/core';

const App: React.FC = () => {
  const [results, setResults] = useState<CompressionResult[]>([]);
  const [outputDir, setOutputDir] = useState('');
  const [settings, setSettings] = useState({
    jpegQuality: 85,
    pngColors: 256,
    pdfDpi: 235,
    pdfJpegQ: 82,
    officeQuality: 80,
    group: false,
  });

  // Load saved output directory on mount
  useEffect(() => {
    invoke<string>('get_output_dir').then((dir) => {
      setOutputDir(dir);
    }).catch(() => {});
  }, []);

  const handleCompression = (newResults: CompressionResult[]) => {
    setResults(prev => [...newResults, ...prev]);
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
          {results.map((r, i) => (
            <ResultCard key={i} result={r} />
          ))}
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
