//! 生成一个用于端到端测试的示例 MIDI：examples/sample.mid
//! 运行：cargo run -p midi-buzzer-core --example make_sample

use midly::num::{u15, u24, u28, u4, u7};
use midly::{Format, Header, MetaMessage, MidiMessage, Smf, Timing, Track, TrackEvent, TrackEventKind};

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
fn program(ch: u8, p: u8) -> TrackEventKind<'static> {
    TrackEventKind::Midi {
        channel: u4::from(ch),
        message: MidiMessage::ProgramChange {
            program: u7::from(p),
        },
    }
}

fn main() {
    let ppq = 480u16;

    // 轨 0：tempo 轨。120BPM 起步，在 tick 2400 处提速到 150BPM
    let tempo_track: Track = vec![
        ev(0, TrackEventKind::Meta(MetaMessage::TrackName(b"Tempo"))),
        ev(0, tempo(500_000)),
        ev(2400, tempo(400_000)),
    ];

    // 轨 1：旋律（C 大调，含一个 C 大三和弦、一个后半的休止、力度变化）
    let melody: Track = vec![
        ev(0, TrackEventKind::Meta(MetaMessage::TrackName(b"Melody"))),
        ev(0, program(0, 0)),
        ev(0, on(0, 60, 100)),   // C4
        ev(480, off(0, 60)),
        ev(0, on(0, 64, 96)),    // E4
        ev(480, off(0, 64)),
        ev(0, on(0, 67, 88)),    // G4
        ev(480, off(0, 67)),
        ev(0, on(0, 72, 100)),   // C5
        ev(480, off(0, 72)),
        // C 大三和弦（tick 1920..2400）
        ev(0, on(0, 60, 90)),
        ev(0, on(0, 64, 90)),
        ev(0, on(0, 67, 90)),
        ev(480, off(0, 60)),
        ev(0, off(0, 64)),
        ev(0, off(0, 67)),
        // tempo 在 2400 变为 150BPM；D5 二分音符（2400..3360）
        ev(0, on(0, 74, 104)),
        ev(960, off(0, 74)),
        // 休止一拍（3360..3840），结尾 C5（3840..4320）
        ev(480, on(0, 72, 112)),
        ev(480, off(0, 72)),
    ];

    // 轨 2：低音（与旋律重叠，验证手动选轨与单音化）
    let bass: Track = vec![
        ev(0, TrackEventKind::Meta(MetaMessage::TrackName(b"Bass"))),
        ev(0, program(0, 32)),
        ev(0, on(1, 48, 80)),    // C3
        ev(960, off(1, 48)),
        ev(0, on(1, 43, 80)),    // G2
        ev(960, off(1, 43)),
        ev(0, on(1, 48, 80)),    // C3
        ev(1440, off(1, 48)),
    ];

    let smf = Smf {
        header: Header {
            format: Format::Parallel,
            timing: Timing::Metrical(u15::from(ppq)),
        },
        tracks: vec![tempo_track, melody, bass],
    };

    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("sample.mid");
    let mut buf = Vec::new();
    smf.write_std(&mut buf).unwrap();
    std::fs::write(&path, &buf).unwrap();
    println!("written: {}", path.display());
}
