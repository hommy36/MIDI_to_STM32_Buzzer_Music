import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { AudioProgress, FormState, Generated, MidiAnalysis, PreviewNote, formatDuration } from "./types";
import { playNotes } from "./audio";
import TrackList from "./components/TrackList";
import OptionsForm from "./components/OptionsForm";
import CodePreview from "./components/CodePreview";

const appWindow = getCurrentWindow();

const MIDI_EXTS = ["mid", "midi"];
const AUDIO_EXTS = ["mp3", "wav", "flac", "ogg", "m4a", "aac"];
const AUDIO_RE = new RegExp(`\\.(${AUDIO_EXTS.join("|")})$`, "i");

export default function App() {
  const [filePath, setFilePath] = useState<string | null>(null);
  const [displayPath, setDisplayPath] = useState<string | null>(null);
  const [analysis, setAnalysis] = useState<MidiAnalysis | null>(null);
  const [selected, setSelected] = useState<number[]>([]);
  const [theme, setTheme] = useState(
    () => localStorage.getItem("theme") || "light"
  );
  const [form, setForm] = useState<FormState>({
    songName: "Song",
    transpose: 0,
    speed: 1.0,
    masterVolume: 100,
    gapMs: 25,
    strategy: "highest",
    includePlayer: true,
  });
  const [generated, setGenerated] = useState<Generated | null>(null);
  const [exported, setExported] = useState<string[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [audioProgress, setAudioProgress] = useState<AudioProgress | null>(null);
  const [busy, setBusy] = useState(false);
  const [playing, setPlaying] = useState<string | null>(null);
  const stopRef = useRef<(() => void) | null>(null);

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    localStorage.setItem("theme", theme);
  }, [theme]);

  const stopPlayback = useCallback(() => {
    stopRef.current?.();
    stopRef.current = null;
    setPlaying(null);
  }, []);

  const startPlayback = useCallback(
    (label: string, notes: PreviewNote[], masterVolume: number) => {
      stopRef.current?.();
      setPlaying(label);
      const stop = playNotes(notes, masterVolume, () => {
        stopRef.current = null;
        setPlaying(null);
      });
      stopRef.current = stop;
    },
    []
  );

  const analyze = useCallback(async (path: string, display?: string) => {
    setBusy(true);
    setError(null);
    setNotice(null);
    setGenerated(null);
    setExported(null);
    stopPlayback();
    try {
      const a = await invoke<MidiAnalysis>("analyze_midi", { path });
      setAnalysis(a);
      setFilePath(path);
      setDisplayPath(display ?? path);
      let best = -1;
      let bestCount = 0;
      for (const t of a.tracks) {
        if (t.noteCount > bestCount) {
          bestCount = t.noteCount;
          best = t.index;
        }
      }
      setSelected(best >= 0 ? [best] : []);
      const base = (display ?? path).split(/[\\/]/).pop() ?? "Song";
      setForm((f) => ({ ...f, songName: base.replace(/\.[^.]+$/, "") || "Song" }));
    } catch (e) {
      setError(String(e));
      setAnalysis(null);
      setFilePath(null);
      setDisplayPath(null);
    } finally {
      setBusy(false);
    }
  }, [stopPlayback]);

  const loadAny = useCallback(
    async (path: string) => {
      if (!AUDIO_RE.test(path)) {
        analyze(path);
        return;
      }
      setBusy(true);
      setError(null);
      setNotice(null);
      setGenerated(null);
      setExported(null);
      setAnalysis(null);
      stopPlayback();
      setAudioProgress({ stage: "准备", percent: 0 });
      let unlisten: (() => void) | undefined;
      try {
        unlisten = await listen<AudioProgress>("audio-progress", (e) =>
          setAudioProgress(e.payload)
        );
        const midPath = await invoke<string>("convert_audio", { path });
        await analyze(midPath, path);
        setNotice("已从音频转换为 MIDI（AI 识别结果可能有误差，请试听确认）");
      } catch (e) {
        setError(String(e));
      } finally {
        unlisten?.();
        setAudioProgress(null);
        setBusy(false);
      }
    },
    [analyze, stopPlayback]
  );

  const auditionTrack = useCallback(
    async (index: number) => {
      if (!filePath) return;
      const label = `轨 ${index}`;
      if (playing === label) {
        stopPlayback();
        return;
      }
      setError(null);
      try {
        const notes = await invoke<PreviewNote[]>("track_notes", {
          path: filePath,
          trackIndex: index,
        });
        if (notes.length === 0) {
          setError(`轨 ${index} 没有可播放的音符`);
          return;
        }
        startPlayback(label, notes, form.masterVolume);
      } catch (e) {
        setError(String(e));
      }
    },
    [filePath, playing, stopPlayback, startPlayback, form.masterVolume]
  );

  const auditionResult = useCallback(async () => {
    if (!filePath) return;
    if (playing === "合并结果") {
      stopPlayback();
      return;
    }
    setError(null);
    try {
      const notes = await invoke<PreviewNote[]>("preview_convert", {
        options: {
          path: filePath,
          tracks: selected,
          songName: form.songName,
          transpose: form.transpose,
          speed: form.speed,
          masterVolume: form.masterVolume,
          gapMs: form.gapMs,
          strategy: form.strategy,
        },
      });
      if (notes.length === 0) {
        setError("合并结果为空，无法试听");
        return;
      }
      startPlayback("合并结果", notes, 100);
    } catch (e) {
      setError(String(e));
    }
  }, [filePath, playing, selected, form, stopPlayback, startPlayback]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    getCurrentWebviewWindow()
      .onDragDropEvent((event) => {
        if (event.payload.type === "drop") {
          const p = event.payload.paths.find((x) =>
            /\.(mid|midi|mp3|wav|flac|ogg|m4a|aac)$/i.test(x)
          );
          if (p) {
            loadAny(p);
          } else {
            setError("请拖入 .mid / .midi 或 mp3/wav/flac/ogg/m4a 音频文件");
          }
        }
      })
      .then((f) => {
        unlisten = f;
      })
      .catch((e) => setError(`拖放监听注册失败: ${String(e)}`));
    return () => {
      unlisten?.();
    };
  }, [loadAny]);

  const openFile = async () => {
    try {
      const p = await open({
        multiple: false,
        filters: [
          { name: "MIDI / 音频", extensions: [...MIDI_EXTS, ...AUDIO_EXTS] },
          { name: "MIDI", extensions: MIDI_EXTS },
          { name: "音频", extensions: AUDIO_EXTS },
        ],
      });
      if (typeof p === "string") loadAny(p);
    } catch (e) {
      setError(`打开文件对话框失败: ${String(e)}`);
    }
  };

  const patchForm = (patch: Partial<FormState>) =>
    setForm((f) => ({ ...f, ...patch }));

  const generate = async () => {
    if (!filePath) return;
    setBusy(true);
    setError(null);
    setExported(null);
    try {
      const g = await invoke<Generated>("generate_code", {
        options: {
          path: filePath,
          tracks: selected,
          songName: form.songName,
          transpose: form.transpose,
          speed: form.speed,
          masterVolume: form.masterVolume,
          gapMs: form.gapMs,
          strategy: form.strategy,
        },
      });
      setGenerated(g);
    } catch (e) {
      setError(String(e));
      setGenerated(null);
    } finally {
      setBusy(false);
    }
  };

  const exportFiles = async () => {
    if (!generated) return;
    const dir = await open({ directory: true });
    if (typeof dir !== "string") return;
    setError(null);
    try {
      const written = await invoke<string[]>("export_code", {
        dir,
        baseName: generated.baseName,
        songC: generated.songC,
        songH: generated.songH,
        playerC: generated.playerC,
        playerH: generated.playerH,
        includePlayer: form.includePlayer,
      });
      setExported(written);
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <div className="app">
      <div className="titlebar" data-tauri-drag-region>
        <div className="drag" data-tauri-drag-region>
          <h1 data-tauri-drag-region>MIDI Buzzer Studio</h1>
        </div>
        <button onClick={openFile} disabled={busy}>
          打开 MIDI / 音频
        </button>
        {displayPath && <span className="file-path">{displayPath}</span>}
        <div className="spacer" data-tauri-drag-region />
        <button
          className="theme-toggle"
          title={theme === "light" ? "切换暗色" : "切换亮色"}
          onClick={() => setTheme(theme === "light" ? "dark" : "light")}
        >
          {theme === "light" ? "🌙" : "☀️"}
        </button>
        <div className="win-controls">
          <button className="win-btn" title="最小化" onClick={() => appWindow.minimize()}>
            —
          </button>
          <button
            className="win-btn"
            title="最大化/还原"
            onClick={() => appWindow.toggleMaximize()}
          >
            ▢
          </button>
          <button
            className="win-btn close"
            title="关闭"
            onClick={() => appWindow.close()}
          >
            ✕
          </button>
        </div>
      </div>

      {error && <div className="banner error">{error}</div>}
      {audioProgress && (
        <div className="banner progress">
          <span className="progress-stage">{audioProgress.stage}</span>
          <div className="progress-track">
            <div
              className="progress-fill"
              style={{ width: `${Math.round(audioProgress.percent * 100)}%` }}
            />
          </div>
          <span className="progress-pct">
            {Math.round(audioProgress.percent * 100)}%
          </span>
        </div>
      )}
      {!audioProgress && notice && <div className="banner info">{notice}</div>}

      {!analysis ? (
        <div className="drop-hint">
          <p>把 .mid / .midi 或 mp3 / wav / flac / ogg / m4a 文件拖进窗口，或点击「打开 MIDI / 音频」</p>
          <p className="drop-hint-sub">音频文件会由本地 AI 模型自动识别成 MIDI（首次使用需下载约 230KB 模型）</p>
        </div>
      ) : (
        <main>
          <section className="col">
            <div className="card">
              <h2>文件信息</h2>
              <div className="info-grid">
                <span>格式</span>
                <b>Type {analysis.format}</b>
                <span>PPQ</span>
                <b>{analysis.ppq}</b>
                <span>初始速度</span>
                <b>{analysis.initialBpm} BPM</b>
                <span>总时长</span>
                <b>{formatDuration(analysis.durationMs)}</b>
                <span>轨道数</span>
                <b>{analysis.trackCount}</b>
              </div>
            </div>
            <TrackList
              tracks={analysis.tracks}
              selected={selected}
              onChange={setSelected}
              onAudition={auditionTrack}
              playingLabel={playing}
            />
          </section>

          <section className="col">
            <OptionsForm form={form} onChange={patchForm} />
            <div className="actions">
              <button
                onClick={auditionResult}
                disabled={busy || selected.length === 0}
                title="试听合并后的最终效果（方波音色，同蜂鸣器）"
              >
                {playing === "合并结果" ? "■ 停止" : "▶ 试听合并结果"}
              </button>
              <button
                className="primary"
                onClick={generate}
                disabled={busy || selected.length === 0}
              >
                {busy ? "处理中…" : "生成 C 代码"}
              </button>
            </div>
            {playing && playing !== "合并结果" && (
              <div className="banner playing">
                正在播放：{playing}
                <span style={{ flex: 1 }} />
                <button className="mini" onClick={stopPlayback}>
                  ■ 停止
                </button>
              </div>
            )}
          </section>
        </main>
      )}

      {generated && (
        <CodePreview
          generated={generated}
          onExport={exportFiles}
          exported={exported}
          onClose={() => setGenerated(null)}
        />
      )}
    </div>
  );
}
