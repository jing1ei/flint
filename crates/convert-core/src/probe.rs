//! Reading media metadata out of `ffprobe` JSON.
//!
//! We only need four facts: how long is it, how big is it, does it have sound, and does it move.
//! "Does it move" is what decides whether `clip.gif -> png` means one file or two hundred.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MediaInfo {
    pub duration_secs: Option<f64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub has_video: bool,
    /// Read before an audio-only conversion: a silent screen recording cannot become an MP3, and
    /// saying so up front beats letting FFmpeg fail with "Output file does not contain any stream".
    pub has_audio: bool,
    pub nb_frames: Option<u64>,
    /// More than one frame: video, animated GIF/WebP/APNG, or an image sequence.
    pub is_animated: bool,
}

/// Arguments for a JSON probe of one file.
pub fn ffprobe_args(input: &Path) -> Vec<String> {
    vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-show_format".into(),
        "-show_streams".into(),
        "-of".into(),
        "json".into(),
        input.to_string_lossy().into(),
    ]
}

pub fn parse_ffprobe_json(json: &str) -> Result<MediaInfo> {
    let v: serde_json::Value =
        serde_json::from_str(json).context("ffprobe did not return valid JSON")?;
    let mut info = MediaInfo {
        duration_secs: v
            .get("format")
            .and_then(|f| f.get("duration"))
            .and_then(|d| d.as_str())
            .and_then(|d| d.parse::<f64>().ok())
            .filter(|d| d.is_finite() && *d > 0.0),
        ..Default::default()
    };

    if let Some(streams) = v.get("streams").and_then(|s| s.as_array()) {
        for s in streams {
            match s.get("codec_type").and_then(|t| t.as_str()) {
                Some("video")
                    if s.pointer("/disposition/attached_pic").and_then(|v| v.as_u64())
                        != Some(1) =>
                {
                    info.has_video = true;
                    if info.width.is_none() {
                        info.width = s
                            .get("width")
                            .and_then(|w| w.as_u64())
                            .and_then(|w| u32::try_from(w).ok())
                            .filter(|w| *w > 0);
                        info.height = s
                            .get("height")
                            .and_then(|h| h.as_u64())
                            .and_then(|h| u32::try_from(h).ok())
                            .filter(|h| *h > 0);
                    }
                    let frames = s
                        .get("nb_frames")
                        .and_then(|f| f.as_str())
                        .and_then(|f| f.parse::<u64>().ok())
                        .or_else(|| s.get("nb_frames").and_then(|f| f.as_u64()));
                    if let Some(f) = frames {
                        info.nb_frames = Some(info.nb_frames.unwrap_or(0).max(f));
                    }
                }
                Some("audio") => info.has_audio = true,
                _ => {}
            }
        }
    }

    info.is_animated = match info.nb_frames {
        Some(n) => n > 1,
        // Unknown frame count (typical for GIF/WebP animations and streamed containers):
        // fall back to "has a video stream and a real duration".
        None => info.has_video && info.duration_secs.map(|d| d > 0.04).unwrap_or(false),
    };

    Ok(info)
}

impl MediaInfo {
    pub fn resolution_label(&self) -> Option<String> {
        Some(format!("{}×{}", self.width?, self.height?))
    }

    pub fn duration_label(&self) -> Option<String> {
        Some(seconds_label(self.duration_secs?))
    }
}

/// `0:12`, `2:02:05` - the way this app spells a length of time, wherever it says one.
///
/// Extracted from [`MediaInfo::duration_label`] because the queue names *two* durations in the
/// message it refuses a trim with, and only one of them comes from a probe. Two spellings of a
/// timestamp in one sentence would look like two different kinds of number.
pub fn seconds_label(secs: f64) -> String {
    let total = secs.trunc().max(0.0) as u64;
    if total >= 3600 {
        format!("{}:{:02}:{:02}", total / 3600, (total % 3600) / 60, total % 60)
    } else {
        format!("{}:{:02}", total / 60, total % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIDEO: &str = r#"{
      "streams": [
        {"codec_type":"video","codec_name":"h264","width":1920,"height":1080,"nb_frames":"1500"},
        {"codec_type":"audio","codec_name":"aac"}
      ],
      "format": {"duration":"60.041667","format_name":"mov,mp4,m4a"}
    }"#;

    const STILL_IMAGE: &str = r#"{
      "streams": [{"codec_type":"video","codec_name":"png","width":800,"height":600,"nb_frames":"1"}],
      "format": {"format_name":"png_pipe"}
    }"#;

    const ANIMATED_GIF: &str = r#"{
      "streams": [{"codec_type":"video","codec_name":"gif","width":480,"height":270}],
      "format": {"duration":"3.400000","format_name":"gif"}
    }"#;

    const MP3: &str = r#"{
      "streams": [{"codec_type":"audio","codec_name":"mp3"}],
      "format": {"duration":"213.5","format_name":"mp3"}
    }"#;

    #[test]
    fn reads_video_facts() {
        let i = parse_ffprobe_json(VIDEO).unwrap();
        assert_eq!(i.width, Some(1920));
        assert_eq!(i.height, Some(1080));
        assert!(i.has_video && i.has_audio);
        assert_eq!(i.nb_frames, Some(1500));
        assert!(i.is_animated);
        assert_eq!(i.duration_label().unwrap(), "1:00");
        assert_eq!(i.resolution_label().unwrap(), "1920×1080");
    }

    #[test]
    fn a_still_png_is_not_animated() {
        let i = parse_ffprobe_json(STILL_IMAGE).unwrap();
        assert!(i.has_video);
        assert!(!i.is_animated, "a single frame PNG must not explode into a sequence");
        assert_eq!(i.duration_secs, None);
    }

    #[test]
    fn an_animated_gif_without_frame_count_is_still_animated() {
        let i = parse_ffprobe_json(ANIMATED_GIF).unwrap();
        assert!(i.is_animated);
    }

    #[test]
    fn audio_only_files_have_no_video() {
        let i = parse_ffprobe_json(MP3).unwrap();
        assert!(!i.has_video && i.has_audio && !i.is_animated);
        assert_eq!(i.duration_label().unwrap(), "3:33");
    }

    #[test]
    fn cover_art_does_not_turn_music_into_video_or_flatten_a_movie() {
        let music = parse_ffprobe_json(r#"{"streams":[
            {"codec_type":"audio"},
            {"codec_type":"video","width":600,"height":600,"nb_frames":"1","disposition":{"attached_pic":1}}
        ],"format":{"duration":"240"}}"#).unwrap();
        assert!(music.has_audio);
        assert!(!music.has_video && !music.is_animated);
        let movie = parse_ffprobe_json(
            r#"{"streams":[
            {"codec_type":"video","width":1920,"height":1080,"nb_frames":"100"},
            {"codec_type":"video","nb_frames":"1","disposition":{"attached_pic":1}}
        ],"format":{"duration":"4"}}"#,
        )
        .unwrap();
        assert!(movie.is_animated);
        assert_eq!(movie.nb_frames, Some(100));
    }

    #[test]
    fn invalid_dimensions_and_nonfinite_duration_are_not_accepted() {
        let info = parse_ffprobe_json(r#"{"streams":[{"codec_type":"video","width":4294967296,"height":0}],"format":{"duration":"inf"}}"#).unwrap();
        assert_eq!(info.duration_secs, None);
        assert_eq!((info.width, info.height), (None, None));
    }

    #[test]
    fn long_durations_use_hours() {
        let i = MediaInfo { duration_secs: Some(7325.0), ..Default::default() };
        assert_eq!(i.duration_label().unwrap(), "2:02:05");
    }

    #[test]
    fn bad_json_is_an_error_not_a_panic() {
        assert!(parse_ffprobe_json("<not json>").is_err());
    }

    #[test]
    fn probe_args_ask_for_json() {
        let a = ffprobe_args(Path::new("/in/clip.mp4")).join(" ");
        assert!(a.contains("-of json"));
        assert!(a.ends_with("/in/clip.mp4"));
    }
}
