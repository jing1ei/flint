//! Parsing FFmpeg's `-progress pipe:1` stream, plus the tiny bit of maths that turns it into a
//! percentage. Kept separate (and pure) because a wrong progress bar is the #1 way a converter
//! feels broken even when it works.

use serde::{Deserialize, Serialize};

/// Which half of a job a progress update is about.
///
/// A dropped file has one phase and always will. A *link* has two - yt-dlp fetching it, then the
/// ordinary conversion - and they are separately measurable: 42% of a download and 42% of an
/// encode are different facts, and a bar that mixed them would run to 100% twice. Carried on the
/// existing [`crate::engine::ProgressUpdate`] rather than on a second event channel, so every
/// consumer that already renders progress renders this too, and an older UI that ignores the field
/// simply sees the fraction it always saw.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// yt-dlp is fetching a link into the job's scratch directory.
    Downloading,
    /// The ordinary pipeline: this is what every dropped file reports, and the default.
    #[default]
    Converting,
}

impl Phase {
    pub const fn id(self) -> &'static str {
        match self {
            Phase::Downloading => "downloading",
            Phase::Converting => "converting",
        }
    }

    /// The word the UI puts in front of the percentage.
    pub const fn label(self) -> &'static str {
        match self {
            Phase::Downloading => "Downloading",
            Phase::Converting => "Converting",
        }
    }
}

/// The two numbers a progress block contributes to the UI. FFmpeg prints more (`frame`, `fps`,
/// `total_size`, `bitrate`); nothing reads them, so they are not carried around pretending to be
/// useful.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProgressSample {
    /// Position in the output, in seconds.
    pub out_time_secs: f64,
    /// Encoding speed relative to realtime (`1.0` = realtime).
    pub speed: Option<f64>,
}

/// Incremental parser: feed it lines, ask it for the latest sample.
#[derive(Debug, Default)]
pub struct ProgressParser {
    current: ProgressSample,
}

impl ProgressParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one line of `key=value`. Returns `Some(sample)` once a block completes
    /// (FFmpeg terminates every block with `progress=continue|end`).
    pub fn push_line(&mut self, line: &str) -> Option<ProgressSample> {
        let line = line.trim();
        let (key, value) = line.split_once('=')?;
        let value = value.trim();
        match key.trim() {
            "out_time_us" | "out_time_ms" => {
                // Careful: FFmpeg's `out_time_ms` is actually microseconds (a long standing quirk).
                if let Ok(us) = value.parse::<i64>() {
                    self.current.out_time_secs = (us.max(0) as f64) / 1_000_000.0;
                }
            }
            "out_time" => {
                if let Some(secs) = parse_timestamp(value) {
                    self.current.out_time_secs = secs;
                }
            }
            "speed" => {
                self.current.speed = value
                    .trim_end_matches('x')
                    .trim()
                    .parse::<f64>()
                    .ok()
                    .filter(|v| v.is_finite() && *v >= 0.0);
            }
            // Every block ends with `progress=continue|end`; either way the sample is complete.
            "progress" => return Some(self.current.clone()),
            _ => {}
        }
        None
    }
}

/// `00:01:23.45` -> 83.45 seconds. Returns `None` for FFmpeg's `N/A` placeholder.
///
/// Shared with [`crate::link`], which reads yt-dlp's `ETA 00:07` off a download line: same colon
/// separated shape, same "we were told nothing" placeholders (`N/A`, `Unknown`), and two copies of
/// it were two chances to fix a rounding or a placeholder in one place only.
pub fn parse_timestamp(v: &str) -> Option<f64> {
    if v.contains("N/A") {
        return None;
    }
    let mut secs = 0.0f64;
    for part in v.split(':') {
        let part = part.parse::<f64>().ok()?;
        if !part.is_finite() || part < 0.0 {
            return None;
        }
        secs = secs * 60.0 + part;
    }
    secs.is_finite().then_some(secs)
}

/// Overall job percentage from the current step's position.
///
/// `step_weights` lets a two-step job (decode + encode) show one smooth bar.
pub fn overall_fraction(step_index: usize, step_weights: &[f32], step_fraction: f32) -> f32 {
    let total: f32 = step_weights.iter().sum();
    if total <= 0.0 {
        return 0.0;
    }
    let done: f32 = step_weights.iter().take(step_index).sum();
    let current = step_weights.get(step_index).copied().unwrap_or(0.0);
    ((done + current * step_fraction.clamp(0.0, 1.0)) / total).clamp(0.0, 1.0)
}

/// Step fraction from a duration, if we know one. Falls back to `None` so the UI can show an
/// indeterminate bar rather than a lying number.
pub fn step_fraction(out_time_secs: f64, duration_secs: Option<f64>) -> Option<f32> {
    let d = duration_secs?;
    if !d.is_finite() || d <= 0.0 || !out_time_secs.is_finite() {
        return None;
    }
    Some(((out_time_secs / d) as f32).clamp(0.0, 1.0))
}

/// Remaining seconds, from progress + measured speed.
pub fn eta_secs(out_time_secs: f64, duration_secs: Option<f64>, speed: Option<f64>) -> Option<f64> {
    let d = duration_secs?;
    if !d.is_finite() || d <= 0.0 || !out_time_secs.is_finite() {
        return None;
    }
    let speed = speed.filter(|s| s.is_finite() && *s > 0.01)?;
    let remaining_media = (d - out_time_secs).max(0.0);
    Some(remaining_media / speed)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_numbers_cannot_escape_into_progress_or_eta() {
        for value in ["NaN", "inf", "-1", "00:NaN", "1e308:1e308"] {
            assert_eq!(parse_timestamp(value), None);
        }
        let mut parser = ProgressParser::new();
        parser.push_line("speed=NaNx");
        parser.push_line("out_time=NaN");
        let sample = parser.push_line("progress=continue").unwrap();
        assert_eq!(sample.speed, None);
        assert_eq!(sample.out_time_secs, 0.0);
        for bad in [f64::NAN, f64::INFINITY] {
            assert_eq!(step_fraction(bad, Some(10.0)), None);
            assert_eq!(step_fraction(1.0, Some(bad)), None);
            assert_eq!(eta_secs(1.0, Some(10.0), Some(bad)), None);
        }
    }

    const BLOCK: &str = "frame=120\nfps=59.8\nbitrate=1200.0kbits/s\ntotal_size=524288\n\
out_time_us=2000000\nspeed=2.5x\nprogress=continue\n";

    #[test]
    fn parses_a_progress_block() {
        let mut p = ProgressParser::new();
        let mut last = None;
        for line in BLOCK.lines() {
            if let Some(s) = p.push_line(line) {
                last = Some(s);
            }
        }
        let s = last.expect("block should complete on progress=");
        assert_eq!(s.out_time_secs, 2.0);
        assert_eq!(s.speed, Some(2.5));
    }

    #[test]
    fn a_block_is_only_reported_once_it_is_complete() {
        let mut p = ProgressParser::new();
        assert!(p.push_line("out_time_us=5000000").is_none());
        let s = p.push_line("progress=end").unwrap();
        assert_eq!(s.out_time_secs, 5.0);
    }

    #[test]
    fn ignores_garbage_and_na_values() {
        let mut p = ProgressParser::new();
        assert!(p.push_line("this is not a key value line").is_none());
        assert!(p.push_line("speed=N/A").is_none());
        assert!(p.push_line("out_time=N/A").is_none());
        let s = p.push_line("progress=continue").unwrap();
        assert_eq!(s.speed, None, "an unparsable speed must not become a fake number");
        assert_eq!(s.out_time_secs, 0.0);
    }

    #[test]
    fn parses_hms_timestamps() {
        assert_eq!(parse_timestamp("00:01:23.5"), Some(83.5));
        assert_eq!(parse_timestamp("01:00:00.0"), Some(3600.0));
        assert_eq!(parse_timestamp("N/A"), None);
    }

    #[test]
    fn two_step_jobs_report_one_smooth_bar() {
        let w = [0.5, 0.5];
        assert_eq!(overall_fraction(0, &w, 0.0), 0.0);
        assert_eq!(overall_fraction(0, &w, 1.0), 0.5);
        assert_eq!(overall_fraction(1, &w, 0.5), 0.75);
        assert_eq!(overall_fraction(1, &w, 1.0), 1.0);
        // out of range input never produces a bar that goes backwards or past 100%
        assert_eq!(overall_fraction(1, &w, 9.0), 1.0);
        assert_eq!(overall_fraction(9, &w, 1.0), 1.0);
    }

    #[test]
    fn unknown_duration_gives_an_indeterminate_bar_instead_of_a_lie() {
        assert_eq!(step_fraction(10.0, None), None);
        assert_eq!(step_fraction(10.0, Some(0.0)), None);
        assert_eq!(step_fraction(5.0, Some(10.0)), Some(0.5));
        assert_eq!(step_fraction(50.0, Some(10.0)), Some(1.0));
    }

    #[test]
    fn eta_uses_measured_speed() {
        assert_eq!(eta_secs(10.0, Some(30.0), Some(2.0)), Some(10.0));
        assert_eq!(eta_secs(10.0, Some(30.0), None), None);
        assert_eq!(eta_secs(40.0, Some(30.0), Some(2.0)), Some(0.0));
    }

    /// Under a trim the duration these two functions are given is the length of the *output*, not of
    /// the source - `queue::run_conversion` is what hands it over. Measured against the source, a 10
    /// second cut of a 10 minute film sits at 2% and finishes with an ETA of eight minutes, which is
    /// the kind of bar that makes a working converter feel broken.
    #[test]
    fn a_trimmed_clip_is_measured_against_the_trimmed_length() {
        use crate::settings::TrimSettings;

        let trim = TrimSettings { enabled: true, start_secs: 30.0, length_secs: 10.0 };
        let film = Some(600.0);
        let expected = trim.expected_output_secs(film);
        assert_eq!(expected, Some(10.0));

        // Five seconds into a ten second cut is half done, with five seconds left at 1x - not 595
        // seconds of a film nobody asked to convert.
        assert_eq!(step_fraction(5.0, expected), Some(0.5));
        assert_eq!(eta_secs(5.0, expected, Some(1.0)), Some(5.0));
        assert_eq!(step_fraction(5.0, film), Some(0.008333334), "the lie this replaces");
        assert_eq!(eta_secs(5.0, film, Some(1.0)), Some(595.0), "...and the ETA that came with it");

        // A clip shorter than the trim is measured against its own length, so it still reaches 100%.
        let short = trim.expected_output_secs(Some(34.0));
        assert_eq!(short, Some(4.0));
        assert_eq!(step_fraction(4.0, short), Some(1.0));

        // And an unknown source duration stays an indeterminate bar rather than becoming "10s".
        assert_eq!(step_fraction(5.0, trim.expected_output_secs(None)), None);
    }

    /// The wire names the UI switches on. Changing one of these is changing the contract.
    #[test]
    fn a_phase_is_named_the_same_way_everywhere() {
        assert_eq!(Phase::default(), Phase::Converting, "a dropped file has one phase");
        assert_eq!(serde_json::to_value(Phase::Downloading).unwrap(), "downloading");
        assert_eq!(serde_json::to_value(Phase::Converting).unwrap(), "converting");
        for phase in [Phase::Downloading, Phase::Converting] {
            assert_eq!(serde_json::to_value(phase).unwrap(), phase.id());
            assert!(phase.label().starts_with(|c: char| c.is_uppercase()), "{}", phase.label());
        }
    }
}
