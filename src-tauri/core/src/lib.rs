pub mod codegen;
pub mod midi;
pub mod player_template;

use serde::{Deserialize, Serialize};

use midi::{MonoStrategy, SongNote};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConvertOptions {
    pub path: String,
    pub tracks: Vec<usize>,
    pub song_name: String,
    pub transpose: i32,
    pub speed: f64,
    pub master_volume: u8,
    pub gap_ms: u32,
    pub strategy: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Generated {
    pub base_name: String,
    pub song_c: String,
    pub song_h: String,
    pub player_c: String,
    pub player_h: String,
    pub note_count: u32,
    pub rest_count: u32,
    pub duration_ms: u64,
    pub warnings: Vec<String>,
}

pub fn analyze_file(path: &str) -> Result<midi::MidiAnalysis, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("无法读取文件 {path}: {e}"))?;
    let parsed = midi::parse(&bytes)?;
    Ok(midi::analyze(&parsed))
}

/// 试听用音符（含绝对时间）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewNote {
    pub start_ms: f64,
    pub duration_ms: f64,
    pub frequency: f32,
    pub volume: u8,
}

/// 单条轨道的原始音符（不做单音化/移调），用于"这条轨是什么"的试听
pub fn track_preview(path: &str, track_index: usize) -> Result<Vec<PreviewNote>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("无法读取文件 {path}: {e}"))?;
    let parsed = midi::parse(&bytes)?;
    let track = parsed
        .tracks
        .get(track_index)
        .ok_or_else(|| format!("轨道序号 {track_index} 越界"))?;
    let mut out = Vec::with_capacity(track.notes.len());
    for n in &track.notes {
        let start = parsed.tempo_map.ticks_to_ms(n.start_tick, parsed.ppq);
        let end = parsed.tempo_map.ticks_to_ms(n.end_tick, parsed.ppq);
        out.push(PreviewNote {
            start_ms: start,
            duration_ms: (end - start).max(1.0),
            frequency: midi::key_to_freq(n.key),
            volume: midi::velocity_to_volume(n.velocity),
        });
    }
    Ok(out)
}

/// 合并/单音化/移调/倍速后的最终音符序列，与生成代码完全一致，用于"转换结果"试听
pub fn convert_preview(options: &ConvertOptions) -> Result<Vec<PreviewNote>, String> {
    let bytes =
        std::fs::read(&options.path).map_err(|e| format!("无法读取文件 {}: {e}", options.path))?;
    let notes = convert_bytes(&bytes, options)?;
    let mut t = 0.0;
    let mut out = Vec::with_capacity(notes.len());
    for n in &notes {
        out.push(PreviewNote {
            start_ms: t,
            duration_ms: n.duration_ms as f64,
            frequency: n.frequency,
            volume: n.volume,
        });
        t += n.duration_ms as f64;
    }
    Ok(out)
}

pub fn convert(options: &ConvertOptions) -> Result<Generated, String> {
    let bytes =
        std::fs::read(&options.path).map_err(|e| format!("无法读取文件 {}: {e}", options.path))?;
    let parsed = midi::parse(&bytes)?;

    if options.tracks.is_empty() {
        return Err("请至少选择一条轨道".to_string());
    }
    let mut selected = Vec::new();
    for &i in &options.tracks {
        let t = parsed
            .tracks
            .get(i)
            .ok_or_else(|| format!("轨道序号 {i} 越界"))?;
        selected.push(t);
    }

    let strategy = match options.strategy.as_str() {
        "lowest" => MonoStrategy::Lowest,
        _ => MonoStrategy::Highest,
    };

    let flat = midi::flatten(&selected, strategy);
    if flat.is_empty() {
        return Err("所选轨道没有任何音符".to_string());
    }

    let (notes, warnings) = midi::build_song(&flat, &parsed.tempo_map, parsed.ppq, options);

    let ident = codegen::sanitize_ident(&options.song_name);
    let song_c = codegen::generate_song_c(&ident, &notes, options.master_volume, options.gap_ms);
    let song_h = codegen::generate_song_h(&ident);

    let note_count = notes.iter().filter(|n| n.frequency > 0.0).count() as u32;
    let rest_count = notes.len() as u32 - note_count;
    let duration_ms = notes.iter().map(|n| n.duration_ms as u64).sum();

    Ok(Generated {
        base_name: ident,
        song_c,
        song_h,
        player_c: player_template::PLAYER_C.to_string(),
        player_h: player_template::PLAYER_H.to_string(),
        note_count,
        rest_count,
        duration_ms,
        warnings,
    })
}

/// 供测试与示例复用：直接从字节流转换
pub fn convert_bytes(bytes: &[u8], options: &ConvertOptions) -> Result<Vec<SongNote>, String> {
    let parsed = midi::parse(bytes)?;
    let mut selected = Vec::new();
    for &i in &options.tracks {
        selected.push(
            parsed
                .tracks
                .get(i)
                .ok_or_else(|| format!("轨道序号 {i} 越界"))?,
        );
    }
    let strategy = match options.strategy.as_str() {
        "lowest" => MonoStrategy::Lowest,
        _ => MonoStrategy::Highest,
    };
    let flat = midi::flatten(&selected, strategy);
    if flat.is_empty() {
        return Err("所选轨道没有任何音符".to_string());
    }
    Ok(midi::build_song(&flat, &parsed.tempo_map, parsed.ppq, options).0)
}
