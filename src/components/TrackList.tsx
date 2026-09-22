import { TrackInfo, noteName } from "../types";

interface Props {
  tracks: TrackInfo[];
  selected: number[];
  onChange: (selected: number[]) => void;
  onAudition: (index: number) => void;
  playingLabel: string | null;
}

export default function TrackList({ tracks, selected, onChange, onAudition, playingLabel }: Props) {
  const toggle = (index: number) => {
    if (selected.includes(index)) {
      onChange(selected.filter((i) => i !== index));
    } else {
      onChange([...selected, index].sort((a, b) => a - b));
    }
  };

  return (
    <div className="card">
      <h2>选择轨道（多选将合并为单音旋律）</h2>
      <table className="track-table">
        <thead>
          <tr>
            <th></th>
            <th>#</th>
            <th>轨名</th>
            <th>乐器</th>
            <th>通道</th>
            <th>音符数</th>
            <th>音域</th>
            <th>试听</th>
          </tr>
        </thead>
        <tbody>
          {tracks.map((t) => (
            <tr
              key={t.index}
              className={selected.includes(t.index) ? "selected" : ""}
              onClick={() => toggle(t.index)}
            >
              <td>
                <input
                  type="checkbox"
                  checked={selected.includes(t.index)}
                  onChange={() => toggle(t.index)}
                  onClick={(e) => e.stopPropagation()}
                />
              </td>
              <td>{t.index}</td>
              <td>{t.name ?? <span className="dim">（未命名）</span>}</td>
              <td>{t.instrument ?? <span className="dim">—</span>}</td>
              <td>{t.channels.length > 0 ? t.channels.map((c) => c + 1).join(", ") : "—"}</td>
              <td>{t.noteCount}</td>
              <td>
                {t.lowest != null && t.highest != null
                  ? `${noteName(t.lowest)} ~ ${noteName(t.highest)}`
                  : "—"}
              </td>
              <td>
                <button
                  className="mini"
                  disabled={t.noteCount === 0}
                  title="试听此轨（方波音色，同蜂鸣器）"
                  onClick={(e) => {
                    e.stopPropagation();
                    onAudition(t.index);
                  }}
                >
                  {playingLabel === `轨 ${t.index}` ? "■" : "▶"}
                </button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
