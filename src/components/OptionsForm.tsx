import { FormState } from "../types";

interface Props {
  form: FormState;
  onChange: (patch: Partial<FormState>) => void;
}

export default function OptionsForm({ form, onChange }: Props) {
  return (
    <div className="card">
      <h2>转换参数</h2>
      <div className="form-grid">
        <label>
          曲名（C 标识符）
          <input
            type="text"
            value={form.songName}
            onChange={(e) => onChange({ songName: e.target.value })}
          />
        </label>
        <label>
          移调（半音，{form.transpose > 0 ? `+${form.transpose}` : form.transpose}）
          <input
            type="range"
            min={-24}
            max={24}
            value={form.transpose}
            onChange={(e) => onChange({ transpose: Number(e.target.value) })}
          />
        </label>
        <label>
          倍速
          <input
            type="number"
            min={0.25}
            max={4}
            step={0.05}
            value={form.speed}
            onChange={(e) => onChange({ speed: Number(e.target.value) })}
          />
        </label>
        <label>
          主音量（{form.masterVolume}）
          <input
            type="range"
            min={0}
            max={100}
            value={form.masterVolume}
            onChange={(e) => onChange({ masterVolume: Number(e.target.value) })}
          />
        </label>
        <label>
          音符间隔（ms）
          <input
            type="number"
            min={0}
            max={100}
            value={form.gapMs}
            onChange={(e) => onChange({ gapMs: Number(e.target.value) })}
          />
        </label>
        <label>
          和弦取音
          <select
            value={form.strategy}
            onChange={(e) => onChange({ strategy: e.target.value })}
          >
            <option value="highest">最高音（旋律通常在上方）</option>
            <option value="lowest">最低音</option>
          </select>
        </label>
        <label className="checkbox">
          <input
            type="checkbox"
            checked={form.includePlayer}
            onChange={(e) => onChange({ includePlayer: e.target.checked })}
          />
          导出时附带 buzzer_player.c/.h（已存在则跳过）
        </label>
      </div>
    </div>
  );
}
