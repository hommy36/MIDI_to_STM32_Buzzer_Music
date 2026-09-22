import { useEffect, useState } from "react";
import { Generated, formatDuration } from "../types";

interface Props {
  generated: Generated;
  onExport: () => void;
  exported: string[] | null;
  onClose: () => void;
}

export default function CodePreview({ generated, onExport, exported, onClose }: Props) {
  const files: [string, string][] = [
    [`${generated.baseName}.c`, generated.songC],
    [`${generated.baseName}.h`, generated.songH],
    ["buzzer_player.c", generated.playerC],
    ["buzzer_player.h", generated.playerH],
  ];
  const [tab, setTab] = useState(0);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const copy = async () => {
    await navigator.clipboard.writeText(files[tab][1]);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  };

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <h2>生成结果</h2>
          <button className="win-btn close" title="关闭 (Esc)" onClick={onClose}>
            ✕
          </button>
        </div>
        <div className="info-grid">
          <span>音符</span>
          <b>{generated.noteCount} 个</b>
          <span>休止符</span>
          <b>{generated.restCount} 个</b>
          <span>总时长</span>
          <b>{formatDuration(generated.durationMs)}</b>
        </div>
        {generated.warnings.length > 0 && (
          <div className="banner warn">
            {generated.warnings.map((w, i) => (
              <div key={i}>{w}</div>
            ))}
          </div>
        )}
        <div className="tabs">
          {files.map(([name], i) => (
            <button
              key={name}
              className={i === tab ? "tab active" : "tab"}
              onClick={() => setTab(i)}
            >
              {name}
            </button>
          ))}
          <span className="spacer" />
          <button onClick={copy}>{copied ? "已复制" : "复制"}</button>
          <button className="primary" onClick={onExport}>
            导出到目录…
          </button>
        </div>
        <pre className="code">{files[tab][1]}</pre>
        {exported && (
          <div className="banner ok">
            已写出：
            {exported.map((p) => (
              <div key={p}>{p}</div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
