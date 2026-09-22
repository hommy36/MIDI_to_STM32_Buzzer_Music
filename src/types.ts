export interface TrackInfo {
  index: number;
  name: string | null;
  channels: number[];
  program: number | null;
  instrument: string | null;
  noteCount: number;
  lowest: number | null;
  highest: number | null;
}

export interface MidiAnalysis {
  format: number;
  ppq: number;
  trackCount: number;
  initialBpm: number;
  durationMs: number;
  tracks: TrackInfo[];
}

export interface Generated {
  baseName: string;
  songC: string;
  songH: string;
  playerC: string;
  playerH: string;
  noteCount: number;
  restCount: number;
  durationMs: number;
  warnings: string[];
}

export interface FormState {
  songName: string;
  transpose: number;
  speed: number;
  masterVolume: number;
  gapMs: number;
  strategy: string;
  includePlayer: boolean;
}

export interface PreviewNote {
  startMs: number;
  durationMs: number;
  frequency: number;
  volume: number;
}

export interface AudioProgress {
  stage: string;
  percent: number;
}

const NOTE_NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

export function noteName(key: number): string {
  return `${NOTE_NAMES[key % 12]}${Math.floor(key / 12) - 1}`;
}

export function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms} ms`;
  const s = ms / 1000;
  if (s < 60) return `${s.toFixed(1)} s`;
  return `${Math.floor(s / 60)}:${String(Math.round(s % 60)).padStart(2, "0")}`;
}
