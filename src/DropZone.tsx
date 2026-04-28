// src/DropZone.tsx
import React, { useState, useEffect, useRef } from 'react';
import { CompressionResult } from './utils';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import { open } from '@tauri-apps/plugin-dialog';
import hamsterImg from './assets/hamster.png';

interface Settings {
  jpegQuality: number;
  pngColors: number;
  pdfDpi: number;
  pdfJpegQ: number;
  officeQuality: number;
  group: boolean;
}

interface Props {
  settings: Settings;
  outputDir: string;
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

const DropZone: React.FC<Props> = ({ settings, outputDir, onComplete }) => {
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
  }, [settings, outputDir]);

  const handleFilePaths = async (paths: string[]) => {
    if (paths.length === 0) return;
    console.log('Received file paths:', paths);
    setProcessing(true);
    setStatusText(`圧縮中… ${paths.length}件`);
    try {
      const rustResults = await invoke<RustCompressResult[]>('compress', {
        inputs: paths,
        settings: {
          jpegQuality: settings.jpegQuality,
          pngColors: settings.pngColors,
          pdfDpi: settings.pdfDpi,
          pdfJpegQ: settings.pdfJpegQ,
          officeQuality: settings.officeQuality,
          outputDir: outputDir || undefined,
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
        <img src={hamsterImg} alt="ハムスター" className="dropzone-icon" />
        <p className="dropzone-text">
          {isDragOver
            ? 'わくわく！ひまわりの種かな？✨'
            : 'ファイルをここに置いてね！🐹🌻'}
        </p>
        <p className="dropzone-sub">またはクリックして選択</p>
      </div>
      {processing && (
        <div className="processing-indicator">
          <div className="spinner"></div>
          <span>{statusText || 'もぐもぐ圧縮中...🐹💨'}</span>
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
