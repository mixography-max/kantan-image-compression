// src/utils.ts
export interface CompressionResult {
  filename: string;
  originalSize: number; // bytes
  compressedSize: number; // bytes
  outputPath: string;
  reduction?: number; // percentage
}

export function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const kb = bytes / 1024;
  if (kb < 1024) return `${kb.toFixed(1)} KB`;
  const mb = kb / 1024;
  if (mb < 1024) return `${mb.toFixed(1)} MB`;
  const gb = mb / 1024;
  return `${gb.toFixed(1)} GB`;
}
