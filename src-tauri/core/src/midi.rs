use std::collections::HashMap;

use midly::{Format, MetaMessage, MidiMessage, Smf, Timing};
use serde::Serialize;

use crate::ConvertOptions;

// ---------------------------------------------------------------------------
// 数据结构

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonoStrategy {
    Highest,
    Lowest,
}

#[derive(Debug, Clone, Copy)]
pub struct Note {
    pub start_tick: u64,
    pub end_tick: u64,
    pub key: u8,
    pub velocity: u8,
}

#[derive(Debug)]
pub struct TrackData {
    pub index: usize,
    pub name: Option<String>,
    pub channels: Vec<u8>,
    pub program: Option<u8>,
    pub notes: Vec<Note>,
    pub end_tick: u64,
}

#[derive(Debug)]
pub struct ParsedMidi {
    pub format: Format,
    pub ppq: u16,
    pub tempo_map: TempoMap,
    pub tracks: Vec<TrackData>,
}

/// tick → 毫秒 换算表：segments 按 tick 升序，首元素 tick 必为 0
#[derive(Debug)]
pub struct TempoMap {
    segments: Vec<(u64, u32)>, // (tick, 每四分音符微秒数)
}

impl TempoMap {
    pub fn initial_bpm(&self) -> f64 {
        60_000_000.0 / self.segments[0].1 as f64
    }

    pub fn ticks_to_ms(&self, tick: u64, ppq: u16) -> f64 {
        let mut ms = 0.0f64;
        let mut prev_tick = 0u64;
        let mut prev_us = self.segments[0].1;
        for &(t, us) in self.segments.iter().skip(1) {
            if t >= tick {
                break;
            }
            ms += (t - prev_tick) as f64 * prev_us as f64 / ppq as f64 / 1000.0;
            prev_tick = t;
            prev_us = us;
        }
        ms + (tick - prev_tick) as f64 * prev_us as f64 / ppq as f64 / 1000.0
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackInfo {
    pub index: usize,
    pub name: Option<String>,
    pub channels: Vec<u8>,
    pub program: Option<u8>,
    pub instrument: Option<String>,
    pub note_count: u32,
    pub lowest: Option<u8>,
    pub highest: Option<u8>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MidiAnalysis {
    pub format: u8,
    pub ppq: u16,
    pub track_count: usize,
    pub initial_bpm: f64,
    pub duration_ms: u64,
    pub tracks: Vec<TrackInfo>,
}

/// 单音化后的片段（ms 在 build_song 阶段才算，这里保持 tick 以便精确合并）
#[derive(Debug, Clone, Copy)]
pub struct FlatSegment {
    pub start_tick: u64,
    pub end_tick: u64,
    pub key: u8,
    pub velocity: u8,
}

/// 最终输出用音符（frequency=0 表示休止符）
#[derive(Debug, Clone)]
pub struct SongNote {
    pub frequency: f32,
    pub duration_ms: u32,
    pub volume: u8,
    pub comment: String,
}

// ---------------------------------------------------------------------------
// 解析

pub fn parse(bytes: &[u8]) -> Result<ParsedMidi, String> {
    let smf = Smf::parse(bytes).map_err(|e| format!("MIDI 解析失败: {e}"))?;

    // 注意 midly 的命名按播放行为：Parallel = format 1（多轨同时播放，最常见），
    // Sequential = format 2（多轨依次独立播放）。只拒绝真正的 format 2。
    if smf.header.format == Format::Sequential {
        return Err("不支持 format 2 的 MIDI 文件".to_string());
    }
    let ppq = match smf.header.timing {
        Timing::Metrical(t) => t.as_int(),
        Timing::Timecode(_, _) => {
            return Err("暂不支持 SMPTE 时间格式的 MIDI 文件".to_string());
        }
    };

    // 第一遍：收集所有轨的 tempo 事件
    let mut tempo_events: Vec<(u64, u32)> = Vec::new();
    for track in &smf.tracks {
        let mut tick = 0u64;
        for ev in track {
            tick += ev.delta.as_int() as u64;
            if let midly::TrackEventKind::Meta(MetaMessage::Tempo(t)) = ev.kind {
                tempo_events.push((tick, t.as_int()));
            }
        }
    }
    tempo_events.sort_by_key(|&(t, _)| t);
    tempo_events.dedup_by_key(|e| e.0);
    if tempo_events.first().map(|e| e.0) != Some(0) {
        tempo_events.insert(0, (0, 500_000)); // 默认 120 BPM
    }
    let tempo_map = TempoMap {
        segments: tempo_events,
    };

    // 第二遍：逐轨提取音符与元信息
    let mut tracks = Vec::new();
    for (index, track) in smf.tracks.iter().enumerate() {
        let mut tick = 0u64;
        let mut name: Option<String> = None;
        let mut program: Option<u8> = None;
        let mut channels: Vec<u8> = Vec::new();
        let mut active: HashMap<(u8, u8), Vec<(u64, u8)>> = HashMap::new();
        let mut notes: Vec<Note> = Vec::new();

        for ev in track {
            tick += ev.delta.as_int() as u64;
            match ev.kind {
                midly::TrackEventKind::Meta(MetaMessage::TrackName(bytes)) => {
                    let s = String::from_utf8_lossy(bytes).trim().to_string();
                    if !s.is_empty() {
                        name = Some(s);
                    }
                }
                midly::TrackEventKind::Midi { channel, message } => {
                    let ch = channel.as_int();
                    if !channels.contains(&ch) {
                        channels.push(ch);
                    }
                    match message {
                        MidiMessage::NoteOn { key, vel } if vel.as_int() > 0 => {
                            active
                                .entry((ch, key.as_int()))
                                .or_default()
                                .push((tick, vel.as_int()));
                        }
                        MidiMessage::NoteOff { key, .. } | MidiMessage::NoteOn { key, .. } => {
                            let k = key.as_int();
                            if let Some(stack) = active.get_mut(&(ch, k)) {
                                if let Some((start, v)) = stack.pop() {
                                    if tick > start {
                                        notes.push(Note {
                                            start_tick: start,
                                            end_tick: tick,
                                            key: k,
                                            velocity: v,
                                        });
                                    }
                                }
                            }
                        }
                        MidiMessage::ProgramChange { program: p } => {
                            program = Some(p.as_int());
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        // 未闭合的音符在轨尾关闭
        for ((_, k), stack) in active {
            for (start, v) in stack {
                if tick > start {
                    notes.push(Note {
                        start_tick: start,
                        end_tick: tick,
                        key: k,
                        velocity: v,
                    });
                }
            }
        }
        notes.sort_by_key(|n| (n.start_tick, n.end_tick));
        channels.sort_unstable();

        tracks.push(TrackData {
            index,
            name,
            channels,
            program,
            notes,
            end_tick: tick,
        });
    }

    Ok(ParsedMidi {
        format: smf.header.format,
        ppq,
        tempo_map,
        tracks,
    })
}

pub fn analyze(parsed: &ParsedMidi) -> MidiAnalysis {
    let end_tick = parsed.tracks.iter().map(|t| t.end_tick).max().unwrap_or(0);
    MidiAnalysis {
        format: match parsed.format {
            Format::SingleTrack => 0,
            Format::Parallel => 1,
            Format::Sequential => 2,
        },
        ppq: parsed.ppq,
        track_count: parsed.tracks.len(),
        initial_bpm: (parsed.tempo_map.initial_bpm() * 10.0).round() / 10.0,
        duration_ms: parsed.tempo_map.ticks_to_ms(end_tick, parsed.ppq).round() as u64,
        tracks: parsed
            .tracks
            .iter()
            .map(|t| TrackInfo {
                index: t.index,
                name: t.name.clone(),
                channels: t.channels.clone(),
                program: t.program,
                instrument: t.program.map(|p| gm_instrument(p).to_string()),
                note_count: t.notes.len() as u32,
                lowest: t.notes.iter().map(|n| n.key).min(),
                highest: t.notes.iter().map(|n| n.key).max(),
            })
            .collect(),
    }
}

// ---------------------------------------------------------------------------
// 单音化：时间切片法

pub fn flatten(tracks: &[&TrackData], strategy: MonoStrategy) -> Vec<FlatSegment> {
    let notes: Vec<Note> = tracks
        .iter()
        .flat_map(|t| t.notes.iter().copied())
        .collect();
    if notes.is_empty() {
        return Vec::new();
    }

    // 所有 note-on / note-off 时刻作为切片边界
    let mut bounds: Vec<u64> = Vec::with_capacity(notes.len() * 2);
    for n in &notes {
        bounds.push(n.start_tick);
        bounds.push(n.end_tick);
    }
    bounds.sort_unstable();
    bounds.dedup();

    let mut segments: Vec<FlatSegment> = Vec::new();
    for w in bounds.windows(2) {
        let (lo, hi) = (w[0], w[1]);
        let mut chosen: Option<Note> = None;
        for n in &notes {
            if n.start_tick <= lo && n.end_tick >= hi {
                let better = match (chosen, strategy) {
                    (None, _) => true,
                    (Some(c), MonoStrategy::Highest) => {
                        n.key > c.key || (n.key == c.key && n.start_tick > c.start_tick)
                    }
                    (Some(c), MonoStrategy::Lowest) => {
                        n.key < c.key || (n.key == c.key && n.start_tick > c.start_tick)
                    }
                };
                if better {
                    chosen = Some(*n);
                }
            }
        }
        if let Some(c) = chosen {
            // 相邻同音同力度切片合并
            if let Some(last) = segments.last_mut() {
                if last.key == c.key && last.velocity == c.velocity && last.end_tick == lo {
                    last.end_tick = hi;
                    continue;
                }
            }
            segments.push(FlatSegment {
                start_tick: lo,
                end_tick: hi,
                key: c.key,
                velocity: c.velocity,
            });
        }
    }
    segments
}

// ---------------------------------------------------------------------------
// 生成最终音符序列（移调 / 倍速 / 力度→音量 / 休止 / 钳制）

pub fn build_song(
    segments: &[FlatSegment],
    tempo_map: &TempoMap,
    ppq: u16,
    opt: &ConvertOptions,
) -> (Vec<SongNote>, Vec<String>) {
    let speed = if opt.speed > 0.0 { opt.speed } else { 1.0 };
    let mut warnings: Vec<String> = Vec::new();
    let mut dropped = 0u32;

    // 第一遍：移调过滤 + tick→ms + 倍速
    struct Row {
        start_ms: f64,
        end_ms: f64,
        key: u8,
        volume: u8,
    }
    let mut rows: Vec<Row> = Vec::new();
    for seg in segments {
        let key = seg.key as i32 + opt.transpose;
        if !(0..=127).contains(&key) {
            dropped += 1;
            continue;
        }
        let key = key as u8;
        rows.push(Row {
            start_ms: tempo_map.ticks_to_ms(seg.start_tick, ppq) / speed,
            end_ms: tempo_map.ticks_to_ms(seg.end_tick, ppq) / speed,
            key,
            volume: velocity_to_volume(seg.velocity),
        });
    }
    if dropped > 0 {
        warnings.push(format!("移调后超出 MIDI 音域(0~127)，已丢弃 {dropped} 个音符"));
    }
    if rows.is_empty() {
        return (Vec::new(), warnings);
    }

    // 第二遍：插入休止 / 合并小间隙，钳制最短时值
    let min_note_ms = (opt.gap_ms + 5).max(1) as f64;
    let mut clamped = 0u32;
    let mut out: Vec<SongNote> = Vec::new();
    let mut cursor = rows[0].start_ms; // 跳过开头静音

    for row in &rows {
        let gap = row.start_ms - cursor;
        if gap >= 10.0 {
            out.push(SongNote {
                frequency: 0.0,
                duration_ms: gap.round().max(1.0) as u32,
                volume: 0,
                comment: "REST".to_string(),
            });
        } else if gap > 0.0 {
            // 小间隙并入前一个音符
            if let Some(last) = out.last_mut() {
                last.duration_ms += gap.round() as u32;
            }
        }

        let mut dur = (row.end_ms - row.start_ms).round();
        if dur < min_note_ms {
            dur = min_note_ms;
            clamped += 1;
        }
        out.push(SongNote {
            frequency: key_to_freq(row.key),
            duration_ms: dur as u32,
            volume: row.volume,
            comment: note_name(row.key),
        });
        cursor = row.start_ms + dur.max(row.end_ms - row.start_ms);
        // cursor 取实际发声结束时间（含钳制延长），避免休止计算重叠
        if cursor < row.end_ms {
            cursor = row.end_ms;
        }
    }
    if clamped > 0 {
        warnings.push(format!(
            "{clamped} 个音符时值过短，已钳制为 {} ms",
            min_note_ms as u32
        ));
    }

    (out, warnings)
}

pub fn velocity_to_volume(velocity: u8) -> u8 {
    ((velocity as u32 * 100 + 63) / 127).max(1) as u8
}

pub fn key_to_freq(key: u8) -> f32 {
    (440.0 * 2f64.powf((key as f64 - 69.0) / 12.0)) as f32
}

pub fn note_name(key: u8) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    let octave = key as i32 / 12 - 1;
    format!("{}{}", NAMES[(key % 12) as usize], octave)
}

pub fn gm_instrument(program: u8) -> &'static str {
    const GM: [&str; 128] = [
        "Acoustic Grand Piano", "Bright Acoustic Piano", "Electric Grand Piano",
        "Honky-tonk Piano", "Electric Piano 1", "Electric Piano 2", "Harpsichord", "Clavinet",
        "Celesta", "Glockenspiel", "Music Box", "Vibraphone",
        "Marimba", "Xylophone", "Tubular Bells", "Dulcimer",
        "Drawbar Organ", "Percussive Organ", "Rock Organ", "Church Organ",
        "Reed Organ", "Accordion", "Harmonica", "Tango Accordion",
        "Acoustic Guitar (nylon)", "Acoustic Guitar (steel)", "Electric Guitar (jazz)",
        "Electric Guitar (clean)", "Electric Guitar (muted)", "Overdriven Guitar",
        "Distortion Guitar", "Guitar Harmonics",
        "Acoustic Bass", "Electric Bass (finger)", "Electric Bass (pick)", "Fretless Bass",
        "Slap Bass 1", "Slap Bass 2", "Synth Bass 1", "Synth Bass 2",
        "Violin", "Viola", "Cello", "Contrabass",
        "Tremolo Strings", "Pizzicato Strings", "Orchestral Harp", "Timpani",
        "String Ensemble 1", "String Ensemble 2", "Synth Strings 1", "Synth Strings 2",
        "Choir Aahs", "Voice Oohs", "Synth Voice", "Orchestra Hit",
        "Trumpet", "Trombone", "Tuba", "Muted Trumpet",
        "French Horn", "Brass Section", "Synth Brass 1", "Synth Brass 2",
        "Soprano Sax", "Alto Sax", "Tenor Sax", "Baritone Sax",
        "Oboe", "English Horn", "Bassoon", "Clarinet",
        "Piccolo", "Flute", "Recorder", "Pan Flute",
        "Blown Bottle", "Shakuhachi", "Whistle", "Ocarina",
        "Lead 1 (square)", "Lead 2 (sawtooth)", "Lead 3 (calliope)", "Lead 4 (chiff)",
        "Lead 5 (charang)", "Lead 6 (voice)", "Lead 7 (fifths)", "Lead 8 (bass + lead)",
        "Pad 1 (new age)", "Pad 2 (warm)", "Pad 3 (polysynth)", "Pad 4 (choir)",
        "Pad 5 (bowed)", "Pad 6 (metallic)", "Pad 7 (halo)", "Pad 8 (sweep)",
        "FX 1 (rain)", "FX 2 (soundtrack)", "FX 3 (crystal)", "FX 4 (atmosphere)",
        "FX 5 (brightness)", "FX 6 (goblins)", "FX 7 (echoes)", "FX 8 (sci-fi)",
        "Sitar", "Banjo", "Shamisen", "Koto",
        "Kalimba", "Bagpipe", "Fiddle", "Shanai",
        "Tinkle Bell", "Agogo", "Steel Drums", "Woodblock",
        "Taiko Drum", "Melodic Tom", "Synth Drum", "Reverse Cymbal",
        "Guitar Fret Noise", "Breath Noise", "Seashore", "Bird Tweet",
        "Telephone Ring", "Helicopter", "Applause", "Gunshot",
    ];
    GM.get(program as usize).copied().unwrap_or("Unknown")
}

// ---------------------------------------------------------------------------
// 测试

#[cfg(test)]
mod tests {
    use super::*;
    use midly::num::{u15, u24, u28, u4, u7};
    use midly::{Header, Track, TrackEvent, TrackEventKind};

    fn ev(delta: u32, kind: TrackEventKind<'static>) -> TrackEvent<'static> {
        TrackEvent {
            delta: u28::from(delta),
            kind,
        }
    }
    fn on(ch: u8, key: u8, vel: u8) -> TrackEventKind<'static> {
        TrackEventKind::Midi {
            channel: u4::from(ch),
            message: MidiMessage::NoteOn {
                key: u7::from(key),
                vel: u7::from(vel),
            },
        }
    }
    fn off(ch: u8, key: u8) -> TrackEventKind<'static> {
        TrackEventKind::Midi {
            channel: u4::from(ch),
            message: MidiMessage::NoteOff {
                key: u7::from(key),
                vel: u7::from(0),
            },
        }
    }
    fn tempo(us: u32) -> TrackEventKind<'static> {
        TrackEventKind::Meta(MetaMessage::Tempo(u24::from(us)))
    }

    fn build_smf(format: Format, ppq: u16, tracks: Vec<Track<'static>>) -> Vec<u8> {
        let smf = Smf {
            header: Header {
                format,
                timing: Timing::Metrical(u15::from(ppq)),
            },
            tracks,
        };
        let mut buf = Vec::new();
        smf.write_std(&mut buf).unwrap();
        buf
    }

    fn opts(track_idx: Vec<usize>) -> ConvertOptions {
        ConvertOptions {
            path: String::new(),
            tracks: track_idx,
            song_name: "Test".into(),
            transpose: 0,
            speed: 1.0,
            master_volume: 100,
            gap_ms: 25,
            strategy: "highest".into(),
        }
    }

    #[test]
    fn tempo_map_and_duration() {
        // ppq=480，120BPM：480 tick = 500ms；后半段换 240BPM：480 tick = 250ms
        let bytes = build_smf(
            Format::SingleTrack,
            480,
            vec![vec![
                ev(0, tempo(500_000)),
                ev(0, on(0, 60, 100)),
                ev(480, off(0, 60)),
                ev(0, tempo(250_000)),
                ev(0, on(0, 61, 100)),
                ev(480, off(0, 61)),
            ]],
        );
        let parsed = parse(&bytes).unwrap();
        assert_eq!(parsed.ppq, 480);
        assert!((parsed.tempo_map.initial_bpm() - 120.0).abs() < 1e-9);

        let notes = crate::convert_bytes(&bytes, &opts(vec![0])).unwrap();
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].duration_ms, 500);
        assert_eq!(notes[1].duration_ms, 250);
        assert!((notes[0].frequency - key_to_freq(60)).abs() < 0.01);
    }

    #[test]
    fn chord_highest_and_lowest() {
        // C4+E4+G4 同时按下
        let bytes = build_smf(
            Format::SingleTrack,
            480,
            vec![vec![
                ev(0, on(0, 60, 90)),
                ev(0, on(0, 64, 90)),
                ev(0, on(0, 67, 90)),
                ev(480, off(0, 60)),
                ev(0, off(0, 64)),
                ev(0, off(0, 67)),
            ]],
        );
        let high = crate::convert_bytes(&bytes, &opts(vec![0])).unwrap();
        assert_eq!(high.len(), 1);
        assert_eq!(high[0].comment, "G4");

        let mut low_opts = opts(vec![0]);
        low_opts.strategy = "lowest".into();
        let low = crate::convert_bytes(&bytes, &low_opts).unwrap();
        assert_eq!(low.len(), 1);
        assert_eq!(low[0].comment, "C4");
    }

    #[test]
    fn overlap_across_tracks() {
        // 轨0：C4 持续 0..960；轨1：E4 在 480..720 插入 → 最高音策略切成 C4|E4|C4
        let bytes = build_smf(
            Format::Parallel,
            480,
            vec![
                vec![ev(0, on(0, 60, 80)), ev(960, off(0, 60))],
                vec![ev(480, on(0, 64, 100)), ev(240, off(0, 64))],
            ],
        );
        let notes = crate::convert_bytes(&bytes, &opts(vec![0, 1])).unwrap();
        assert_eq!(notes.len(), 3);
        assert_eq!(notes[0].comment, "C4");
        assert_eq!(notes[0].duration_ms, 500);
        assert_eq!(notes[1].comment, "E4");
        assert_eq!(notes[1].duration_ms, 250);
        assert_eq!(notes[2].comment, "C4");
        assert_eq!(notes[2].duration_ms, 250);
        // 力度跟随被选中音符
        assert_eq!(notes[1].volume, velocity_to_volume(100));
    }

    #[test]
    fn rest_inserted_and_small_gap_merged() {
        // C4 0..480，间隙 96 tick(=100ms)，E4 576..1056 → 中间一个 100ms 休止
        let bytes = build_smf(
            Format::SingleTrack,
            480,
            vec![vec![
                ev(0, on(0, 60, 100)),
                ev(480, off(0, 60)),
                ev(96, on(0, 64, 100)),
                ev(480, off(0, 64)),
            ]],
        );
        let notes = crate::convert_bytes(&bytes, &opts(vec![0])).unwrap();
        assert_eq!(notes.len(), 3);
        assert_eq!(notes[1].frequency, 0.0);
        assert_eq!(notes[1].duration_ms, 100);

        // 间隙只有 5 tick(≈5.2ms) → 并入前一个音符，不产生休止
        let bytes2 = build_smf(
            Format::SingleTrack,
            480,
            vec![vec![
                ev(0, on(0, 60, 100)),
                ev(480, off(0, 60)),
                ev(5, on(0, 64, 100)),
                ev(480, off(0, 64)),
            ]],
        );
        let notes2 = crate::convert_bytes(&bytes2, &opts(vec![0])).unwrap();
        assert_eq!(notes2.len(), 2);
        assert!(notes2[0].duration_ms > 500);
    }

    #[test]
    fn velocity_to_volume_mapping() {
        assert_eq!(velocity_to_volume(127), 100);
        assert_eq!(velocity_to_volume(64), 50);
        assert_eq!(velocity_to_volume(1), 1);
    }

    #[test]
    fn transpose_drop_out_of_range() {
        let bytes = build_smf(
            Format::SingleTrack,
            480,
            vec![vec![ev(0, on(0, 126, 100)), ev(480, off(0, 126))]],
        );
        let mut o = opts(vec![0]);
        o.transpose = 4;
        let parsed = parse(&bytes).unwrap();
        let tracks: Vec<&TrackData> = parsed.tracks.iter().collect();
        let flat = flatten(&tracks, MonoStrategy::Highest);
        let (notes, warnings) = build_song(&flat, &parsed.tempo_map, parsed.ppq, &o);
        assert!(notes.is_empty());
        assert!(warnings.iter().any(|w| w.contains("丢弃")));
    }

    #[test]
    fn short_note_clamped() {
        // 1 tick ≈ 1.04ms < gap(25)+5 → 钳制为 30ms
        let bytes = build_smf(
            Format::SingleTrack,
            480,
            vec![vec![ev(0, on(0, 60, 100)), ev(1, off(0, 60))]],
        );
        let parsed = parse(&bytes).unwrap();
        let tracks: Vec<&TrackData> = parsed.tracks.iter().collect();
        let flat = flatten(&tracks, MonoStrategy::Highest);
        let (notes, warnings) = build_song(&flat, &parsed.tempo_map, parsed.ppq, &opts(vec![0]));
        assert_eq!(notes[0].duration_ms, 30);
        assert!(warnings.iter().any(|w| w.contains("钳制")));
    }

    #[test]
    fn speed_multiplier() {
        let bytes = build_smf(
            Format::SingleTrack,
            480,
            vec![vec![ev(0, on(0, 60, 100)), ev(480, off(0, 60))]],
        );
        let mut o = opts(vec![0]);
        o.speed = 2.0;
        let notes = crate::convert_bytes(&bytes, &o).unwrap();
        assert_eq!(notes[0].duration_ms, 250);
    }

    #[test]
    fn format2_rejected() {
        // midly 的 Sequential 对应 raw format 2（多轨依次独立播放）
        let bytes = build_smf(Format::Sequential, 480, vec![vec![]]);
        assert!(parse(&bytes).unwrap_err().contains("format 2"));
    }

    #[test]
    fn note_names() {
        assert_eq!(note_name(69), "A4");
        assert_eq!(note_name(60), "C4");
        assert_eq!(note_name(61), "C#4");
        assert!((key_to_freq(69) - 440.0).abs() < 0.01);
        assert!((key_to_freq(72) - 523.251).abs() < 0.01);
    }
}
