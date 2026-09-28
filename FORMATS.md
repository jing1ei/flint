# Supported formats

Generated from the format catalog in `crates/convert-core/src/format.rs` (`cargo run -p convert-core --bin format-report`). Do not edit by hand.

**✅ works out of the box** (bundled engine) · **⚙︎ needs a free helper app** (auto-detected; the app tells you the one-line install command) · **—** not supported in that direction.

**194 input file extensions · 91 output formats · 6 categories**

## Video

One-click default: **mp4** · quick picks: `mp4`, `webm`, `mov`, `gif`, `mp3`

| Format | Extensions | Read | Write | Notes |
| --- | --- | :---: | :---: | --- |
| MP4 (H.264/H.265/AV1) | `.mp4` | ✅ | ✅ | Best all-round web + demo format |
| iTunes Video | `.m4v` | ✅ | ✅ |  |
| QuickMovie / ProRes | `.mov` `.qt` | ✅ | ✅ | ProRes output for editing round-trips |
| Matroska | `.mkv` | ✅ | ✅ | Keeps multiple audio/subtitle tracks |
| WebM (VP9/AV1) | `.webm` | ✅ | ✅ | Smallest files for web pages |
| AVI | `.avi` | ✅ | ✅ | Legacy container; H.264 on output |
| Windows Media Video | `.wmv` | ✅ | ✅ | H.264 inside an ASF container |
| Flash Video | `.flv` | ✅ | ✅ | Flash era video, read + write |
| Flash MP4 Video | `.f4v` | ✅ | ✅ |  |
| MPEG-1/2 Program Stream | `.mpg` `.mpeg` `.m1v` `.m2v` `.vob` | ✅ | ✅ | DVD / VOB sources |
| MPEG Transport Stream | `.ts` `.m2ts` `.mts` `.m2t` | ✅ | ✅ | Camcorder / broadcast captures |
| 3GPP Mobile | `.3gp` `.3g2` | ✅ | ✅ | Old phone video |
| Ogg Theora | `.ogv` `.ogx` | ✅ | ✅ |  |
| Advanced Systems Format | `.asf` | ✅ | ✅ |  |
| RealMedia | `.rm` `.rmvb` | ✅ | — | Decode only |
| DivX / Xvid | `.divx` `.xvid` | ✅ | — | Decode only |
| DV / DVCPRO | `.dv` `.dif` | ✅ | — | Decode only; DV needs exact 720×480/576 frames |
| Material Exchange Format | `.mxf` | ✅ | ✅ | Broadcast masters |
| YUV4MPEG2 | `.y4m` | ✅ | ✅ | Raw intermediate |
| Raw H.264 / H.265 stream | `.h264` `.264` `.h265` `.265` `.hevc` | ✅ | ✅ | Elementary streams |
| NUT | `.nut` | ✅ | ✅ |  |
| Motion JPEG | `.mjpeg` `.mjpg` | ✅ | ✅ |  |

## Audio

One-click default: **mp3** · quick picks: `mp3`, `m4a`, `wav`, `flac`, `opus`

| Format | Extensions | Read | Write | Notes |
| --- | --- | :---: | :---: | --- |
| MIDI (piano) | `.mid` `.midi` | ✅ | — | Render notes with the built-in basic piano; type 0/1; audio output only |
| MP3 | `.mp3` | ✅ | ✅ | Universal audio default |
| AAC in MP4 (m4a) | `.m4a` `.m4b` `.m4r` | ✅ | ✅ | Best quality-per-byte for Apple devices |
| Raw AAC (ADTS) | `.aac` `.adts` | ✅ | ✅ |  |
| WAV (PCM) | `.wav` `.wave` | ✅ | ✅ | Lossless, uncompressed |
| FLAC | `.flac` | ✅ | ✅ | Lossless, compressed |
| Apple Lossless | `.m4a` | — | ✅ | Written into an .m4a container |
| Opus | `.opus` | ✅ | ✅ | Best small-size speech/music |
| Ogg Vorbis | `.ogg` `.oga` | ✅ | ✅ |  |
| AIFF / AIFC | `.aiff` `.aif` `.aifc` | ✅ | ✅ |  |
| Core Audio Format | `.caf` | ✅ | ✅ | macOS native |
| Windows Media Audio | `.wma` | ✅ | ✅ | Output uses wmav2 |
| Dolby Digital AC-3 | `.ac3` | ✅ | ✅ |  |
| Dolby Digital Plus | `.eac3` `.ec3` | ✅ | ✅ |  |
| DTS | `.dts` | ✅ | ✅ | Experimental encoder |
| Matroska Audio | `.mka` | ✅ | ✅ |  |
| AMR narrowband | `.amr` | ✅ | — | Decode only (8 kHz voice) |
| MPEG audio layer II | `.mp2` `.mpa` | ✅ | ✅ |  |
| Monkey's Audio | `.ape` | ✅ | — | Decode only |
| WavPack | `.wv` | ✅ | ✅ |  |
| True Audio | `.tta` | ✅ | ✅ |  |
| Sun/NeXT AU | `.au` `.snd` | ✅ | ✅ |  |
| Creative Voice | `.voc` | ✅ | ✅ |  |
| Sony Wave64 | `.w64` | ✅ | ✅ |  |
| GSM 06.10 | `.gsm` | ✅ | — | Decode only |
| Speex | `.spx` | ✅ | — | Decode only |
| RealAudio | `.ra` | ✅ | — | Decode only |
| Amiga 8SVX | `.8svx` `.iff` | ✅ | — | Decode only |

## Image

One-click default: **jpg** · quick picks: `jpg`, `png`, `webp`, `avif`, `pdf_page`

| Format | Extensions | Read | Write | Notes |
| --- | --- | :---: | :---: | --- |
| JPEG | `.jpg` `.jpeg` `.jpe` `.jfif` | ✅ | ✅ | Default photo output |
| PNG | `.png` | ✅ | ✅ | Lossless, transparency |
| WebP (still + animated) | `.webp` | ✅ | ✅ | ~30% smaller than JPEG |
| AVIF (still + animated) | `.avif` | ✅ | ✅ | Smallest modern web image |
| GIF (animated) | `.gif` | ✅ | ✅ | Palette-optimised output |
| Animated PNG | `.apng` | ✅ | ✅ |  |
| TIFF | `.tiff` `.tif` | ✅ | ✅ | Print / archival |
| Windows Bitmap | `.bmp` `.dib` | ✅ | ✅ |  |
| Windows Icon | `.ico` | ✅ | ✅ | Multi-size favicons |
| HEIC / HEIF | `.heic` `.heif` `.hif` | ⚙︎ macOS sips / ImageMagick | ⚙︎ macOS sips / ImageMagick | iPhone photos |
| Apple Icon Image | `.icns` | ⚙︎ macOS sips / ImageMagick | ⚙︎ macOS sips / ImageMagick | macOS app icons |
| SVG (vector) | `.svg` `.svgz` | ⚙︎ ImageMagick | — | Rasterised on input; no raster->vector |
| PDF page (as image) | `.pdf` | — | ⚙︎ ImageMagick | Image → PDF page (the PDF → image direction lives in Document) |
| Photoshop | `.psd` `.psb` | ⚙︎ ImageMagick | — | Flattened composite |
| Illustrator / EPS / PS | `.ai` `.eps` `.ps` | ⚙︎ ImageMagick | ⚙︎ ImageMagick |  |
| Truevision TGA | `.tga` `.icb` `.vda` `.vst` | ✅ | ✅ |  |
| Netpbm | `.ppm` `.pgm` `.pbm` `.pnm` `.pam` | ✅ | ✅ |  |
| PC Paintbrush | `.pcx` | ✅ | ✅ |  |
| DirectDraw Surface | `.dds` | ✅ | — | Game textures, decode only |
| OpenEXR (HDR) | `.exr` | ✅ | ✅ | VFX / HDR |
| Radiance HDR | `.hdr` `.pic` | ✅ | ✅ |  |
| JPEG 2000 | `.jp2` `.j2k` `.jpf` `.jpx` | ✅ | ✅ |  |
| X PixMap | `.xpm` | ✅ | — | Decode only |
| X BitMap | `.xbm` | ✅ | ✅ |  |
| DPX | `.dpx` | ✅ | ✅ | Film scans |
| SGI / RGB | `.sgi` `.rgb` `.rgba` `.bw` | ✅ | ✅ |  |
| Sun Raster | `.ras` `.sun` | ✅ | ✅ |  |
| Wireless Bitmap | `.wbmp` | ✅ | ✅ |  |
| QOI | `.qoi` | ✅ | ✅ |  |
| Camera RAW | `.cr2` `.cr3` `.crw` `.nef` `.nrw` `.arw` `.srf` `.sr2` `.dng` `.orf` `.rw2` `.raf` `.pef` `.srw` `.x3f` `.3fr` `.erf` `.mos` `.mrw` `.raw` | ⚙︎ macOS sips / ImageMagick | — | Decode only (Canon, Nikon, Sony, Fuji, Olympus, Panasonic, Pentax, Samsung, Sigma...) |

## Document

One-click default: **pdf** · quick picks: `pdf`, `docx`, `md`, `html`, `txt`

| Format | Extensions | Read | Write | Notes |
| --- | --- | :---: | :---: | --- |
| PDF | `.pdf` | ⚙︎ LibreOffice / Poppler (pdftoppm) / Poppler (pdftotext) / Poppler (pdftohtml) / ImageMagick / macOS sips | ⚙︎ LibreOffice | Universal document output |
| Word (docx) | `.docx` | ⚙︎ LibreOffice | ⚙︎ LibreOffice |  |
| Word 97-2003 | `.doc` | ⚙︎ LibreOffice | ⚙︎ LibreOffice |  |
| OpenDocument Text | `.odt` `.fodt` | ⚙︎ LibreOffice | ⚙︎ LibreOffice |  |
| Rich Text | `.rtf` | ⚙︎ LibreOffice | ⚙︎ LibreOffice |  |
| Plain text | `.txt` `.text` `.log` | ⚙︎ Pandoc / LibreOffice | ⚙︎ Pandoc / LibreOffice |  |
| Markdown | `.md` `.markdown` `.mdown` | ⚙︎ Pandoc | ⚙︎ Pandoc |  |
| HTML | `.html` `.htm` `.xhtml` | ⚙︎ Pandoc / LibreOffice | ⚙︎ Pandoc / LibreOffice |  |
| EPUB ebook | `.epub` | ⚙︎ Pandoc | ⚙︎ Pandoc |  |
| FictionBook | `.fb2` | ⚙︎ Pandoc | ⚙︎ Pandoc |  |
| LaTeX | `.tex` `.latex` | ⚙︎ Pandoc | ⚙︎ Pandoc |  |
| reStructuredText | `.rst` | ⚙︎ Pandoc | ⚙︎ Pandoc |  |
| PowerPoint (pptx) | `.pptx` | ⚙︎ LibreOffice | ⚙︎ LibreOffice |  |
| PowerPoint 97-2003 | `.ppt` | ⚙︎ LibreOffice | ⚙︎ LibreOffice |  |
| OpenDocument Presentation | `.odp` | ⚙︎ LibreOffice | ⚙︎ LibreOffice |  |
| Excel (xlsx) | `.xlsx` | ⚙︎ LibreOffice | ⚙︎ LibreOffice |  |
| Excel 97-2003 | `.xls` | ⚙︎ LibreOffice | ⚙︎ LibreOffice |  |
| OpenDocument Spreadsheet | `.ods` `.fods` | ⚙︎ LibreOffice | ⚙︎ LibreOffice |  |
| CSV | `.csv` | ⚙︎ LibreOffice | ⚙︎ LibreOffice |  |
| TSV | `.tsv` `.tab` | ⚙︎ LibreOffice | ⚙︎ LibreOffice |  |
| JSON / YAML data | `.json` `.yaml` `.yml` | ⚙︎ Pandoc | ⚙︎ Pandoc | Structured text via Pandoc |

## Subtitle

One-click default: **srt** · quick picks: `srt`, `vtt`, `ass`

| Format | Extensions | Read | Write | Notes |
| --- | --- | :---: | :---: | --- |
| SubRip | `.srt` | ✅ | ✅ | Most widely supported |
| WebVTT | `.vtt` | ✅ | ✅ | HTML5 <track> |
| Advanced SubStation | `.ass` `.ssa` | ✅ | ✅ | Styled subtitles |
| MicroDVD / SubViewer | `.sub` `.sbv` | ✅ | — | Read only |
| LRC lyrics | `.lrc` | ✅ | ✅ |  |
| TTML / DFXP | `.ttml` `.dfxp` | — | ✅ | Write only |

## Flash

One-click default: **mp4** · quick picks: `mp4`, `gif`, `png`

| Format | Extensions | Read | Write | Notes |
| --- | --- | :---: | :---: | --- |
| Flash movie (SWF) | `.swf` | ⚙︎ Ruffle | — | Rendered by Ruffle, then encoded to video/GIF |

## Cross-category conversions

| From | To | How |
| --- | --- | --- |
| Video | Audio | stream extracted and re-encoded (`clip.mp4` → `clip.mp3`) |
| Video / Flash | GIF, animated WebP, APNG | frame-rate + palette optimised |
| Video | JPEG, PNG, WebP, … | frame sequence, 1 frame per second by default |
| Flash | PNG | the frames Ruffle renders, one file per frame |
| Animated GIF / WebP | MP4, WebM | real video, constant frame rate |
| Images | HEIC, ICNS, AI | written by the helper that owns the format (sips / ImageMagick) |
| Images | PDF | one page per image (ImageMagick) |
| PDF | PNG, JPEG | one image per page (Poppler → ImageMagick → sips) |
| Office / Slides | PNG, JPEG | printed to PDF first, then rasterised |
| Markdown / HTML / EPUB | PDF | Pandoc → HTML → LibreOffice print |
| Video | SRT, VTT, ASS | embedded subtitle track demuxed |

