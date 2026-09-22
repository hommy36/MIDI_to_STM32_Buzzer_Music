//! 开发用端到端演示：读取 examples/sample.mid → 转换 → 写出生成的 C 代码到 examples/out/
//! 运行：cargo run -p midi-buzzer-core --example gen_demo

use midi_buzzer_core::{analyze_file, convert, ConvertOptions};

fn main() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
    let sample = dir.join("sample.mid");
    let path = sample.to_string_lossy().to_string();

    let analysis = analyze_file(&path).expect("analyze failed");
    println!(
        "format={} ppq={} bpm={} duration={}ms tracks={}",
        analysis.format, analysis.ppq, analysis.initial_bpm, analysis.duration_ms, analysis.track_count
    );
    for t in &analysis.tracks {
        println!(
            "  track {}: name={:?} program={:?} notes={} range={:?}~{:?}",
            t.index, t.name, t.program, t.note_count, t.lowest, t.highest
        );
    }

    let options = ConvertOptions {
        path,
        tracks: vec![1, 2], // Melody + Bass，验证跨轨单音化
        song_name: "SampleSong".into(),
        transpose: 0,
        speed: 1.0,
        master_volume: 100,
        gap_ms: 25,
        strategy: "highest".into(),
    };
    let gen = convert(&options).expect("convert failed");
    println!(
        "notes={} rests={} duration={}ms warnings={:?}",
        gen.note_count, gen.rest_count, gen.duration_ms, gen.warnings
    );

    let out = dir.join("out");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(out.join(format!("{}.c", gen.base_name)), &gen.song_c).unwrap();
    std::fs::write(out.join(format!("{}.h", gen.base_name)), &gen.song_h).unwrap();
    std::fs::write(out.join("buzzer_player.c"), &gen.player_c).unwrap();
    std::fs::write(out.join("buzzer_player.h"), &gen.player_h).unwrap();
    println!("generated files written to {}", out.display());

    // 打印曲谱前 12 行便于人工核对
    for line in gen.song_c.lines().take(20) {
        println!("{line}");
    }
}
