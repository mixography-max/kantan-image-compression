// src/HistoryPanel.tsx
import React, { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { formatSize } from './utils';

interface HistoryEntry {
  filename: string;
  originalSize: number;
  compressedSize: number;
  reduction: number;
  outputPath: string;
  timestamp: string;
}

interface Props {
  refreshTrigger: number;
}

const HistoryPanel: React.FC<Props> = ({ refreshTrigger }) => {
  const [history, setHistory] = useState<HistoryEntry[]>([]);
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [isOpen, setIsOpen] = useState(false);

  const loadHistory = async () => {
    try {
      const data = await invoke<HistoryEntry[]>('get_history');
      setHistory(data);
    } catch (e) {
      console.error('Failed to load history', e);
    }
  };

  useEffect(() => {
    loadHistory();
  }, [refreshTrigger]);

  const totalOriginal = history.reduce((s, h) => s + h.originalSize, 0);
  const totalCompressed = history.reduce((s, h) => s + h.compressedSize, 0);
  const totalSaved = totalOriginal - totalCompressed;

  const handleClearAll = async () => {
    if (!confirm('すべての履歴を削除しますか？')) return;
    try {
      await invoke('clear_history');
      setHistory([]);
      setSelected(new Set());
    } catch (e) {
      console.error('Failed to clear history', e);
    }
  };

  const handleDeleteSelected = async () => {
    if (selected.size === 0) return;
    try {
      const data = await invoke<HistoryEntry[]>('delete_history_entries', {
        indices: Array.from(selected),
      });
      setHistory(data);
      setSelected(new Set());
    } catch (e) {
      console.error('Failed to delete entries', e);
    }
  };

  const handleExport = async () => {
    try {
      const path = await invoke<string>('export_history_csv');
      alert(`CSVを書き出しました:\n${path}`);
    } catch (e) {
      console.error('Failed to export CSV', e);
    }
  };

  const toggleSelect = (i: number) => {
    setSelected(prev => {
      const next = new Set(prev);
      if (next.has(i)) next.delete(i);
      else next.add(i);
      return next;
    });
  };

  const toggleSelectAll = () => {
    if (selected.size === history.length) {
      setSelected(new Set());
    } else {
      setSelected(new Set(history.map((_, i) => i)));
    }
  };

  const formatTimestamp = (ts: string) => {
    if (ts.startsWith('unix:')) {
      const secs = parseInt(ts.slice(5), 10);
      const d = new Date(secs * 1000);
      return `${d.getFullYear()}/${(d.getMonth()+1).toString().padStart(2,'0')}/${d.getDate().toString().padStart(2,'0')} ${d.getHours().toString().padStart(2,'0')}:${d.getMinutes().toString().padStart(2,'0')}`;
    }
    return ts;
  };

  return (
    <div className="history-panel">
      <div
        className="history-header"
        onClick={() => { setIsOpen(!isOpen); if (!isOpen) loadHistory(); }}
      >
        <h3>📊 圧縮履歴 ({history.length}件)</h3>
        <span className="history-toggle">{isOpen ? '▲' : '▼'}</span>
      </div>

      {isOpen && (
        <>
          {history.length > 0 && (
            <div className="history-stats">
              <span>📁 合計 {history.length} 件</span>
              <span>💾 総削減量: <strong>{formatSize(totalSaved)}</strong></span>
            </div>
          )}

          <div className="history-actions">
            <button className="btn-small btn-export" onClick={handleExport} disabled={history.length === 0}>
              📥 CSV書き出し
            </button>
            <button className="btn-small btn-delete" onClick={handleDeleteSelected} disabled={selected.size === 0}>
              🗑️ 選択削除 ({selected.size})
            </button>
            <button className="btn-small btn-danger" onClick={handleClearAll} disabled={history.length === 0}>
              🗑️ 全削除
            </button>
          </div>

          {history.length > 0 && (
            <div className="history-list">
              <div className="history-select-all">
                <label>
                  <input
                    type="checkbox"
                    checked={selected.size === history.length && history.length > 0}
                    onChange={toggleSelectAll}
                  />
                  すべて選択
                </label>
              </div>
              {history.slice().reverse().map((entry, ri) => {
                const i = history.length - 1 - ri; // actual index
                return (
                  <div key={i} className={`history-item ${selected.has(i) ? 'selected' : ''}`}>
                    <input
                      type="checkbox"
                      checked={selected.has(i)}
                      onChange={() => toggleSelect(i)}
                    />
                    <div className="history-item-info">
                      <span className="history-filename">{entry.filename}</span>
                      <span className="history-detail">
                        {formatSize(entry.originalSize)} → {formatSize(entry.compressedSize)}
                        <span className="history-reduction">({entry.reduction.toFixed(1)}%削減)</span>
                      </span>
                    </div>
                    <span className="history-time">{formatTimestamp(entry.timestamp)}</span>
                  </div>
                );
              })}
            </div>
          )}

          {history.length === 0 && (
            <div className="history-empty">履歴はまだありません 🐹</div>
          )}
        </>
      )}
    </div>
  );
};

export default HistoryPanel;
