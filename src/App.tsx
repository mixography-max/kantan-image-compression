// src/App.tsx
import React, { useState } from 'react';
import DropZone from './DropZone';
import SettingsPanel from './SettingsPanel';
import ResultCard from './ResultCard';
import { CompressionResult } from './utils';

const App: React.FC = () => {
  const [results, setResults] = useState<CompressionResult[]>([]);
  const [settings, setSettings] = useState({
    jpegQuality: 85,
    pngColors: 256,
    pdfPreset: '235,82',
    officeQuality: 80,
    group: false,
  });

  const handleCompression = (newResults: CompressionResult[]) => {
    setResults(prev => [...newResults, ...prev]);
  };

  const handleSettingsChange = (newSettings: typeof settings) => {
    setSettings(newSettings);
  };

  return (
    <div className="app">
      <header>
        <h1>かんたん画像圧縮くん</h1>
        <span className="sub">ドラッグ&ドロップで画像・PDF・Office ファイルを圧縮</span>
      </header>
      <div className="main">
        <div className="left">
          <DropZone settings={settings} onComplete={handleCompression} />
          {results.map((r, i) => (
            <ResultCard key={i} result={r} />
          ))}
        </div>
        <div className="right">
          <SettingsPanel settings={settings} onChange={handleSettingsChange} />
        </div>
      </div>
    </div>
  );
};

export default App;
