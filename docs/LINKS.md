# Link Sources

Paste individual media links through File > Paste Links (`Cmd/Ctrl+L`).
The queue accepts at most 20 links alongside local files.

| Service | Supported URL Shape |
| --- | --- |
| YouTube | `https://www.youtube.com/watch?v=ID`, `https://youtu.be/ID`, shorts and embeds |
| Bilibili | `https://www.bilibili.com/video/ID`, `https://b23.tv/ID` |
| QQ Music | `https://y.qq.com/n/ryqq/songDetail/ID` |
| NetEase Music | `https://music.163.com/song?id=ID`, `https://music.163.com/#/song?id=ID`, `https://y.music.163.com/m/song?id=ID` |
| SoundCloud | `https://soundcloud.com/ARTIST/TRACK`, including track secret-token paths |
| Bandcamp | `https://ARTIST.bandcamp.com/track/TRACK` |

Music short links, albums, playlists and artist pages are not expanded. Open the individual
track in a browser and copy its full web address. Spotify is intentionally unsupported;
the app does not search another service for a substitute recording.

## Output And Quality

Music links default to MP3 and offer audio output formats. Video links retain their existing
video and audio choices. Batch Crop & convert can select one time range for all media inputs.

The source helper is yt-dlp, with bundled FFmpeg/ffprobe for processing and validation.
Title, artist and album tags are requested where available; retention depends on the format.
Source streams may be lossy even when an output is WAV or FLAC. Re-encoding cannot restore detail.

Preview-only or DRM-only results are refused. Retrieved music must have a readable audio stream
and measurable duration. If it is substantially shorter than the source's reported track length,
the batch refuses it before creating a final output. This catches common preview/truncation cases,
but cannot prove completeness when a service supplies incorrect metadata.

Use only sources you own or have permission to convert. Account access and subscription level
do not grant permission to bypass a service's restrictions.

## Troubleshooting

- **Missing yt-dlp:** use Settings > Helpers. On Windows, install it using the displayed instructions.
- **Unsupported address:** copy the individual track's full web URL from the service, not an app URI.
- **Sign-in required:** Settings > Links can use a selected browser session or exported `cookies.txt`.
  A check launched with a failed link in the queue tests that link. Otherwise it tests the displayed
  reference URL. A successful check proves access only to that reference, not all services.
- **Safari access:** macOS may require Full Disk Access and an application restart. Other browsers
  or an exported cookie file are alternatives.
- **Preview/protected/unavailable track:** check the source page and your account's access.
  An authorized local file can be converted instead. The app does not remove DRM or switch sources.
- **Region or rate restriction:** retry when access is available; the app does not bypass it.
- **Bandcamp client challenge:** update yt-dlp. Some environments also require yt-dlp's optional
  request-impersonation dependencies; use the helper's installation documentation.
- **Output folder unavailable:** choose an existing custom folder in Settings > Output.

Source-service behavior is external to this repository. Unit tests use fixed fixtures; browser
tests use the mock backend. Opt-in live tests must not be treated as guarantees for every account,
region, source format or future API version.
