// src/ResultCard.tsx
import React from 'react';
import { CompressionResult, formatSize } from './utils';
import { revealItemInDir } from '@tauri-apps/plugin-opener';

interface Props {
  result: CompressionResult;
}

const ResultCard: React.FC<Props> = ({ result }) => {
  const { filename, originalSize, compressedSize, outputPath, reduction: precomputed } = result;
  const reduction = precomputed ?? (originalSize
    ? ((originalSize - compressedSize) / originalSize) * 100
    : 0);

  const getClass = () => {
    if (reduction >= 50) return 'good';
    if (reduction >= 20) return 'ok';
    return 'bad';
  };

  const handleOpen = async () => {
    try {
      await revealItemInDir(outputPath);
    } catch (e) {
      console.error('Failed to open folder', e);
    }
  };

  return (
    <div className={`result-card ${getClass()}`}>
      <div>
        <strong>{filename}</strong>
        <div>{formatSize(originalSize)} → {formatSize(compressedSize)}</div>
        <div>{reduction.toFixed(1)}% 削減</div>
      </div>
      <button className="button" onClick={handleOpen}>開く</button>
    </div>
  );
};

export default ResultCard;
