//! 音频文件解码：任意格式 → f32 单声道 → 重采样到模型采样率（22050 Hz）

use std::path::Path;

use rubato::{FftFixedInOut, Resampler};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

pub const MODEL_SAMPLE_RATE: u32 = 22050;

/// 解码音频文件并混成单声道，返回 (采样, 原始采样率)
pub fn decode_to_mono(path: &Path) -> Result<(Vec<f32>, u32), String> {
    let file =
        std::fs::File::open(path).map_err(|e| format!("无法打开音频 {}: {e}", path.display()))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|_| "无法识别的音频格式（支持 mp3/wav/flac/ogg/m4a）".to_string())?;
    let mut format = probed.format;
    let track = format.default_track().ok_or("音频文件中没有音轨")?;
    let track_id = track.id;
    let sample_rate = track
        .codec_params
        .sample_rate
        .ok_or("无法确定音频采样率")?;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| format!("不支持的音频编码: {e}"))?;

    let mut mono: Vec<f32> = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymphoniaError::IoError(e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(SymphoniaError::ResetRequired) => {
                return Err("音频流中途重置，暂不支持该文件".to_string());
            }
            Err(e) => return Err(format!("读取音频数据失败: {e}")),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            // 个别坏包跳过，不中断整首转换
            Err(_) => continue,
        };
        let spec = *decoded.spec();
        let n_channels = spec.channels.count().max(1);
        let mut sbuf = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        sbuf.copy_interleaved_ref(decoded);
        let interleaved = sbuf.samples();
        if n_channels == 1 {
            mono.extend_from_slice(interleaved);
        } else {
            let nc = n_channels as f32;
            for frame in interleaved.chunks_exact(n_channels) {
                mono.push(frame.iter().sum::<f32>() / nc);
            }
        }
    }
    if mono.is_empty() {
        return Err("音频内容为空或无法解码".to_string());
    }
    Ok((mono, sample_rate))
}

/// 重采样到模型要求的 22050 Hz
pub fn resample_to_model_rate(samples: &[f32], from_sr: u32) -> Result<Vec<f32>, String> {
    if from_sr == MODEL_SAMPLE_RATE {
        return Ok(samples.to_vec());
    }
    if from_sr == 0 {
        return Err("采样率非法".to_string());
    }
    const CHUNK: usize = 8192;
    let mut resampler =
        FftFixedInOut::<f32>::new(from_sr as usize, MODEL_SAMPLE_RATE as usize, CHUNK, 1)
            .map_err(|e| format!("重采样器创建失败: {e}"))?;
    let estimated = samples.len() as u64 * MODEL_SAMPLE_RATE as u64 / from_sr as u64;
    let mut out: Vec<f32> = Vec::with_capacity(estimated as usize + CHUNK);
    let mut pos = 0usize;
    while pos < samples.len() {
        let want = resampler.input_frames_next();
        if pos + want <= samples.len() {
            let chunk = resampler
                .process(&[&samples[pos..pos + want]], None)
                .map_err(|e| format!("重采样失败: {e}"))?;
            out.extend_from_slice(&chunk[0]);
            pos += want;
        } else {
            // FFT 重采样会把尾部补零到整块，多出的尾巴最后统一截掉
            let chunk = resampler
                .process_partial(Some(&[&samples[pos..]]), None)
                .map_err(|e| format!("重采样失败: {e}"))?;
            out.extend_from_slice(&chunk[0]);
            break;
        }
    }
    let expected =
        (samples.len() as u64 * MODEL_SAMPLE_RATE as u64 + from_sr as u64 / 2) / from_sr as u64;
    out.truncate(expected as usize);
    Ok(out)
}

/// 解码 + 混单声道 + 重采样，一步到位
pub fn load_for_model(path: &Path) -> Result<Vec<f32>, String> {
    let (samples, sr) = decode_to_mono(path)?;
    resample_to_model_rate(&samples, sr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resample_halves_44k1() {
        // 44100Hz、1 秒 440Hz 正弦 → 22050Hz 约 22050 点
        let sr = 44100usize;
        let samples: Vec<f32> = (0..sr)
            .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / sr as f32).sin())
            .collect();
        let out = resample_to_model_rate(&samples, sr as u32).unwrap();
        let expect = MODEL_SAMPLE_RATE as usize;
        let diff = (out.len() as i64 - expect as i64).unsigned_abs() as usize;
        assert!(diff < expect / 20, "重采样长度 {} 与期望 {expect} 偏差过大", out.len());
    }

    #[test]
    fn resample_passthrough_at_model_rate() {
        let samples = vec![0.1f32; 1000];
        let out = resample_to_model_rate(&samples, MODEL_SAMPLE_RATE).unwrap();
        assert_eq!(out, samples);
    }

    #[test]
    fn decode_wav_fixture() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/melody.wav");
        if !path.is_file() {
            eprintln!("跳过：未找到 {}", path.display());
            return;
        }
        let (samples, sr) = decode_to_mono(&path).unwrap();
        assert_eq!(sr, MODEL_SAMPLE_RATE);
        assert!(samples.len() > MODEL_SAMPLE_RATE as usize * 3, "旋律应超过 3 秒");
    }
}
