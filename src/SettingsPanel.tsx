// src/SettingsPanel.tsx
import React from 'react';
import { open } from '@tauri-apps/plugin-dialog';

interface Settings {
  jpegQuality: number;
  pngColors: number;
  pdfDpi: number;
  pdfJpegQ: number;
  officeQuality: number;
  group: boolean;
  progressiveJpeg: boolean;
  stripMetadata: boolean;
  maxWidth: number;
  maxHeight: number;
}

interface Props {
  settings: Settings;
  onChange: (newSettings: Settings) => void;
  outputDir: string;
  onOutputDirChange: (dir: string) => void;
}

const PDF_PRESETS = [
  { label: '高品質 (460dpi / Q90)', dpi: 460, q: 90 },
  { label: '⭐ 推奨 (235dpi / Q82)', dpi: 235, q: 82 },
  { label: '標準 (200dpi / Q82)', dpi: 200, q: 82 },
  { label: '軽量 (150dpi / Q75)', dpi: 150, q: 75 },
  { label: '最小 (72dpi / Q70)', dpi: 72, q: 70 },
];

const SettingsPanel: React.FC<Props> = ({ settings, onChange, outputDir, onOutputDirChange }) => {
  const handleChange = (e: React.ChangeEvent<HTMLInputElement | HTMLSelectElement>) => {
    const target = e.target as HTMLInputElement | HTMLSelectElement;
    const { name, value, type } = target;
    let newVal: any;
    if (type === 'checkbox') {
      newVal = (target as HTMLInputElement).checked;
    } else {
      newVal = Number(value);
    }
    onChange({ ...settings, [name]: newVal } as any);
  };

  const handlePreset = (e: React.ChangeEvent<HTMLSelectElement>) => {
    const preset = PDF_PRESETS.find(p => p.label === e.target.value);
    if (preset) {
      onChange({ ...settings, pdfDpi: preset.dpi, pdfJpegQ: preset.q });
    }
  };

  const handlePickOutputDir = async () => {
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: '保存先フォルダを選択',
      });
      if (selected && typeof selected === 'string') {
        onOutputDirChange(selected);
      }
    } catch (e) {
      console.error('Folder picker error', e);
    }
  };

  // Check if current values match a preset
  const currentPresetLabel = PDF_PRESETS.find(
    p => p.dpi === settings.pdfDpi && p.q === settings.pdfJpegQ
  )?.label ?? '';

  // Display shortened path
  const displayDir = outputDir
    ? outputDir.replace(/^\/Users\/[^/]+/, '~')
    : '~/Desktop/圧縮済み';

  return (
    <div className="right">
      <h2>設定</h2>

      <div className="setting-item output-dir-setting">
        <label>📁 保存先フォルダ</label>
        <div className="output-dir-row">
          <span className="output-dir-path" title={outputDir}>{displayDir}</span>
          <button className="output-dir-btn" onClick={handlePickOutputDir}>変更</button>
        </div>
      </div>

      <div className="setting-group">
        <label className="setting-group-title">📸 JPEG 設定</label>
        <div className="setting-item">
          <label>品質</label>
          <input
            type="range"
            name="jpegQuality"
            min={10}
            max={100}
            value={settings.jpegQuality}
            onChange={handleChange}
          />
          <span>{settings.jpegQuality}%</span>
        </div>
        <div className="setting-item">
          <label>
            <input
              type="checkbox"
              name="progressiveJpeg"
              checked={settings.progressiveJpeg}
              onChange={handleChange}
            />
            プログレッシブ JPEG
          </label>
        </div>
      </div>

      <div className="setting-item">
        <label>🎨 PNG 色数</label>
        <input
          type="number"
          name="pngColors"
          min={2}
          max={256}
          value={settings.pngColors}
          onChange={handleChange}
        />
      </div>

      <div className="setting-group">
        <label className="setting-group-title">📄 PDF 圧縮設定</label>
        <div className="setting-item">
          <label>プリセット</label>
          <select value={currentPresetLabel} onChange={handlePreset}>
            <option value="" disabled>カスタム</option>
            {PDF_PRESETS.map(p => (
              <option key={p.label} value={p.label}>{p.label}</option>
            ))}
          </select>
        </div>
        <div className="setting-item">
          <label>解像度 (DPI)</label>
          <input
            type="range"
            name="pdfDpi"
            min={72}
            max={600}
            step={1}
            value={settings.pdfDpi}
            onChange={handleChange}
          />
          <span>{settings.pdfDpi} dpi</span>
        </div>
        <div className="setting-item">
          <label>画像品質 (JPEG Q)</label>
          <input
            type="range"
            name="pdfJpegQ"
            min={10}
            max={100}
            value={settings.pdfJpegQ}
            onChange={handleChange}
          />
          <span>{settings.pdfJpegQ}%</span>
        </div>
      </div>

      <div className="setting-item">
        <label>📎 Office内画像 JPEG品質</label>
        <input
          type="range"
          name="officeQuality"
          min={10}
          max={100}
          value={settings.officeQuality}
          onChange={handleChange}
        />
        <span>{settings.officeQuality}%</span>
      </div>

      <div className="setting-group">
        <label className="setting-group-title">🔧 共通オプション</label>
        <div className="setting-item">
          <label>
            <input
              type="checkbox"
              name="stripMetadata"
              checked={settings.stripMetadata}
              onChange={handleChange}
            />
            EXIF / メタデータを削除
          </label>
        </div>
        <div className="setting-item">
          <label>📐 長辺の最大サイズ (px)</label>
          <input
            type="number"
            name="maxWidth"
            min={0}
            max={10000}
            step={100}
            value={settings.maxWidth}
            onChange={handleChange}
            placeholder="0 = リサイズなし"
          />
          <span className="setting-hint">{settings.maxWidth === 0 ? 'リサイズなし' : `${settings.maxWidth}px`}</span>
        </div>
        <div className="setting-item">
          <label>
            <input
              type="checkbox"
              name="group"
              checked={settings.group}
              onChange={handleChange}
            />
            ファイルをフォルダにまとめる
          </label>
        </div>
      </div>
    </div>
  );
};

export default SettingsPanel;
