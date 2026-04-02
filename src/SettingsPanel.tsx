// src/SettingsPanel.tsx
import React from 'react';

interface Settings {
  jpegQuality: number;
  pngColors: number;
  pdfPreset: string;
  officeQuality: number;
  group: boolean;
}

interface Props {
  settings: Settings;
  onChange: (newSettings: Settings) => void;
}

const SettingsPanel: React.FC<Props> = ({ settings, onChange }) => {
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

  return (
    <div className="right">
      <h2>設定</h2>
      <div className="setting-item">
        <label>JPEG 品質</label>
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
        <label>PNG 色数</label>
        <input
          type="number"
          name="pngColors"
          min={2}
          max={256}
          value={settings.pngColors}
          onChange={handleChange}
        />
      </div>
      <div className="setting-item">
        <label>PDF プリセット (解像度,品質)</label>
        <input
          type="text"
          name="pdfPreset"
          value={settings.pdfPreset}
          onChange={handleChange}
        />
      </div>
      <div className="setting-item">
        <label>Office 品質</label>
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
  );
};

export default SettingsPanel;
