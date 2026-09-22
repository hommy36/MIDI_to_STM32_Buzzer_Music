use midi_buzzer_core::midi::MidiAnalysis;
use midi_buzzer_core::{
    analyze_file, convert, convert_preview, track_preview, ConvertOptions, Generated, PreviewNote,
};

#[tauri::command]
fn analyze_midi(path: String) -> Result<MidiAnalysis, String> {
    analyze_file(&path)
}

#[tauri::command]
fn generate_code(options: ConvertOptions) -> Result<Generated, String> {
    convert(&options)
}

#[tauri::command]
fn track_notes(path: String, track_index: usize) -> Result<Vec<PreviewNote>, String> {
    track_preview(&path, track_index)
}

#[tauri::command]
fn preview_convert(options: ConvertOptions) -> Result<Vec<PreviewNote>, String> {
    convert_preview(&options)
}

#[tauri::command]
fn export_code(
    dir: String,
    base_name: String,
    song_c: String,
    song_h: String,
    player_c: String,
    player_h: String,
    include_player: bool,
) -> Result<Vec<String>, String> {
    let dir = std::path::Path::new(&dir);
    if !dir.is_dir() {
        return Err(format!("目录不存在: {}", dir.display()));
    }

    let mut written = Vec::new();
    for (name, content) in [
        (format!("{base_name}.c"), song_c),
        (format!("{base_name}.h"), song_h),
    ] {
        let path = dir.join(name);
        std::fs::write(&path, content).map_err(|e| format!("写入 {} 失败: {e}", path.display()))?;
        written.push(path.display().to_string());
    }

    if include_player {
        for (name, content) in [
            ("buzzer_player.c", player_c),
            ("buzzer_player.h", player_h),
        ] {
            let path = dir.join(name);
            if path.exists() {
                // 播放器是通用文件，不覆盖用户可能的本地修改
                continue;
            }
            std::fs::write(&path, content)
                .map_err(|e| format!("写入 {} 失败: {e}", path.display()))?;
            written.push(path.display().to_string());
        }
    }

    Ok(written)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            analyze_midi,
            generate_code,
            track_notes,
            preview_convert,
            export_code
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
