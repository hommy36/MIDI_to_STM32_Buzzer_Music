//! 命令行转换工具：
//!   cargo run -p midi-buzzer-core --example mbc -- <mid路径> <输出目录> <曲名> [轨号,逗号分隔] [移调] [倍速]
//! 轨号省略时自动选音符最多的轨。始终先打印 analyze 结果。

use midi_buzzer_core::{analyze_file, convert, ConvertOptions};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 3 {
        eprintln!("usage: mbc <mid> <outdir> <song_name> [tracks,csv] [transpose] [speed]");
        std::process::exit(2);
    }
    let (path, outdir, song_name) = (&args[0], &args[1], &args[2]);

    let analysis = analyze_file(path).expect("analyze failed");
    println!(
        "format={} ppq={} bpm={} duration={}ms tracks={}",
        analysis.format, analysis.ppq, analysis.initial_bpm, analysis.duration_ms, analysis.track_count
    );
    for t in &analysis.tracks {
        println!(
            "  track {}: name={:?} ch={:?} instrument={:?} notes={} range={:?}~{:?}",
            t.index, t.name, t.channels, t.instrument, t.note_count, t.lowest, t.highest
        );
    }

    let tracks: Vec<usize> = match args.get(3) {
        Some(csv) => csv.split(',').map(|s| s.trim().parse().expect("bad track index")).collect(),
        None => {
            let best = analysis
                .tracks
                .iter()
                .max_by_key(|t| t.note_count)
                .map(|t| t.index)
                .expect("no tracks");
            println!("auto-selected track {best} (most notes)");
            vec![best]
        }
    };
    let transpose: i32 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(0);
    let speed: f64 = args.get(5).and_then(|s| s.parse().ok()).unwrap_or(1.0);

    let options = ConvertOptions {
        path: path.clone(),
        tracks,
        song_name: song_name.clone(),
        transpose,
        speed,
        master_volume: 100,
        gap_ms: 25,
        strategy: "highest".into(),
    };
    let gen = convert(&options).expect("convert failed");
    println!(
        "notes={} rests={} duration={}ms warnings={:?}",
        gen.note_count, gen.rest_count, gen.duration_ms, gen.warnings
    );

    std::fs::create_dir_all(outdir).unwrap();
    let out = std::path::Path::new(outdir);
    std::fs::write(out.join(format!("{}.c", gen.base_name)), &gen.song_c).unwrap();
    std::fs::write(out.join(format!("{}.h", gen.base_name)), &gen.song_h).unwrap();
    std::fs::write(out.join("buzzer_player.c"), &gen.player_c).unwrap();
    std::fs::write(out.join("buzzer_player.h"), &gen.player_h).unwrap();
    println!("written to {outdir}: {}.c/.h, buzzer_player.c/.h", gen.base_name);
}
