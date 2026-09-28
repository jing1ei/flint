//! Offline Standard MIDI File rendering with a small, built-in piano-like synthesizer.
use crate::{engine::EngineError, probe::MediaInfo};
use midly::{Format, MetaMessage, MidiMessage, Timing, TrackEventKind};
use std::{
    fs::File,
    io::{BufWriter, Read, Write},
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

const RATE: u32 = 44_100;
const MAX_BYTES: u64 = 16 * 1024 * 1024;
const MAX_EVENTS: usize = 200_000;
const MAX_SECONDS: f64 = 1800.0;
const RELEASE: f64 = 0.35;
const TAIL: f64 = 1.0;

#[derive(Clone, Copy)]
struct Event {
    frame: u64,
    channel: usize,
    message: MidiMessage,
}
struct Score {
    events: Vec<Event>,
    frames: u64,
}
fn invalid(message: impl Into<String>) -> EngineError {
    EngineError::Selection(format!("MIDI: {}", message.into()))
}
fn cancelled(cancel: &Arc<AtomicBool>) -> Result<(), EngineError> {
    if cancel.load(Ordering::Relaxed) {
        Err(EngineError::Cancelled)
    } else {
        Ok(())
    }
}
fn read_score(path: &Path, cancel: &Arc<AtomicBool>) -> Result<Score, EngineError> {
    cancelled(cancel)?;
    let mut bytes = Vec::new();
    File::open(path)?.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(invalid("file exceeds the 16 MB limit"));
    }
    let (header, tracks) =
        midly::parse(&bytes).map_err(|e| invalid(format!("could not read this file ({e})")))?;
    let expected_tracks = tracks.size_hint().0;
    if header.format == Format::Sequential {
        return Err(invalid(
            "type 2 contains independent sequences. Export type 0 or type 1 MIDI first.",
        ));
    }
    let mut timed = Vec::new();
    let mut track_count = 0;
    for (track, events) in tracks.enumerate() {
        cancelled(cancel)?;
        let events = events.map_err(|e| invalid(format!("invalid track ({e})")))?;
        track_count += 1;
        let mut tick = 0u64;
        let mut ended = false;
        for (index, event) in events.enumerate() {
            let event = event.map_err(|e| invalid(format!("invalid event ({e})")))?;
            if index % 1024 == 0 {
                cancelled(cancel)?;
            }
            tick = tick
                .checked_add(event.delta.as_int() as u64)
                .ok_or_else(|| invalid("tick overflow"))?;
            if timed.len() >= MAX_EVENTS {
                return Err(invalid("more than 200000 events"));
            }
            timed.push((tick, track, index, event.kind));
            if matches!(event.kind, TrackEventKind::Meta(MetaMessage::EndOfTrack)) {
                ended = true;
                break;
            }
        }
        if !ended {
            return Err(invalid("track is missing its end-of-track event"));
        }
    }
    if track_count != expected_tracks || (header.format == Format::SingleTrack && track_count != 1)
    {
        return Err(invalid("track count does not match the MIDI header"));
    }
    timed.sort_by_key(|&(tick, track, index, _)| (tick, track, index));
    let mut seconds_per_tick = match header.timing {
        Timing::Metrical(ticks) if ticks.as_int() > 0 => 0.5 / ticks.as_int() as f64,
        Timing::Timecode(fps, ticks) if ticks > 0 => 1.0 / (fps.as_f32() as f64 * ticks as f64),
        _ => return Err(invalid("time division must be positive")),
    };
    let mut seconds = 0.0;
    let mut previous = 0;
    let mut events = Vec::new();
    let mut has_notes = false;
    for (index, (tick, _, _, kind)) in timed.into_iter().enumerate() {
        if index % 1024 == 0 {
            cancelled(cancel)?;
        }
        seconds += (tick - previous) as f64 * seconds_per_tick;
        previous = tick;
        if seconds > MAX_SECONDS {
            return Err(invalid("duration exceeds the 30 minute limit"));
        }
        match kind {
            TrackEventKind::Meta(MetaMessage::Tempo(tempo)) => {
                if tempo.as_int() == 0 {
                    return Err(invalid("tempo must be positive"));
                }
                if let Timing::Metrical(ticks) = header.timing {
                    seconds_per_tick = tempo.as_int() as f64 / 1_000_000.0 / ticks.as_int() as f64;
                }
            }
            TrackEventKind::Midi { channel, message } => {
                if matches!(message, MidiMessage::NoteOn { vel, .. } if vel.as_int() > 0) {
                    has_notes = true;
                }
                events.push(Event {
                    frame: (seconds * RATE as f64).round() as u64,
                    channel: channel.as_int() as usize,
                    message,
                });
            }
            _ => {}
        }
    }
    if !has_notes {
        return Err(invalid("no playable notes found"));
    }
    Ok(Score { events, frames: ((seconds + TAIL) * RATE as f64).ceil() as u64 })
}

pub fn probe(path: &Path, cancel: &Arc<AtomicBool>) -> Option<MediaInfo> {
    let score = read_score(path, cancel).ok()?;
    Some(MediaInfo {
        duration_secs: Some(score.frames as f64 / RATE as f64),
        has_audio: true,
        ..MediaInfo::default()
    })
}

#[derive(Clone, Copy)]
struct Channel {
    sustain: bool,
    volume: f64,
    expression: f64,
    pan: f64,
    bend: f64,
}
impl Default for Channel {
    fn default() -> Self {
        Self { sustain: false, volume: 100.0 / 127.0, expression: 1.0, pan: 0.5, bend: 0.0 }
    }
}
struct Voice {
    channel: usize,
    key: u8,
    held: bool,
    age: u64,
    released: Option<u64>,
    gain: f64,
    freq: f64,
    phase: [f64; 6],
    step: [f64; 6],
    amplitude: [f64; 6],
    decay: [f64; 6],
}
impl Voice {
    fn new(channel: usize, key: u8, velocity: u8, bend: f64) -> Self {
        let freq = 440.0 * 2.0f64.powf((key as f64 - 69.0) / 12.0);
        let mut voice = Self {
            channel,
            key,
            held: true,
            age: 0,
            released: None,
            gain: (velocity as f64 / 127.0).powf(1.5) * 0.18,
            freq,
            phase: [0.0; 6],
            step: [0.0; 6],
            amplitude: [1.0, 0.42, 0.19, 0.1, 0.055, 0.03],
            decay: [0.0; 6],
        };
        for i in 0..6 {
            let time = (3.8 * (261.63 / freq).sqrt()).clamp(0.7, 9.0) / (1.0 + i as f64 * 0.9);
            voice.decay[i] = (-1.0 / (time * RATE as f64)).exp();
        }
        voice.tune(bend);
        voice
    }
    fn tune(&mut self, bend: f64) {
        for i in 0..6 {
            // Slightly stretched overtones and faster treble decay approximate struck piano strings.
            let partial = (i + 1) as f64;
            let freq = self.freq
                * 2.0f64.powf(bend / 12.0)
                * partial
                * (1.0 + 0.00012 * partial * partial).sqrt();
            self.step[i] = if freq < RATE as f64 * 0.48 {
                std::f64::consts::TAU * freq / RATE as f64
            } else {
                0.0
            };
        }
    }
    fn sample(&mut self) -> f64 {
        let attack = (self.age as f64 / (RATE as f64 * 0.004)).min(1.0);
        let release = self
            .released
            .map(|age| (1.0 - (self.age - age) as f64 / (RATE as f64 * RELEASE)).max(0.0))
            .unwrap_or(1.0);
        let mut value = 0.0;
        for i in 0..6 {
            if self.step[i] > 0.0 {
                value += self.phase[i].sin() * self.amplitude[i];
            }
            self.phase[i] = (self.phase[i] + self.step[i]) % std::f64::consts::TAU;
            self.amplitude[i] *= self.decay[i];
        }
        self.age += 1;
        value * self.gain * attack * release
    }
    fn alive(&self) -> bool {
        self.amplitude[0] * self.gain > 0.00001
            && self.released.is_none_or(|age| self.age - age < (RATE as f64 * RELEASE) as u64)
    }
    fn release(&mut self) {
        if self.released.is_none() {
            self.released = Some(self.age);
        }
    }
}

fn event(
    event: Event,
    channels: &mut [Channel; 16],
    voices: &mut Vec<Voice>,
) -> Result<(), EngineError> {
    let c = &mut channels[event.channel];
    match event.message {
        MidiMessage::NoteOn { key, vel } if vel.as_int() > 0 => {
            if voices.len() >= 256 {
                return Err(invalid("more than 256 simultaneous piano voices"));
            }
            voices.push(Voice::new(event.channel, key.as_int(), vel.as_int(), c.bend));
        }
        MidiMessage::NoteOff { key, .. } | MidiMessage::NoteOn { key, .. } => {
            if let Some(v) = voices
                .iter_mut()
                .find(|v| v.channel == event.channel && v.key == key.as_int() && v.held)
            {
                v.held = false;
                if !c.sustain {
                    v.release();
                }
            }
        }
        MidiMessage::PitchBend { bend } => {
            c.bend = (bend.0.as_int() as f64 - 8192.0) / 8192.0 * 2.0;
            for v in voices.iter_mut().filter(|v| v.channel == event.channel) {
                v.tune(c.bend);
            }
        }
        MidiMessage::Controller { controller, value } => match controller.as_int() {
            7 => c.volume = value.as_int() as f64 / 127.0,
            11 => c.expression = value.as_int() as f64 / 127.0,
            10 => c.pan = value.as_int() as f64 / 127.0,
            64 => {
                c.sustain = value.as_int() >= 64;
                if !c.sustain {
                    for v in voices.iter_mut().filter(|v| v.channel == event.channel && !v.held) {
                        v.release();
                    }
                }
            }
            120 => voices.retain(|v| v.channel != event.channel),
            123 => {
                for v in voices.iter_mut().filter(|v| v.channel == event.channel) {
                    v.held = false;
                    if !c.sustain {
                        v.release();
                    }
                }
            }
            121 => {
                c.sustain = false;
                c.expression = 1.0;
                c.bend = 0.0;
                for v in voices.iter_mut().filter(|v| v.channel == event.channel) {
                    v.tune(0.0);
                    if !v.held {
                        v.release();
                    }
                }
            }
            _ => {}
        },
        // One piano sound on every channel, including channel 10; program changes are ignored.
        _ => {}
    }
    Ok(())
}

/// Stream stereo PCM into private job scratch. The caller removes scratch on failure/cancel.
pub fn render(
    input: &Path,
    output: &Path,
    cancel: &Arc<AtomicBool>,
    progress: &mut dyn FnMut(f32),
) -> Result<(), EngineError> {
    let score = read_score(input, cancel)?;
    let data_bytes = u32::try_from(score.frames * 4).map_err(|_| invalid("render is too large"))?;
    let mut wav = BufWriter::new(File::options().write(true).create_new(true).open(output)?);
    wav.write_all(b"RIFF")?;
    wav.write_all(&(36 + data_bytes).to_le_bytes())?;
    wav.write_all(b"WAVEfmt ")?;
    wav.write_all(&16u32.to_le_bytes())?;
    wav.write_all(&1u16.to_le_bytes())?;
    wav.write_all(&2u16.to_le_bytes())?;
    wav.write_all(&RATE.to_le_bytes())?;
    wav.write_all(&(RATE * 4).to_le_bytes())?;
    wav.write_all(&4u16.to_le_bytes())?;
    wav.write_all(&16u16.to_le_bytes())?;
    wav.write_all(b"data")?;
    wav.write_all(&data_bytes.to_le_bytes())?;
    let mut channels = [Channel::default(); 16];
    let mut voices = Vec::new();
    let mut cursor = 0;
    let end = score.frames - (TAIL * RATE as f64) as u64;
    for frame in 0..score.frames {
        if frame % 1024 == 0 {
            cancelled(cancel)?;
        }
        if frame % 4410 == 0 {
            progress(frame as f32 / score.frames as f32);
        }
        while cursor < score.events.len() && score.events[cursor].frame <= frame {
            event(score.events[cursor], &mut channels, &mut voices)?;
            cursor += 1;
        }
        if frame == end {
            for v in &mut voices {
                v.release();
            }
        }
        let mut left = 0.0;
        let mut right = 0.0;
        for v in &mut voices {
            let c = channels[v.channel];
            let sample = v.sample() * c.volume * c.expression;
            left += sample * (1.0 - c.pan).sqrt();
            right += sample * c.pan.sqrt();
        }
        voices.retain(Voice::alive);
        for sample in [left, right] {
            let sample = (sample.tanh() * i16::MAX as f64).round() as i16;
            wav.write_all(&sample.to_le_bytes())?;
        }
    }
    cancelled(cancel)?;
    wav.flush()?;
    progress(1.0);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn midi(format: u16, tracks: &[&[u8]]) -> Vec<u8> {
        let mut bytes = b"MThd\0\0\0\x06".to_vec();
        bytes.extend(format.to_be_bytes());
        bytes.extend((tracks.len() as u16).to_be_bytes());
        bytes.extend(480u16.to_be_bytes());
        for track in tracks {
            bytes.extend(b"MTrk");
            bytes.extend((track.len() as u32).to_be_bytes());
            bytes.extend(*track);
        }
        bytes
    }
    fn input(dir: &Path, bytes: &[u8]) -> std::path::PathBuf {
        let path = dir.join("song.mid");
        std::fs::write(&path, bytes).unwrap();
        path
    }
    const NOTE: &[u8] = &[0, 0x90, 69, 100, 0x83, 0x60, 0x80, 69, 0, 0, 0xff, 0x2f, 0];
    #[test]
    fn truncated_event_after_valid_notes_is_not_a_successful_short_song() {
        let dir = tempfile::tempdir().unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let bytes = midi(0, &[&[0, 0x90, 60, 100, 0x83, 0x60, 0x80, 60]]);
        assert!(read_score(&input(dir.path(), &bytes), &cancel).is_err());
    }

    #[test]
    fn missing_declared_track_is_not_silently_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let mut bytes = midi(1, &[NOTE]);
        bytes[11] = 2;
        assert!(read_score(&input(dir.path(), &bytes), &cancel).is_err());
    }

    #[test]
    fn unterminated_track_and_excess_events_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let no_end = midi(0, &[&NOTE[..NOTE.len() - 4]]);
        assert!(read_score(&input(dir.path(), &no_end), &cancel).is_err());
        let mut track = Vec::new();
        for _ in 0..MAX_EVENTS + 1 {
            track.extend([0, 0x90, 60, 1]);
        }
        track.extend([0, 0xff, 0x2f, 0]);
        let result = read_score(&input(dir.path(), &midi(0, &[&track])), &cancel);
        assert!(
            matches!(result, Err(EngineError::Selection(message)) if message.contains("200000"))
        );
    }
    #[test]
    fn running_status_and_smpte_timecode_are_respected() {
        let dir = tempfile::tempdir().unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        // 25 frames/s, 40 ticks/frame: 1000 ticks is one second. Tempo is ignored.
        let track = &[
            0, 0xff, 0x51, 3, 0x0f, 0x42, 0x40, 0, 0x90, 60, 100, 0x87, 0x68, 60, 0, 0, 0xff, 0x2f,
            0,
        ];
        let mut bytes = midi(0, &[track]);
        bytes[12] = 0xe7;
        bytes[13] = 40;
        let score = read_score(&input(dir.path(), &bytes), &cancel).unwrap();
        assert_eq!(score.events.last().unwrap().frame, 44_100);
        assert_eq!(score.frames, 88_200);
    }

    #[test]
    fn tempo_track_controls_other_tracks_and_default_tempo_is_120() {
        let dir = tempfile::tempdir().unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let path = input(dir.path(), &midi(0, &[NOTE]));
        assert_eq!(read_score(&path, &cancel).unwrap().frames, 66_150);
        // At tick 240, change from 120 BPM to 60 BPM. Tick 480 is at 0.75s.
        let tempo = &[0x81, 0x70, 0xff, 0x51, 3, 0x0f, 0x42, 0x40, 0, 0xff, 0x2f, 0];
        let path = input(dir.path(), &midi(1, &[tempo, NOTE]));
        let score = read_score(&path, &cancel).unwrap();
        assert_eq!(score.events.last().unwrap().frame, 33_075);
        assert_eq!(score.frames, 77_175);
    }
    #[test]
    fn piano_wav_has_correct_pitch_duration_and_silent_release_tail() {
        let dir = tempfile::tempdir().unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let path = input(dir.path(), &midi(0, &[NOTE]));
        let wav = dir.path().join("piano.wav");
        render(&path, &wav, &cancel, &mut |_| {}).unwrap();
        let bytes = std::fs::read(wav).unwrap();
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(bytes.len(), 44 + 66_150 * 4);
        let samples: Vec<f64> = bytes[44..]
            .as_chunks::<4>()
            .0
            .iter()
            .map(|f| i16::from_le_bytes([f[0], f[1]]) as f64 / 32768.0)
            .collect();
        let energy: f64 = samples[4410..13230].iter().map(|v| v * v).sum();
        assert!(energy > 1.0);
        let power = |frequency: f64| {
            let (mut re, mut im) = (0.0, 0.0);
            for (i, s) in samples[4410..13230].iter().enumerate() {
                let phase = std::f64::consts::TAU * frequency * i as f64 / RATE as f64;
                re += s * phase.cos();
                im += s * phase.sin();
            }
            re * re + im * im
        };
        assert!(power(440.0) > power(415.0) * 10.0);
        assert!(samples[44100..].iter().all(|v| *v == 0.0));
    }
    #[test]
    fn sustain_velocity_zero_and_channels_release_independently() {
        let mut channels = [Channel::default(); 16];
        let mut voices = Vec::new();
        let mut send = |channel, message| {
            event(Event { frame: 0, channel, message }, &mut channels, &mut voices).unwrap()
        };
        send(0, MidiMessage::Controller { controller: 64.into(), value: 127.into() });
        for channel in [0, 1] {
            send(channel, MidiMessage::NoteOn { key: 60.into(), vel: 100.into() });
        }
        send(0, MidiMessage::NoteOn { key: 60.into(), vel: 0.into() });
        assert!(!voices[0].held && voices[0].released.is_none());
        assert!(voices[1].held);
        event(
            Event {
                frame: 0,
                channel: 0,
                message: MidiMessage::Controller { controller: 64.into(), value: 0.into() },
            },
            &mut channels,
            &mut voices,
        )
        .unwrap();
        assert!(voices[0].released.is_some());
        assert!(voices[1].released.is_none());
    }
    #[test]
    fn malformed_empty_type2_and_oversized_time_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        for bytes in [
            b"bad".to_vec(),
            midi(0, &[&[0, 0xff, 0x2f, 0]]),
            midi(2, &[NOTE]),
            midi(0, &[&[0xff, 0xff, 0xff, 0x7f, 0x90, 60, 100]]),
        ] {
            assert!(read_score(&input(dir.path(), &bytes), &cancel).is_err());
        }
    }
    #[test]
    fn cancellation_interrupts_rendering() {
        let dir = tempfile::tempdir().unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let path = input(dir.path(), &midi(0, &[NOTE]));
        let result = render(&path, &dir.path().join("cancel.wav"), &cancel, &mut |_| {
            cancel.store(true, Ordering::Relaxed)
        });
        assert!(matches!(result, Err(EngineError::Cancelled)));
    }
}
