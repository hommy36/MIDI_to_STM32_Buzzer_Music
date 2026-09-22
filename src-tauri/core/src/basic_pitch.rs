//! Spotify basic-pitch 模型的 Rust 移植（ICASSP 2022 ONNX 版）。
//! 算法与官方 Python 实现（inference.py / note_creation.py）逐行对齐，
//! 差异：不提取 pitch bend（蜂鸣器用不上），不限制频率范围。

use std::path::Path;

use tract_onnx::prelude::*;

pub const MODEL_SAMPLE_RATE: usize = 22050;
pub const FFT_HOP: usize = 256;
/// 模型输入窗口采样数（约 2 秒）
pub const AUDIO_N_SAMPLES: usize = 43844;
/// 每窗口输出帧数
pub const ANNOT_N_FRAMES: usize = 172;
const N_OVERLAPPING_FRAMES: usize = 30;
const OVERLAP_LEN: usize = N_OVERLAPPING_FRAMES * FFT_HOP; // 7680
const HOP_SIZE: usize = AUDIO_N_SAMPLES - OVERLAP_LEN; // 36164
const FRAMES_PER_WINDOW: usize = ANNOT_N_FRAMES - N_OVERLAPPING_FRAMES; // 142
const N_FREQS: usize = 88;

const ONSET_THRESH: f32 = 0.5;
const FRAME_THRESH: f32 = 0.3;
/// 最短音符（帧）。127.7ms * (22050/256) / 1000 ≈ 11
const MIN_NOTE_LEN: usize = 11;
const ENERGY_TOL: usize = 11;
const MIDI_OFFSET: u8 = 21;
const MAX_FREQ_IDX: usize = 87;
const MAGIC_ALIGNMENT_OFFSET: f32 = 0.0018;

/// 一个识别出的音符事件
#[derive(Debug, Clone, Copy)]
pub struct NoteEvent {
    pub start_s: f32,
    pub end_s: f32,
    pub pitch: u8,
    pub amplitude: f32,
}

/// 行优先矩阵（n_rows × 88）
struct Matrix {
    data: Vec<f32>,
    rows: usize,
}

impl Matrix {
    fn new() -> Self {
        Matrix {
            data: Vec::new(),
            rows: 0,
        }
    }
    fn get(&self, r: usize, c: usize) -> f32 {
        self.data[r * N_FREQS + c]
    }
    fn set(&mut self, r: usize, c: usize, v: f32) {
        self.data[r * N_FREQS + c] = v;
    }
    fn max(&self) -> f32 {
        self.data.iter().cloned().fold(f32::NEG_INFINITY, f32::max)
    }
    fn truncate_rows(&mut self, n: usize) {
        if n < self.rows {
            self.data.truncate(n * N_FREQS);
            self.rows = n;
        }
    }
}

type Model = SimplePlan<TypedFact, Box<dyn TypedOp>, Graph<TypedFact, Box<dyn TypedOp>>>;

fn load_model(path: &Path) -> Result<Model, String> {
    tract_onnx::onnx()
        .model_for_path(path)
        .map_err(|e| format!("模型文件加载失败: {e}"))?
        .with_input_fact(0, f32::fact([1usize, AUDIO_N_SAMPLES, 1]).into())
        .map_err(|e| format!("模型输入配置失败: {e}"))?
        // 注意：不用 into_optimized()——push_split_down 优化 pass 在 debug 构建下
        // 会因 fact 一致性断言崩溃（tract 0.21 已知问题），declutter 已含主要化简
        .into_typed()
        .map_err(|e| format!("模型类型推断失败: {e}"))?
        .into_decluttered()
        .map_err(|e| format!("模型化简失败: {e}"))?
        .into_runnable()
        .map_err(|e| format!("模型编译失败: {e}"))
}

/// 对 22050Hz 单声道音频运行 basic-pitch，返回音符事件列表。
/// `progress` 收到 0.0~1.0 的推理进度。
pub fn transcribe(
    audio: &[f32],
    model_path: &Path,
    mut progress: impl FnMut(f32),
) -> Result<Vec<NoteEvent>, String> {
    let model = load_model(model_path)?;
    let orig_len = audio.len();
    if orig_len == 0 {
        return Err("音频为空".to_string());
    }

    // 前补 overlap/2 个零（官方做法），再按 hop 分窗
    let mut padded = Vec::with_capacity(orig_len + OVERLAP_LEN / 2 + AUDIO_N_SAMPLES);
    padded.resize(OVERLAP_LEN / 2, 0.0);
    padded.extend_from_slice(audio);

    let mut notes = Matrix::new();
    let mut onsets = Matrix::new();

    let n_windows = padded.len().div_ceil(HOP_SIZE);
    let n_olap = N_OVERLAPPING_FRAMES / 2; // 15
    let mut win_idx = 0usize;
    let mut start = 0usize;
    while start < padded.len() {
        let mut window = vec![0f32; AUDIO_N_SAMPLES];
        let avail = (padded.len() - start).min(AUDIO_N_SAMPLES);
        window[..avail].copy_from_slice(&padded[start..start + avail]);

        let input = tract_ndarray::Array3::from_shape_vec((1, AUDIO_N_SAMPLES, 1), window)
            .map_err(|e| format!("输入构造失败: {e}"))?;
        let outputs = model
            .run(tvec![input.into_tensor().into()])
            .map_err(|e| format!("模型推理失败: {e}"))?;

        // ONNX 图的输出顺序为 [:2 onset, :1 note, :0 contour]，
        // 形状分别为 (1,172,88)、(1,172,88)、(1,172,264)。
        debug_assert_eq!(outputs.len(), 3);
        let onset_view = outputs[0]
            .to_array_view::<f32>()
            .map_err(|e| format!("输出解析失败: {e}"))?;
        let note_view = outputs[1]
            .to_array_view::<f32>()
            .map_err(|e| format!("输出解析失败: {e}"))?;
        let onset_shape = onset_view.shape();
        let note_shape = note_view.shape();
        if note_shape[2] != N_FREQS || onset_shape[2] != N_FREQS || note_shape[1] != ANNOT_N_FRAMES
        {
            return Err(format!(
                "模型输出形状异常: note={note_shape:?} onset={onset_shape:?}"
            ));
        }

        // 去掉每窗口首尾各 15 帧重叠区，拼接
        for frame in n_olap..ANNOT_N_FRAMES - n_olap {
            for f in 0..N_FREQS {
                onsets.data.push(onset_view[[0, frame, f]]);
                notes.data.push(note_view[[0, frame, f]]);
            }
            onsets.rows += 1;
            notes.rows += 1;
        }

        win_idx += 1;
        progress(win_idx as f32 / n_windows as f32);
        start += HOP_SIZE;
    }

    // 裁剪到原音频实际对应的帧数
    let keep = (orig_len as f64 / HOP_SIZE as f64 * FRAMES_PER_WINDOW as f64) as usize;
    notes.truncate_rows(keep);
    onsets.truncate_rows(keep);

    let events = extract_notes(&notes, &onsets);
    Ok(events
        .into_iter()
        .map(|(s, e, pitch, amp)| NoteEvent {
            start_s: frame_to_time(s),
            end_s: frame_to_time(e),
            pitch,
            amplitude: amp,
        })
        .collect())
}

fn frame_to_time(frame: usize) -> f32 {
    let t = frame as f32 * FFT_HOP as f32 / MODEL_SAMPLE_RATE as f32;
    let window_number = (frame / ANNOT_N_FRAMES) as f32;
    let window_offset = (FFT_HOP as f32 / MODEL_SAMPLE_RATE as f32)
        * (ANNOT_N_FRAMES as f32 - AUDIO_N_SAMPLES as f32 / FFT_HOP as f32)
        + MAGIC_ALIGNMENT_OFFSET;
    t - window_offset * window_number
}

/// 由帧能量差推断额外的 onset，与预测 onset 取逐点最大值
fn infer_onsets(onsets: &Matrix, frames: &Matrix) -> Matrix {
    let n = frames.rows;
    let mut fd = vec![0f32; n * N_FREQS];
    for nn in 1..=2usize {
        for t in 0..n {
            for f in 0..N_FREQS {
                let prev = if t >= nn { frames.get(t - nn, f) } else { 0.0 };
                let diff = frames.get(t, f) - prev;
                if nn == 1 || diff < fd[t * N_FREQS + f] {
                    fd[t * N_FREQS + f] = diff;
                }
            }
        }
    }
    for v in fd.iter_mut() {
        if *v < 0.0 {
            *v = 0.0;
        }
    }
    let zero_rows = 2.min(n) * N_FREQS;
    for v in fd[..zero_rows].iter_mut() {
        *v = 0.0;
    }
    let fd_max = fd.iter().cloned().fold(0f32, f32::max);
    let onset_max = onsets.max();
    let mut out = Matrix {
        data: vec![0f32; n * N_FREQS],
        rows: n,
    };
    for i in 0..n * N_FREQS {
        let scaled = if fd_max > 0.0 {
            onset_max * fd[i] / fd_max
        } else {
            0.0
        };
        out.data[i] = onsets.data[i].max(scaled);
    }
    out
}

/// onset 时间轴上的严格局部极大值
fn is_peak(onsets: &Matrix, t: usize, f: usize) -> bool {
    let v = onsets.get(t, f);
    v > onsets.get(t - 1, f) && v > onsets.get(t + 1, f)
}

/// 官方 output_to_notes_polyphonic 的移植（infer_onsets=true, melodia_trick=true）
fn extract_notes(frames: &Matrix, onsets_raw: &Matrix) -> Vec<(usize, usize, u8, f32)> {
    let n_frames = frames.rows;
    let mut events = Vec::new();
    if n_frames < 2 {
        return events;
    }
    let onsets = infer_onsets(onsets_raw, frames);

    // 峰值 onset，按时间倒序处理（与官方一致）
    let mut peaks: Vec<(usize, usize)> = Vec::new();
    for t in 1..n_frames - 1 {
        for f in 0..N_FREQS {
            if onsets.get(t, f) >= ONSET_THRESH && is_peak(&onsets, t, f) {
                peaks.push((t, f));
            }
        }
    }
    peaks.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));

    let mut remaining = Matrix {
        data: frames.data.clone(),
        rows: n_frames,
    };

    for (s, f) in peaks {
        if s >= n_frames - 1 {
            continue;
        }
        let mut i = s + 1;
        let mut k = 0usize;
        while i < n_frames - 1 && k < ENERGY_TOL {
            if remaining.get(i, f) < FRAME_THRESH {
                k += 1;
            } else {
                k = 0;
            }
            i += 1;
        }
        i -= k;
        if i - s <= MIN_NOTE_LEN {
            continue;
        }
        zero_span(&mut remaining, s, i, f);
        let mut amp = 0f32;
        for t in s..i {
            amp += frames.get(t, f);
        }
        amp /= (i - s) as f32;
        events.push((s, i, f as u8 + MIDI_OFFSET, amp));
    }

    // melodia trick：从剩余能量中迭代提取
    loop {
        let max = remaining.max();
        if max <= FRAME_THRESH {
            break;
        }
        let (mut i_mid, mut f) = (0usize, 0usize);
        'outer: for t in 0..n_frames {
            for c in 0..N_FREQS {
                if remaining.get(t, c) == max {
                    i_mid = t;
                    f = c;
                    break 'outer;
                }
            }
        }
        remaining.set(i_mid, f, 0.0);

        // 前向
        let mut i = i_mid + 1;
        let mut k = 0usize;
        while i < n_frames - 1 && k < ENERGY_TOL {
            if remaining.get(i, f) < FRAME_THRESH {
                k += 1;
            } else {
                k = 0;
            }
            zero_span(&mut remaining, i, i + 1, f);
            i += 1;
        }
        let i_end = i - 1 - k;

        // 后向
        let mut i = i_mid as isize - 1;
        let mut k = 0usize;
        while i > 0 && k < ENERGY_TOL {
            let t = i as usize;
            if remaining.get(t, f) < FRAME_THRESH {
                k += 1;
            } else {
                k = 0;
            }
            zero_span(&mut remaining, t, t + 1, f);
            i -= 1;
        }
        let i_start = (i + 1) as usize + k;

        if i_end <= i_start || i_end - i_start <= MIN_NOTE_LEN {
            continue;
        }
        let mut amp = 0f32;
        for t in i_start..i_end {
            amp += frames.get(t, f);
        }
        amp /= (i_end - i_start) as f32;
        events.push((i_start, i_end, f as u8 + MIDI_OFFSET, amp));
    }

    events
}

/// 清零 remaining[start..end, f-1..=f+1]（边界截断）
fn zero_span(m: &mut Matrix, start: usize, end: usize, f: usize) {
    for t in start..end {
        m.set(t, f, 0.0);
        if f < MAX_FREQ_IDX {
            m.set(t, f + 1, 0.0);
        }
        if f > 0 {
            m.set(t, f - 1, 0.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mat_from(rows: usize, fill: impl Fn(usize, usize) -> f32) -> Matrix {
        let mut data = vec![0f32; rows * N_FREQS];
        for t in 0..rows {
            for f in 0..N_FREQS {
                data[t * N_FREQS + f] = fill(t, f);
            }
        }
        Matrix { data, rows }
    }

    #[test]
    fn extract_onset_driven_note() {
        // 50 帧里 10..30 帧在 48 频点（MIDI 69）有能量，10 帧处有 onset 峰
        let freq_idx = 69 - MIDI_OFFSET as usize;
        let frames = mat_from(50, |t, f| {
            if (10..30).contains(&t) && f == freq_idx {
                0.9
            } else {
                0.0
            }
        });
        let onsets = mat_from(50, |t, f| {
            if t == 10 && f == freq_idx {
                0.95
            } else {
                0.0
            }
        });
        let events = extract_notes(&frames, &onsets);
        assert_eq!(events.len(), 1, "应识别出恰好一个音符: {events:?}");
        let (s, e, pitch, _amp) = events[0];
        assert_eq!(pitch, 69);
        assert!(s <= 10 && e >= 29, "音符范围 {s}..{e} 应覆盖 10..29");
    }

    #[test]
    fn melodia_trick_finds_note_without_onset() {
        // 无 onset，但 20..45 帧有持续能量 → melodia trick 应捞出
        let freq_idx = 60 - MIDI_OFFSET as usize;
        let frames = mat_from(60, |t, f| {
            if (20..45).contains(&t) && f == freq_idx {
                0.8
            } else {
                0.0
            }
        });
        let onsets = mat_from(60, |_, _| 0.0);
        let events = extract_notes(&frames, &onsets);
        assert_eq!(events.len(), 1, "melodia trick 应识别出一个音符: {events:?}");
        assert_eq!(events[0].2, 60);
    }

    #[test]
    fn short_notes_are_dropped() {
        // 能量只持续 5 帧（< 11），应被丢弃
        let freq_idx = 69 - MIDI_OFFSET as usize;
        let frames = mat_from(50, |t, f| {
            if (10..15).contains(&t) && f == freq_idx {
                0.9
            } else {
                0.0
            }
        });
        let onsets = mat_from(50, |t, f| {
            if t == 10 && f == freq_idx {
                0.95
            } else {
                0.0
            }
        });
        let events = extract_notes(&frames, &onsets);
        assert!(events.is_empty(), "过短音符应被丢弃: {events:?}");
    }

    #[test]
    fn frame_time_is_monotonic() {
        let mut prev = frame_to_time(0);
        for f in 1..1000 {
            let t = frame_to_time(f);
            assert!(t > prev, "帧时间应单调递增: f={f} t={t} prev={prev}");
            prev = t;
        }
    }

    /// 与官方 Python 实现（onnxruntime）在合成旋律上的输出对齐
    fn synth_melody() -> Vec<f32> {
        let sr = MODEL_SAMPLE_RATE as f32;
        let mut out = vec![0f32; (0.2 * sr) as usize];
        for (midi, dur) in [(69u8, 0.6f32), (72, 0.6), (76, 0.6), (67, 0.6)] {
            let freq = 440.0 * 2f32.powf((midi as f32 - 69.0) / 12.0);
            let n = (dur * sr) as usize;
            for i in 0..n {
                let t = i as f32 / sr;
                let env = (t / 0.01).min(1.0) * ((dur - t) / 0.05).min(1.0);
                out.push(0.5 * (2.0 * std::f32::consts::PI * freq * t).sin() * env.max(0.0));
            }
            out.extend(std::iter::repeat(0.0).take((0.15 * sr) as usize));
        }
        out
    }

    #[test]
    fn transcribe_synthetic_melody_matches_reference() {
        let model = std::env::var("BASIC_PITCH_MODEL")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| {
                std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../models/nmp.onnx")
            });
        if !model.is_file() {
            eprintln!("跳过：未找到模型文件 {}", model.display());
            return;
        }
        let audio = synth_melody();
        let mut events = transcribe(&audio, &model, |_| {}).expect("推理失败");
        events.sort_by(|a, b| a.start_s.partial_cmp(&b.start_s).unwrap());
        // 官方 Python 参考输出（testdata/ref_events.json）
        let expected = [(0.186f32, 69u8), (0.952, 72), (1.683, 76), (2.439, 67)];
        assert_eq!(events.len(), expected.len(), "音符数量不符: {events:?}");
        for (ev, (t, p)) in events.iter().zip(expected) {
            assert_eq!(ev.pitch, p, "音高不符: {ev:?}");
            assert!(
                (ev.start_s - t).abs() < 0.12,
                "起始时间偏差过大: {ev:?} 期望约 {t}s"
            );
        }
    }
}
