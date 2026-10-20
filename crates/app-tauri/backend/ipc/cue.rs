//! Cue tones — the audible half of the notification rule.
//!
//! The webview decides WHICH event deserves a cue and whether the switch is on;
//! the shell owns playback. A hidden or minimized webview can have its audio
//! stalled, and a cue that only rings while the window is in front is one the
//! user misses exactly when it matters — the shell stays alive in the tray
//! either way.
//!
//! The phrase is synthesised rather than shipped: one WAV builder covers all
//! three cues, with no asset to load before the first tone and no per-platform
//! clip to keep in sync.

use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

/// One short phrase per state that stops the agent loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cue {
    /// The turn ended on its own.
    Turn,
    /// The agent is blocked on the user.
    Attention,
    /// It gave up.
    Error,
}

impl Cue {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "turn" => Some(Self::Turn),
            "attention" => Some(Self::Attention),
            "error" => Some(Self::Error),
            _ => None,
        }
    }

    /// `(start s, duration s, frequency Hz)` per note — rising for a finish,
    /// twice-flat for "you are needed", falling for a failure. The shape
    /// carries the meaning, so the cues stay distinguishable without looking.
    fn notes(self) -> &'static [(f32, f32, f32)] {
        match self {
            Self::Turn => &[(0.0, 0.10, 784.0), (0.09, 0.14, 1047.0)],
            Self::Attention => &[(0.0, 0.09, 988.0), (0.13, 0.12, 988.0)],
            Self::Error => &[(0.0, 0.12, 523.0), (0.12, 0.18, 392.0)],
        }
    }

    /// End of the last note, in seconds.
    fn end(self) -> f32 {
        self.notes()
            .iter()
            .map(|(start, dur, _)| start + dur)
            .fold(0.0, f32::max)
    }
}

const SAMPLE_RATE: u32 = 44_100;
/// Peak amplitude: quiet enough to sit under the desktop's own alerts, loud
/// enough to be heard over a fan.
const PEAK: f32 = 0.22;
/// Attack ramp — a square edge clicks, and a cue that clicks is worse than none.
const ATTACK_S: f32 = 0.008;
/// Tail after the last note, so the file does not end on a hard cut.
const TAIL_S: f32 = 0.06;

/// The phrase as a mono 16-bit PCM WAV.
pub fn wav(cue: Cue) -> Vec<u8> {
    let frames = (((cue.end() + TAIL_S) * SAMPLE_RATE as f32).ceil() as usize).max(1);
    let mut samples = vec![0f32; frames];
    for &(start, dur, freq) in cue.notes() {
        let from = (start * SAMPLE_RATE as f32).round() as usize;
        let to = (((start + dur) * SAMPLE_RATE as f32).round() as usize).min(frames);
        for (i, sample) in samples.iter_mut().enumerate().take(to).skip(from) {
            let t = i as f32 / SAMPLE_RATE as f32 - start;
            let env = if t < ATTACK_S {
                t / ATTACK_S
            } else {
                (1.0 - (t - ATTACK_S) / (dur - ATTACK_S).max(f32::EPSILON)).clamp(0.0, 1.0)
            };
            *sample += (t * freq * std::f32::consts::TAU).sin() * env;
        }
    }

    let data_len = (frames * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    out.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes()); // byte rate
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        let v = (s.clamp(-1.0, 1.0) * PEAK * i16::MAX as f32) as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// Play one cue. Fire-and-forget: a machine with no player must not turn "no
/// sound" into an error the UI has to show, and the IPC reply must not wait for
/// the tone to finish.
pub fn play(cue: Cue) {
    std::thread::Builder::new()
        .name("cue-tone".into())
        .spawn(move || {
            if let Err(e) = play_blocking(cue) {
                eprintln!("[cue] playback failed: {e}");
            }
        })
        .ok();
}

fn play_blocking(cue: Cue) -> Result<(), String> {
    let path = temp_path();
    std::fs::write(&path, wav(cue)).map_err(|e| format!("{}: {e}", path.display()))?;
    let played = play_file(&path);
    let _ = std::fs::remove_file(&path);
    played
}

static SEQ: AtomicU64 = AtomicU64::new(1);

fn temp_path() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "husk-cue-{}-{}.wav",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ))
}

#[cfg(target_os = "linux")]
fn play_file(path: &Path) -> Result<(), String> {
    // PulseAudio / PipeWire first — they honour the default sink and are what
    // the desktop mixes through — raw ALSA last.
    let players: [(&str, &[&str]); 3] = [("paplay", &[]), ("pw-play", &[]), ("aplay", &["-q"])];
    for (program, args) in players {
        match Command::new(program).args(args).arg(path).status() {
            Ok(status) if status.success() => return Ok(()),
            Ok(_) => continue,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(format!("{program}: {e}")),
        }
    }
    Err("no audio player found (paplay / pw-play / aplay)".into())
}

#[cfg(target_os = "macos")]
fn play_file(path: &Path) -> Result<(), String> {
    let status = Command::new("afplay")
        .arg(path)
        .status()
        .map_err(|e| format!("afplay: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("afplay exited with {status}"))
    }
}

#[cfg(target_os = "windows")]
fn play_file(path: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    // The shell is a GUI binary: a spawned console must never flash.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let script = format!(
        "(New-Object Media.SoundPlayer '{}').PlaySync()",
        path.display().to_string().replace('\'', "''")
    );
    let status = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map_err(|e| format!("powershell: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("powershell exited with {status}"))
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn play_file(_path: &Path) -> Result<(), String> {
    Err("no cue player on this platform".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_cues_are_ignored() {
        assert_eq!(Cue::parse("turn"), Some(Cue::Turn));
        assert_eq!(Cue::parse("attention"), Some(Cue::Attention));
        assert_eq!(Cue::parse("error"), Some(Cue::Error));
        assert_eq!(Cue::parse("bell"), None);
    }

    /// Manual check on a real machine: `cargo test -p husk cue -- --ignored
    /// --nocapture`. Ignored by default — CI has no speakers.
    #[test]
    #[ignore = "plays audio on the host"]
    fn plays_each_cue() {
        for cue in [Cue::Turn, Cue::Attention, Cue::Error] {
            play_blocking(cue).expect("cue playback");
            // A breath between phrases, so the three shapes stay distinct.
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
    }

    #[test]
    fn every_phrase_is_a_short_mono_wav() {
        for cue in [Cue::Turn, Cue::Attention, Cue::Error] {
            let bytes = wav(cue);
            assert_eq!(&bytes[0..4], b"RIFF", "{cue:?}");
            assert_eq!(&bytes[8..12], b"WAVE", "{cue:?}");
            assert_eq!(&bytes[36..40], b"data", "{cue:?}");
            let declared = u32::from_le_bytes(bytes[40..44].try_into().unwrap()) as usize;
            assert_eq!(declared, bytes.len() - 44, "{cue:?}");
            // Under half a second: a cue is a nudge, not a jingle.
            assert!(declared < SAMPLE_RATE as usize, "{cue:?}");
            // Audible content, not an empty buffer.
            assert!(
                bytes[44..]
                    .chunks_exact(2)
                    .any(|s| i16::from_le_bytes([s[0], s[1]]) != 0),
                "{cue:?} is silent"
            );
        }
    }
}
