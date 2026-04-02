// src/DropZone.tsx
import React, { useState, useEffect, useRef } from 'react';
import { CompressionResult } from './utils';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import { open } from '@tauri-apps/plugin-dialog';

interface Settings {
  jpegQuality: number;
  pngColors: number;
  pdfPreset: string;
  officeQuality: number;
  group: boolean;
}

interface Props {
  settings: Settings;
  onComplete: (results: CompressionResult[]) => void;
}

// Matches the Rust CompressResult struct
interface RustCompressResult {
  filename: string;
  outputFilename: string;
  outputPath: string;
  originalSize: number;
  compressedSize: number;
  reduction: number;
  isError: boolean;
  errorMessage: string | null;
}

const DropZone: React.FC<Props> = ({ settings, onComplete }) => {
  const [processing, setProcessing] = useState(false);
  const [statusText, setStatusText] = useState('');
  const [isDragOver, setIsDragOver] = useState(false);
  const unlistenRef = useRef<(() => void) | null>(null);

  useEffect(() => {
    let cancelled = false;

    const setupListener = async () => {
      const appWindow = getCurrentWebviewWindow();

      const unlisten = await appWindow.onDragDropEvent((event) => {
        if (cancelled) return;

        if (event.payload.type === 'over') {
          setIsDragOver(true);
        } else if (event.payload.type === 'drop') {
          setIsDragOver(false);
          const paths = event.payload.paths;
          if (paths && paths.length > 0) {
            handleFilePaths(paths);
          }
        } else if (event.payload.type === 'leave') {
          setIsDragOver(false);
        }
      });

      if (!cancelled) {
        unlistenRef.current = unlisten;
      } else {
        unlisten();
      }
    };

    setupListener();

    return () => {
      cancelled = true;
      if (unlistenRef.current) {
        unlistenRef.current();
      }
    };
  }, [settings]);

  const handleFilePaths = async (paths: string[]) => {
    if (paths.length === 0) return;
    console.log('Received file paths:', paths);
    setProcessing(true);
    setStatusText(`圧縮中… ${paths.length}件`);
    try {
      // Parse PDF preset
      const [pdfDpi, pdfJpegQ] = settings.pdfPreset.split(',').map(Number);

      const rustResults = await invoke<RustCompressResult[]>('compress', {
        inputs: paths,
        settings: {
          jpegQuality: settings.jpegQuality,
          pngColors: settings.pngColors,
          pdfDpi: pdfDpi || 235,
          pdfJpegQ: pdfJpegQ || 82,
          officeQuality: settings.officeQuality,
        },
      });

      console.log('Compression results:', rustResults);

      const results: CompressionResult[] = rustResults
        .filter(r => !r.isError)
        .map(r => ({
          filename: r.filename,
          originalSize: r.originalSize,
          compressedSize: r.compressedSize,
          outputPath: r.outputPath,
          reduction: r.reduction,
        }));

      // Report errors
      const errors = rustResults.filter(r => r.isError);
      for (const err of errors) {
        console.error(`Compression error for ${err.filename}: ${err.errorMessage}`);
      }

      if (results.length > 0) {
        onComplete(results);
      }
      setStatusText(`✅ ${rustResults.length}件完了${errors.length > 0 ? ` (${errors.length}件エラー)` : ''}`);
    } catch (e) {
      console.error('Compression error', e);
      setStatusText(`❌ エラー: ${e}`);
    } finally {
      setProcessing(false);
    }
  };

  const handleClick = async () => {
    try {
      const selected = await open({
        multiple: true,
        filters: [
          {
            name: 'Supported Files',
            extensions: ['jpg', 'jpeg', 'png', 'pdf', 'docx', 'xlsx', 'pptx'],
          },
        ],
      });
      if (selected && Array.isArray(selected) && selected.length > 0) {
        handleFilePaths(selected);
      } else if (selected && typeof selected === 'string') {
        handleFilePaths([selected]);
      }
    } catch (e) {
      console.error('File dialog error', e);
    }
  };

  return (
    <>
      <div
        className={`dropzone ${isDragOver ? 'dragover' : ''}`}
        onClick={handleClick}
      >
        <div className="dropzone-icon">📁</div>
        <p className="dropzone-text">
          {isDragOver
            ? 'ここにドロップしてください…'
            : 'ファイルをここにドラッグ＆ドロップ'}
        </p>
        <p className="dropzone-sub">またはクリックして選択</p>
      </div>
      {processing && (
        <div className="processing-indicator">
          <div className="spinner"></div>
          <span>{statusText || '圧縮中…'}</span>
        </div>
      )}
      {!processing && statusText && (
        <div className="processing-indicator" style={{ borderColor: 'rgba(100, 255, 218, 0.3)' }}>
          <span>{statusText}</span>
        </div>
      )}
    </>
  );
};

export default DropZone;
