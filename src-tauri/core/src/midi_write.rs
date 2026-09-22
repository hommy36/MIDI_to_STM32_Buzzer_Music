//! 音符事件 → Standard MIDI File 字节流（单轨，PPQ 480，固定 120 BPM）

use midly::num::{u15, u24, u28, u4, u7};
use midly::{Format, Header, MetaMessage, MidiMessage, Smf, Timing, TrackEvent, TrackEventKind};

use crate::basic_pitch::NoteEvent;

const PPQ: u16 = 480;
/// 120 BPM 下每拍 0.5s，480 tick/拍 → 960 tick/秒
const TICKS_PER_SEC: f64 = PPQ as f64 * 2.0;

fn to_tick(t_s: f32) -> u32 {
    (t_s.max(0.0) as f64 * TICKS_PER_SEC).round() as u32
}

/// 把音符事件序列写成 SMF 字节
pub fn notes_to_smf(name: &str, events: &[NoteEvent]) -> Result<Vec<u8>, String> {
    enum Ev {
        On(u8),
        Off,
    }
    let mut timed: Vec<(u32, u8, Ev)> = Vec::with_capacity(events.len() * 2);
    for e in events {
        let on = to_tick(e.start_s);
        let mut off = to_tick(e.end_s);
        if off <= on {
            off = on + 1;
        }
        let vel = (e.amplitude * 127.0).round().clamp(1.0, 127.0) as u8;
        timed.push((on, e.pitch, Ev::On(vel)));
        timed.push((off, e.pitch, Ev::Off));
    }
    // 同 tick 时先关后开，避免同音重复触发粘连
    timed.sort_by(|a, b| {
        a.0.cmp(&b.0).then_with(|| {
            matches!(b.2, Ev::Off)
                .cmp(&matches!(a.2, Ev::Off))
                .then(a.1.cmp(&b.1))
        })
    });

    let mut track: Vec<TrackEvent> = Vec::with_capacity(timed.len() + 3);
    track.push(TrackEvent {
        delta: u28::new(0),
        kind: TrackEventKind::Meta(MetaMessage::Tempo(u24::new(500_000))),
    });
    track.push(TrackEvent {
        delta: u28::new(0),
        kind: TrackEventKind::Meta(MetaMessage::TrackName(name.as_bytes())),
    });

    let mut last = 0u32;
    for (tick, pitch, ev) in timed {
        let delta = u28::new(tick - last);
        last = tick;
        let kind = TrackEventKind::Midi {
            channel: u4::new(0),
            message: match ev {
                Ev::On(vel) => MidiMessage::NoteOn {
                    key: u7::new(pitch),
                    vel: u7::new(vel),
                },
                Ev::Off => MidiMessage::NoteOff {
                    key: u7::new(pitch),
                    vel: u7::new(0),
                },
            },
        };
        track.push(TrackEvent { delta, kind });
    }
    track.push(TrackEvent {
        delta: u28::new(0),
        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
    });

    let mut smf = Smf::new(Header {
        format: Format::SingleTrack,
        timing: Timing::Metrical(u15::new(PPQ)),
    });
    smf.tracks.push(track);

    let mut buf = Vec::new();
    smf.write_std(&mut buf)
        .map_err(|e| format!("MIDI 序列化失败: {e}"))?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smf_roundtrip() {
        let events = vec![
            NoteEvent {
                start_s: 0.5,
                end_s: 1.0,
                pitch: 69,
                amplitude: 0.8,
            },
            NoteEvent {
                start_s: 1.0,
                end_s: 1.5,
                pitch: 72,
                amplitude: 0.6,
            },
        ];
        let bytes = notes_to_smf("test", &events).unwrap();
        let parsed = crate::midi::parse(&bytes).unwrap();
        assert_eq!(parsed.ppq, PPQ);
        let notes: Vec<_> = parsed.tracks.iter().flat_map(|t| t.notes.iter()).collect();
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].key, 69);
        assert_eq!(notes[1].key, 72);
    }
}
