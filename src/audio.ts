import { PreviewNote } from "./types";

let ctx: AudioContext | null = null;

/**
 * 用方波振荡器播放音符序列（与蜂鸣器音色一致）。
 * 返回停止函数；自然播完时触发 onEnded。
 */
export function playNotes(
  notes: PreviewNote[],
  masterVolume: number,
  onEnded: () => void
): () => void {
  if (!ctx) ctx = new AudioContext();
  if (ctx.state === "suspended") void ctx.resume();

  const oscillators: OscillatorNode[] = [];
  const t0 = ctx.currentTime + 0.06;
  const master = masterVolume / 100;
  let maxEnd = 0;

  for (const n of notes) {
    if (n.frequency <= 0 || n.volume === 0) continue;
    const start = t0 + n.startMs / 1000;
    const dur = Math.max(0.02, n.durationMs / 1000);
    const osc = ctx.createOscillator();
    osc.type = "square";
    osc.frequency.value = n.frequency;
    const gain = ctx.createGain();
    const v = (n.volume / 100) * master * 0.12;
    gain.gain.setValueAtTime(0, start);
    gain.gain.linearRampToValueAtTime(v, start + 0.006);
    gain.gain.setValueAtTime(v, start + Math.max(0.006, dur - 0.025));
    gain.gain.linearRampToValueAtTime(0, start + dur);
    osc.connect(gain);
    gain.connect(ctx.destination);
    osc.start(start);
    osc.stop(start + dur + 0.01);
    oscillators.push(osc);
    maxEnd = Math.max(maxEnd, n.startMs + n.durationMs);
  }

  const timer = window.setTimeout(onEnded, maxEnd + 120);
  return () => {
    window.clearTimeout(timer);
    for (const o of oscillators) {
      try {
        o.stop();
      } catch {
        /* 已停止 */
      }
    }
  };
}
