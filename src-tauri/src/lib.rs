use midi_buzzer_core::midi::MidiAnalysis;
use midi_buzzer_core::{
    analyze_file, convert, convert_preview, track_preview, ConvertOptions, Generated, PreviewNote,
};
use tauri::{AppHandle, Emitter, Manager};

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

// ---------------- 音频转 MIDI（basic-pitch 本地推理） ----------------

const MODEL_URL: &str =
    "https://github.com/spotify/basic-pitch/raw/main/basic_pitch/saved_models/icassp_2022/nmp.onnx";
const MODEL_SIZE: u64 = 230_444;

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AudioProgress {
    stage: String,
    percent: f32,
}

fn emit_progress(app: &AppHandle, stage: &str, percent: f32) {
    let _ = app.emit(
        "audio-progress",
        AudioProgress {
            stage: stage.to_string(),
            percent: percent.clamp(0.0, 1.0),
        },
    );
}

/// 代理解析优先级：HTTPS_PROXY 环境变量 > Windows 系统代理 > 直连。
/// 不写死任何代理地址。
fn proxy_url() -> Option<String> {
    for var in ["HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy"] {
        if let Ok(p) = std::env::var(var) {
            let p = p.trim();
            if !p.is_empty() {
                return Some(normalize_proxy(p));
            }
        }
    }
    #[cfg(windows)]
    {
        windows_system_proxy()
    }
    #[cfg(not(windows))]
    {
        None
    }
}

fn normalize_proxy(p: &str) -> String {
    if p.contains("://") {
        p.to_string()
    } else {
        format!("http://{p}")
    }
}

#[cfg(windows)]
fn windows_system_proxy() -> Option<String> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    let key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Internet Settings")
        .ok()?;
    let enabled: u32 = key.get_value("ProxyEnable").unwrap_or(0);
    if enabled == 0 {
        return None;
    }
    let server: String = key.get_value("ProxyServer").ok()?;
    let server = server.trim();
    if server.is_empty() {
        return None;
    }
    // 形如 "http=h:1;https=h:2;socks=h:3" 或统一的 "h:p"
    if server.contains('=') {
        let mut first: Option<&str> = None;
        for part in server.split(';') {
            let mut kv = part.splitn(2, '=');
            let (k, v) = (kv.next()?.trim(), kv.next().unwrap_or("").trim());
            if v.is_empty() {
                continue;
            }
            if first.is_none() {
                first = Some(v);
            }
            if k.eq_ignore_ascii_case("https") {
                return Some(normalize_proxy(v));
            }
        }
        first.map(normalize_proxy)
    } else {
        Some(normalize_proxy(server))
    }
}

fn http_agent() -> ureq::Agent {
    let mut builder = ureq::AgentBuilder::new().timeout(std::time::Duration::from_secs(120));
    if let Some(p) = proxy_url() {
        if let Ok(proxy) = ureq::Proxy::new(p) {
            builder = builder.proxy(proxy);
        }
    }
    builder.build()
}

fn model_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("无法定位应用数据目录: {e}"))?;
    Ok(dir.join("models").join("nmp.onnx"))
}

/// 确保模型已下载（首次使用从 GitHub 拉取，约 230KB），返回模型文件路径
fn ensure_model(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    let path = model_path(app)?;
    if let Ok(meta) = std::fs::metadata(&path) {
        if meta.len() == MODEL_SIZE {
            return Ok(path);
        }
    }
    let parent = path.parent().ok_or("模型路径非法")?;
    std::fs::create_dir_all(parent).map_err(|e| format!("创建模型目录失败: {e}"))?;

    emit_progress(app, "下载模型", 0.0);
    let resp = http_agent()
        .get(MODEL_URL)
        .call()
        .map_err(|e| format!("模型下载失败: {e}（可配置系统代理或 HTTPS_PROXY 后重试）"))?;
    let total = resp
        .header("Content-Length")
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(MODEL_SIZE);

    use std::io::{Read, Write};
    let tmp = path.with_extension("tmp");
    let mut file =
        std::fs::File::create(&tmp).map_err(|e| format!("无法创建模型文件: {e}"))?;
    let mut reader = resp.into_reader();
    let mut buf = [0u8; 65536];
    let mut got = 0u64;
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("模型下载中断: {e}"))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])
            .map_err(|e| format!("模型写入失败: {e}"))?;
        got += n as u64;
        emit_progress(app, "下载模型", got as f32 / total as f32);
    }
    drop(file);
    if got != MODEL_SIZE {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("模型下载不完整（{got}/{MODEL_SIZE} 字节），请重试"));
    }
    std::fs::rename(&tmp, &path).map_err(|e| format!("模型保存失败: {e}"))?;
    Ok(path)
}

/// 音频文件 → MIDI：解码、AI 推理、写出临时 .mid，返回其路径
#[tauri::command]
async fn convert_audio(app: AppHandle, path: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let model = ensure_model(&app)?;
        let app2 = app.clone();
        let bytes = midi_buzzer_core::audio_to_midi(
            &path,
            &model.to_string_lossy(),
            move |stage, p| emit_progress(&app2, stage, p),
        )?;
        let stem = std::path::Path::new(&path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("audio");
        let out = std::env::temp_dir().join(format!("{stem}_basic_pitch.mid"));
        std::fs::write(&out, bytes)
            .map_err(|e| format!("临时 MIDI 写入失败 {}: {e}", out.display()))?;
        Ok(out.to_string_lossy().into_owned())
    })
    .await
    .map_err(|e| format!("转换任务异常: {e}"))?
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
            export_code,
            convert_audio
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
