// Generated from crates/convert-core/src/format.rs — do not edit by hand.
// Data only: consumed by src/lib/mock.ts to make the browser preview behave like the real app.
import type { CatalogView } from "./types";

export const MOCK_CATALOG: CatalogView = {
  "categories": [
    {
      "id": "video",
      "label": "Video",
      "default_target": "mp4",
      "suggested_targets": [
        "mp4",
        "webm",
        "mov",
        "gif",
        "mp3"
      ],
      "inputs": [
        {
          "id": "mp4",
          "name": "MP4 (H.264/H.265/AV1)",
          "extension": "mp4",
          "extensions": [
            "mp4"
          ],
          "notes": "Best all-round web + demo format",
          "available": true,
          "needs": []
        },
        {
          "id": "m4v",
          "name": "iTunes Video",
          "extension": "m4v",
          "extensions": [
            "m4v"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "mov",
          "name": "QuickMovie / ProRes",
          "extension": "mov",
          "extensions": [
            "mov",
            "qt"
          ],
          "notes": "ProRes output for editing round-trips",
          "available": true,
          "needs": []
        },
        {
          "id": "mkv",
          "name": "Matroska",
          "extension": "mkv",
          "extensions": [
            "mkv"
          ],
          "notes": "Keeps multiple audio/subtitle tracks",
          "available": true,
          "needs": []
        },
        {
          "id": "webm",
          "name": "WebM (VP9/AV1)",
          "extension": "webm",
          "extensions": [
            "webm"
          ],
          "notes": "Smallest files for web pages",
          "available": true,
          "needs": []
        },
        {
          "id": "avi",
          "name": "AVI",
          "extension": "avi",
          "extensions": [
            "avi"
          ],
          "notes": "Legacy container; H.264 on output",
          "available": true,
          "needs": []
        },
        {
          "id": "wmv",
          "name": "Windows Media Video",
          "extension": "wmv",
          "extensions": [
            "wmv"
          ],
          "notes": "H.264 inside an ASF container",
          "available": true,
          "needs": []
        },
        {
          "id": "flv",
          "name": "Flash Video",
          "extension": "flv",
          "extensions": [
            "flv"
          ],
          "notes": "Flash era video, read + write",
          "available": true,
          "needs": []
        },
        {
          "id": "f4v",
          "name": "Flash MP4 Video",
          "extension": "f4v",
          "extensions": [
            "f4v"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "mpg",
          "name": "MPEG-1/2 Program Stream",
          "extension": "mpg",
          "extensions": [
            "mpg",
            "mpeg",
            "m1v",
            "m2v",
            "vob"
          ],
          "notes": "DVD / VOB sources",
          "available": true,
          "needs": []
        },
        {
          "id": "ts",
          "name": "MPEG Transport Stream",
          "extension": "ts",
          "extensions": [
            "ts",
            "m2ts",
            "mts",
            "m2t"
          ],
          "notes": "Camcorder / broadcast captures",
          "available": true,
          "needs": []
        },
        {
          "id": "3gp",
          "name": "3GPP Mobile",
          "extension": "3gp",
          "extensions": [
            "3gp",
            "3g2"
          ],
          "notes": "Old phone video",
          "available": true,
          "needs": []
        },
        {
          "id": "ogv",
          "name": "Ogg Theora",
          "extension": "ogv",
          "extensions": [
            "ogv",
            "ogx"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "asf",
          "name": "Advanced Systems Format",
          "extension": "asf",
          "extensions": [
            "asf"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "rm",
          "name": "RealMedia",
          "extension": "rm",
          "extensions": [
            "rm",
            "rmvb"
          ],
          "notes": "Decode only",
          "available": true,
          "needs": []
        },
        {
          "id": "divx",
          "name": "DivX / Xvid",
          "extension": "divx",
          "extensions": [
            "divx",
            "xvid"
          ],
          "notes": "Decode only",
          "available": true,
          "needs": []
        },
        {
          "id": "dv",
          "name": "DV / DVCPRO",
          "extension": "dv",
          "extensions": [
            "dv",
            "dif"
          ],
          "notes": "Decode only; DV needs exact 720×480/576 frames",
          "available": true,
          "needs": []
        },
        {
          "id": "mxf",
          "name": "Material Exchange Format",
          "extension": "mxf",
          "extensions": [
            "mxf"
          ],
          "notes": "Broadcast masters",
          "available": true,
          "needs": []
        },
        {
          "id": "y4m",
          "name": "YUV4MPEG2",
          "extension": "y4m",
          "extensions": [
            "y4m"
          ],
          "notes": "Raw intermediate",
          "available": true,
          "needs": []
        },
        {
          "id": "h264",
          "name": "Raw H.264 / H.265 stream",
          "extension": "h264",
          "extensions": [
            "h264",
            "264",
            "h265",
            "265",
            "hevc"
          ],
          "notes": "Elementary streams",
          "available": true,
          "needs": []
        },
        {
          "id": "nut",
          "name": "NUT",
          "extension": "nut",
          "extensions": [
            "nut"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "mjpeg",
          "name": "Motion JPEG",
          "extension": "mjpeg",
          "extensions": [
            "mjpeg",
            "mjpg"
          ],
          "notes": "",
          "available": true,
          "needs": []
        }
      ],
      "outputs": [
        {
          "id": "mp4",
          "name": "MP4 (H.264/H.265/AV1)",
          "extension": "mp4",
          "extensions": [
            "mp4"
          ],
          "notes": "Best all-round web + demo format",
          "available": true,
          "needs": []
        },
        {
          "id": "m4v",
          "name": "iTunes Video",
          "extension": "m4v",
          "extensions": [
            "m4v"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "mov",
          "name": "QuickMovie / ProRes",
          "extension": "mov",
          "extensions": [
            "mov",
            "qt"
          ],
          "notes": "ProRes output for editing round-trips",
          "available": true,
          "needs": []
        },
        {
          "id": "mkv",
          "name": "Matroska",
          "extension": "mkv",
          "extensions": [
            "mkv"
          ],
          "notes": "Keeps multiple audio/subtitle tracks",
          "available": true,
          "needs": []
        },
        {
          "id": "webm",
          "name": "WebM (VP9/AV1)",
          "extension": "webm",
          "extensions": [
            "webm"
          ],
          "notes": "Smallest files for web pages",
          "available": true,
          "needs": []
        },
        {
          "id": "avi",
          "name": "AVI",
          "extension": "avi",
          "extensions": [
            "avi"
          ],
          "notes": "Legacy container; H.264 on output",
          "available": true,
          "needs": []
        },
        {
          "id": "wmv",
          "name": "Windows Media Video",
          "extension": "wmv",
          "extensions": [
            "wmv"
          ],
          "notes": "H.264 inside an ASF container",
          "available": true,
          "needs": []
        },
        {
          "id": "flv",
          "name": "Flash Video",
          "extension": "flv",
          "extensions": [
            "flv"
          ],
          "notes": "Flash era video, read + write",
          "available": true,
          "needs": []
        },
        {
          "id": "f4v",
          "name": "Flash MP4 Video",
          "extension": "f4v",
          "extensions": [
            "f4v"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "mpg",
          "name": "MPEG-1/2 Program Stream",
          "extension": "mpg",
          "extensions": [
            "mpg",
            "mpeg",
            "m1v",
            "m2v",
            "vob"
          ],
          "notes": "DVD / VOB sources",
          "available": true,
          "needs": []
        },
        {
          "id": "ts",
          "name": "MPEG Transport Stream",
          "extension": "ts",
          "extensions": [
            "ts",
            "m2ts",
            "mts",
            "m2t"
          ],
          "notes": "Camcorder / broadcast captures",
          "available": true,
          "needs": []
        },
        {
          "id": "3gp",
          "name": "3GPP Mobile",
          "extension": "3gp",
          "extensions": [
            "3gp",
            "3g2"
          ],
          "notes": "Old phone video",
          "available": true,
          "needs": []
        },
        {
          "id": "ogv",
          "name": "Ogg Theora",
          "extension": "ogv",
          "extensions": [
            "ogv",
            "ogx"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "asf",
          "name": "Advanced Systems Format",
          "extension": "asf",
          "extensions": [
            "asf"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "mxf",
          "name": "Material Exchange Format",
          "extension": "mxf",
          "extensions": [
            "mxf"
          ],
          "notes": "Broadcast masters",
          "available": true,
          "needs": []
        },
        {
          "id": "y4m",
          "name": "YUV4MPEG2",
          "extension": "y4m",
          "extensions": [
            "y4m"
          ],
          "notes": "Raw intermediate",
          "available": true,
          "needs": []
        },
        {
          "id": "h264",
          "name": "Raw H.264 / H.265 stream",
          "extension": "h264",
          "extensions": [
            "h264",
            "264",
            "h265",
            "265",
            "hevc"
          ],
          "notes": "Elementary streams",
          "available": true,
          "needs": []
        },
        {
          "id": "nut",
          "name": "NUT",
          "extension": "nut",
          "extensions": [
            "nut"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "mjpeg",
          "name": "Motion JPEG",
          "extension": "mjpeg",
          "extensions": [
            "mjpeg",
            "mjpg"
          ],
          "notes": "",
          "available": true,
          "needs": []
        }
      ]
    },
    {
      "id": "audio",
      "label": "Audio",
      "default_target": "mp3",
      "suggested_targets": [
        "mp3",
        "m4a",
        "wav",
        "flac",
        "opus"
      ],
      "inputs": [
        {
          "id": "midi",
          "name": "MIDI (piano)",
          "extension": "mid",
          "extensions": [
            "mid",
            "midi"
          ],
          "notes": "Render notes with the built-in basic piano; type 0/1; audio output only",
          "available": true,
          "needs": []
        },
        {
          "id": "mp3",
          "name": "MP3",
          "extension": "mp3",
          "extensions": [
            "mp3"
          ],
          "notes": "Universal audio default",
          "available": true,
          "needs": []
        },
        {
          "id": "m4a",
          "name": "AAC in MP4 (m4a)",
          "extension": "m4a",
          "extensions": [
            "m4a",
            "m4b",
            "m4r"
          ],
          "notes": "Best quality-per-byte for Apple devices",
          "available": true,
          "needs": []
        },
        {
          "id": "aac",
          "name": "Raw AAC (ADTS)",
          "extension": "aac",
          "extensions": [
            "aac",
            "adts"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "wav",
          "name": "WAV (PCM)",
          "extension": "wav",
          "extensions": [
            "wav",
            "wave"
          ],
          "notes": "Lossless, uncompressed",
          "available": true,
          "needs": []
        },
        {
          "id": "flac",
          "name": "FLAC",
          "extension": "flac",
          "extensions": [
            "flac"
          ],
          "notes": "Lossless, compressed",
          "available": true,
          "needs": []
        },
        {
          "id": "opus",
          "name": "Opus",
          "extension": "opus",
          "extensions": [
            "opus"
          ],
          "notes": "Best small-size speech/music",
          "available": true,
          "needs": []
        },
        {
          "id": "ogg",
          "name": "Ogg Vorbis",
          "extension": "ogg",
          "extensions": [
            "ogg",
            "oga"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "aiff",
          "name": "AIFF / AIFC",
          "extension": "aiff",
          "extensions": [
            "aiff",
            "aif",
            "aifc"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "caf",
          "name": "Core Audio Format",
          "extension": "caf",
          "extensions": [
            "caf"
          ],
          "notes": "macOS native",
          "available": true,
          "needs": []
        },
        {
          "id": "wma",
          "name": "Windows Media Audio",
          "extension": "wma",
          "extensions": [
            "wma"
          ],
          "notes": "Output uses wmav2",
          "available": true,
          "needs": []
        },
        {
          "id": "ac3",
          "name": "Dolby Digital AC-3",
          "extension": "ac3",
          "extensions": [
            "ac3"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "eac3",
          "name": "Dolby Digital Plus",
          "extension": "eac3",
          "extensions": [
            "eac3",
            "ec3"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "dts",
          "name": "DTS",
          "extension": "dts",
          "extensions": [
            "dts"
          ],
          "notes": "Experimental encoder",
          "available": true,
          "needs": []
        },
        {
          "id": "mka",
          "name": "Matroska Audio",
          "extension": "mka",
          "extensions": [
            "mka"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "amr",
          "name": "AMR narrowband",
          "extension": "amr",
          "extensions": [
            "amr"
          ],
          "notes": "Decode only (8 kHz voice)",
          "available": true,
          "needs": []
        },
        {
          "id": "mp2",
          "name": "MPEG audio layer II",
          "extension": "mp2",
          "extensions": [
            "mp2",
            "mpa"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "ape",
          "name": "Monkey's Audio",
          "extension": "ape",
          "extensions": [
            "ape"
          ],
          "notes": "Decode only",
          "available": true,
          "needs": []
        },
        {
          "id": "wv",
          "name": "WavPack",
          "extension": "wv",
          "extensions": [
            "wv"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "tta",
          "name": "True Audio",
          "extension": "tta",
          "extensions": [
            "tta"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "au",
          "name": "Sun/NeXT AU",
          "extension": "au",
          "extensions": [
            "au",
            "snd"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "voc",
          "name": "Creative Voice",
          "extension": "voc",
          "extensions": [
            "voc"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "w64",
          "name": "Sony Wave64",
          "extension": "w64",
          "extensions": [
            "w64"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "gsm",
          "name": "GSM 06.10",
          "extension": "gsm",
          "extensions": [
            "gsm"
          ],
          "notes": "Decode only",
          "available": true,
          "needs": []
        },
        {
          "id": "spx",
          "name": "Speex",
          "extension": "spx",
          "extensions": [
            "spx"
          ],
          "notes": "Decode only",
          "available": true,
          "needs": []
        },
        {
          "id": "ra",
          "name": "RealAudio",
          "extension": "ra",
          "extensions": [
            "ra"
          ],
          "notes": "Decode only",
          "available": true,
          "needs": []
        },
        {
          "id": "8svx",
          "name": "Amiga 8SVX",
          "extension": "8svx",
          "extensions": [
            "8svx",
            "iff"
          ],
          "notes": "Decode only",
          "available": true,
          "needs": []
        }
      ],
      "outputs": [
        {
          "id": "mp3",
          "name": "MP3",
          "extension": "mp3",
          "extensions": [
            "mp3"
          ],
          "notes": "Universal audio default",
          "available": true,
          "needs": []
        },
        {
          "id": "m4a",
          "name": "AAC in MP4 (m4a)",
          "extension": "m4a",
          "extensions": [
            "m4a",
            "m4b",
            "m4r"
          ],
          "notes": "Best quality-per-byte for Apple devices",
          "available": true,
          "needs": []
        },
        {
          "id": "aac",
          "name": "Raw AAC (ADTS)",
          "extension": "aac",
          "extensions": [
            "aac",
            "adts"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "wav",
          "name": "WAV (PCM)",
          "extension": "wav",
          "extensions": [
            "wav",
            "wave"
          ],
          "notes": "Lossless, uncompressed",
          "available": true,
          "needs": []
        },
        {
          "id": "flac",
          "name": "FLAC",
          "extension": "flac",
          "extensions": [
            "flac"
          ],
          "notes": "Lossless, compressed",
          "available": true,
          "needs": []
        },
        {
          "id": "alac",
          "name": "Apple Lossless",
          "extension": "m4a",
          "extensions": [
            "m4a"
          ],
          "notes": "Written into an .m4a container",
          "available": true,
          "needs": []
        },
        {
          "id": "opus",
          "name": "Opus",
          "extension": "opus",
          "extensions": [
            "opus"
          ],
          "notes": "Best small-size speech/music",
          "available": true,
          "needs": []
        },
        {
          "id": "ogg",
          "name": "Ogg Vorbis",
          "extension": "ogg",
          "extensions": [
            "ogg",
            "oga"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "aiff",
          "name": "AIFF / AIFC",
          "extension": "aiff",
          "extensions": [
            "aiff",
            "aif",
            "aifc"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "caf",
          "name": "Core Audio Format",
          "extension": "caf",
          "extensions": [
            "caf"
          ],
          "notes": "macOS native",
          "available": true,
          "needs": []
        },
        {
          "id": "wma",
          "name": "Windows Media Audio",
          "extension": "wma",
          "extensions": [
            "wma"
          ],
          "notes": "Output uses wmav2",
          "available": true,
          "needs": []
        },
        {
          "id": "ac3",
          "name": "Dolby Digital AC-3",
          "extension": "ac3",
          "extensions": [
            "ac3"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "eac3",
          "name": "Dolby Digital Plus",
          "extension": "eac3",
          "extensions": [
            "eac3",
            "ec3"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "dts",
          "name": "DTS",
          "extension": "dts",
          "extensions": [
            "dts"
          ],
          "notes": "Experimental encoder",
          "available": true,
          "needs": []
        },
        {
          "id": "mka",
          "name": "Matroska Audio",
          "extension": "mka",
          "extensions": [
            "mka"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "mp2",
          "name": "MPEG audio layer II",
          "extension": "mp2",
          "extensions": [
            "mp2",
            "mpa"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "wv",
          "name": "WavPack",
          "extension": "wv",
          "extensions": [
            "wv"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "tta",
          "name": "True Audio",
          "extension": "tta",
          "extensions": [
            "tta"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "au",
          "name": "Sun/NeXT AU",
          "extension": "au",
          "extensions": [
            "au",
            "snd"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "voc",
          "name": "Creative Voice",
          "extension": "voc",
          "extensions": [
            "voc"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "w64",
          "name": "Sony Wave64",
          "extension": "w64",
          "extensions": [
            "w64"
          ],
          "notes": "",
          "available": true,
          "needs": []
        }
      ]
    },
    {
      "id": "image",
      "label": "Image",
      "default_target": "jpg",
      "suggested_targets": [
        "jpg",
        "png",
        "webp",
        "avif",
        "pdf_page"
      ],
      "inputs": [
        {
          "id": "jpg",
          "name": "JPEG",
          "extension": "jpg",
          "extensions": [
            "jpg",
            "jpeg",
            "jpe",
            "jfif"
          ],
          "notes": "Default photo output",
          "available": true,
          "needs": []
        },
        {
          "id": "png",
          "name": "PNG",
          "extension": "png",
          "extensions": [
            "png"
          ],
          "notes": "Lossless, transparency",
          "available": true,
          "needs": []
        },
        {
          "id": "webp",
          "name": "WebP (still + animated)",
          "extension": "webp",
          "extensions": [
            "webp"
          ],
          "notes": "~30% smaller than JPEG",
          "available": true,
          "needs": []
        },
        {
          "id": "avif",
          "name": "AVIF (still + animated)",
          "extension": "avif",
          "extensions": [
            "avif"
          ],
          "notes": "Smallest modern web image",
          "available": true,
          "needs": []
        },
        {
          "id": "gif",
          "name": "GIF (animated)",
          "extension": "gif",
          "extensions": [
            "gif"
          ],
          "notes": "Palette-optimised output",
          "available": true,
          "needs": []
        },
        {
          "id": "apng",
          "name": "Animated PNG",
          "extension": "png",
          "extensions": [
            "apng"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "tiff",
          "name": "TIFF",
          "extension": "tiff",
          "extensions": [
            "tiff",
            "tif"
          ],
          "notes": "Print / archival",
          "available": true,
          "needs": []
        },
        {
          "id": "bmp",
          "name": "Windows Bitmap",
          "extension": "bmp",
          "extensions": [
            "bmp",
            "dib"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "ico",
          "name": "Windows Icon",
          "extension": "ico",
          "extensions": [
            "ico"
          ],
          "notes": "Multi-size favicons",
          "available": true,
          "needs": []
        },
        {
          "id": "heic",
          "name": "HEIC / HEIF",
          "extension": "heic",
          "extensions": [
            "heic",
            "heif",
            "hif"
          ],
          "notes": "iPhone photos",
          "available": true,
          "needs": [
            "macOS sips",
            "ImageMagick"
          ]
        },
        {
          "id": "icns",
          "name": "Apple Icon Image",
          "extension": "icns",
          "extensions": [
            "icns"
          ],
          "notes": "macOS app icons",
          "available": true,
          "needs": [
            "macOS sips",
            "ImageMagick"
          ]
        },
        {
          "id": "svg",
          "name": "SVG (vector)",
          "extension": "svg",
          "extensions": [
            "svg",
            "svgz"
          ],
          "notes": "Rasterised on input; no raster->vector",
          "available": true,
          "needs": [
            "ImageMagick"
          ]
        },
        {
          "id": "psd",
          "name": "Photoshop",
          "extension": "psd",
          "extensions": [
            "psd",
            "psb"
          ],
          "notes": "Flattened composite",
          "available": true,
          "needs": [
            "ImageMagick"
          ]
        },
        {
          "id": "ai",
          "name": "Illustrator / EPS / PS",
          "extension": "ai",
          "extensions": [
            "ai",
            "eps",
            "ps"
          ],
          "notes": "",
          "available": true,
          "needs": [
            "ImageMagick"
          ]
        },
        {
          "id": "tga",
          "name": "Truevision TGA",
          "extension": "tga",
          "extensions": [
            "tga",
            "icb",
            "vda",
            "vst"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "ppm",
          "name": "Netpbm",
          "extension": "ppm",
          "extensions": [
            "ppm",
            "pgm",
            "pbm",
            "pnm",
            "pam"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "pcx",
          "name": "PC Paintbrush",
          "extension": "pcx",
          "extensions": [
            "pcx"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "dds",
          "name": "DirectDraw Surface",
          "extension": "dds",
          "extensions": [
            "dds"
          ],
          "notes": "Game textures, decode only",
          "available": true,
          "needs": []
        },
        {
          "id": "exr",
          "name": "OpenEXR (HDR)",
          "extension": "exr",
          "extensions": [
            "exr"
          ],
          "notes": "VFX / HDR",
          "available": true,
          "needs": []
        },
        {
          "id": "hdr",
          "name": "Radiance HDR",
          "extension": "hdr",
          "extensions": [
            "hdr",
            "pic"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "jp2",
          "name": "JPEG 2000",
          "extension": "jp2",
          "extensions": [
            "jp2",
            "j2k",
            "jpf",
            "jpx"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "xpm",
          "name": "X PixMap",
          "extension": "xpm",
          "extensions": [
            "xpm"
          ],
          "notes": "Decode only",
          "available": true,
          "needs": []
        },
        {
          "id": "xbm",
          "name": "X BitMap",
          "extension": "xbm",
          "extensions": [
            "xbm"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "dpx",
          "name": "DPX",
          "extension": "dpx",
          "extensions": [
            "dpx"
          ],
          "notes": "Film scans",
          "available": true,
          "needs": []
        },
        {
          "id": "sgi",
          "name": "SGI / RGB",
          "extension": "sgi",
          "extensions": [
            "sgi",
            "rgb",
            "rgba",
            "bw"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "sun",
          "name": "Sun Raster",
          "extension": "ras",
          "extensions": [
            "ras",
            "sun"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "wbmp",
          "name": "Wireless Bitmap",
          "extension": "wbmp",
          "extensions": [
            "wbmp"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "qoi",
          "name": "QOI",
          "extension": "qoi",
          "extensions": [
            "qoi"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "raw_camera",
          "name": "Camera RAW",
          "extension": "cr2",
          "extensions": [
            "cr2",
            "cr3",
            "crw",
            "nef",
            "nrw",
            "arw",
            "srf",
            "sr2",
            "dng",
            "orf",
            "rw2",
            "raf",
            "pef",
            "srw",
            "x3f",
            "3fr",
            "erf",
            "mos",
            "mrw",
            "raw"
          ],
          "notes": "Decode only (Canon, Nikon, Sony, Fuji, Olympus, Panasonic, Pentax, Samsung, Sigma...)",
          "available": true,
          "needs": [
            "macOS sips",
            "ImageMagick"
          ]
        }
      ],
      "outputs": [
        {
          "id": "jpg",
          "name": "JPEG",
          "extension": "jpg",
          "extensions": [
            "jpg",
            "jpeg",
            "jpe",
            "jfif"
          ],
          "notes": "Default photo output",
          "available": true,
          "needs": []
        },
        {
          "id": "png",
          "name": "PNG",
          "extension": "png",
          "extensions": [
            "png"
          ],
          "notes": "Lossless, transparency",
          "available": true,
          "needs": []
        },
        {
          "id": "webp",
          "name": "WebP (still + animated)",
          "extension": "webp",
          "extensions": [
            "webp"
          ],
          "notes": "~30% smaller than JPEG",
          "available": true,
          "needs": []
        },
        {
          "id": "avif",
          "name": "AVIF (still + animated)",
          "extension": "avif",
          "extensions": [
            "avif"
          ],
          "notes": "Smallest modern web image",
          "available": true,
          "needs": []
        },
        {
          "id": "gif",
          "name": "GIF (animated)",
          "extension": "gif",
          "extensions": [
            "gif"
          ],
          "notes": "Palette-optimised output",
          "available": true,
          "needs": []
        },
        {
          "id": "apng",
          "name": "Animated PNG",
          "extension": "png",
          "extensions": [
            "apng"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "tiff",
          "name": "TIFF",
          "extension": "tiff",
          "extensions": [
            "tiff",
            "tif"
          ],
          "notes": "Print / archival",
          "available": true,
          "needs": []
        },
        {
          "id": "bmp",
          "name": "Windows Bitmap",
          "extension": "bmp",
          "extensions": [
            "bmp",
            "dib"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "ico",
          "name": "Windows Icon",
          "extension": "ico",
          "extensions": [
            "ico"
          ],
          "notes": "Multi-size favicons",
          "available": true,
          "needs": []
        },
        {
          "id": "heic",
          "name": "HEIC / HEIF",
          "extension": "heic",
          "extensions": [
            "heic",
            "heif",
            "hif"
          ],
          "notes": "iPhone photos",
          "available": true,
          "needs": [
            "macOS sips",
            "ImageMagick"
          ]
        },
        {
          "id": "icns",
          "name": "Apple Icon Image",
          "extension": "icns",
          "extensions": [
            "icns"
          ],
          "notes": "macOS app icons",
          "available": true,
          "needs": [
            "macOS sips",
            "ImageMagick"
          ]
        },
        {
          "id": "pdf_page",
          "name": "PDF page (as image)",
          "extension": "pdf",
          "extensions": [
            "pdf"
          ],
          "notes": "Image → PDF page (the PDF → image direction lives in Document)",
          "available": true,
          "needs": [
            "ImageMagick"
          ]
        },
        {
          "id": "ai",
          "name": "Illustrator / EPS / PS",
          "extension": "ai",
          "extensions": [
            "ai",
            "eps",
            "ps"
          ],
          "notes": "",
          "available": true,
          "needs": [
            "ImageMagick"
          ]
        },
        {
          "id": "tga",
          "name": "Truevision TGA",
          "extension": "tga",
          "extensions": [
            "tga",
            "icb",
            "vda",
            "vst"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "ppm",
          "name": "Netpbm",
          "extension": "ppm",
          "extensions": [
            "ppm",
            "pgm",
            "pbm",
            "pnm",
            "pam"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "pcx",
          "name": "PC Paintbrush",
          "extension": "pcx",
          "extensions": [
            "pcx"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "exr",
          "name": "OpenEXR (HDR)",
          "extension": "exr",
          "extensions": [
            "exr"
          ],
          "notes": "VFX / HDR",
          "available": true,
          "needs": []
        },
        {
          "id": "hdr",
          "name": "Radiance HDR",
          "extension": "hdr",
          "extensions": [
            "hdr",
            "pic"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "jp2",
          "name": "JPEG 2000",
          "extension": "jp2",
          "extensions": [
            "jp2",
            "j2k",
            "jpf",
            "jpx"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "xbm",
          "name": "X BitMap",
          "extension": "xbm",
          "extensions": [
            "xbm"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "dpx",
          "name": "DPX",
          "extension": "dpx",
          "extensions": [
            "dpx"
          ],
          "notes": "Film scans",
          "available": true,
          "needs": []
        },
        {
          "id": "sgi",
          "name": "SGI / RGB",
          "extension": "sgi",
          "extensions": [
            "sgi",
            "rgb",
            "rgba",
            "bw"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "sun",
          "name": "Sun Raster",
          "extension": "ras",
          "extensions": [
            "ras",
            "sun"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "wbmp",
          "name": "Wireless Bitmap",
          "extension": "wbmp",
          "extensions": [
            "wbmp"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "qoi",
          "name": "QOI",
          "extension": "qoi",
          "extensions": [
            "qoi"
          ],
          "notes": "",
          "available": true,
          "needs": []
        }
      ]
    },
    {
      "id": "document",
      "label": "Document",
      "default_target": "pdf",
      "suggested_targets": [
        "pdf",
        "docx",
        "md",
        "html",
        "txt"
      ],
      "inputs": [
        {
          "id": "pdf",
          "name": "PDF",
          "extension": "pdf",
          "extensions": [
            "pdf"
          ],
          "notes": "Universal document output",
          "available": true,
          "needs": [
            "LibreOffice",
            "Poppler",
            "ImageMagick",
            "macOS sips"
          ]
        },
        {
          "id": "docx",
          "name": "Word (docx)",
          "extension": "docx",
          "extensions": [
            "docx"
          ],
          "notes": "",
          "available": true,
          "needs": [
            "LibreOffice"
          ]
        },
        {
          "id": "doc",
          "name": "Word 97-2003",
          "extension": "doc",
          "extensions": [
            "doc"
          ],
          "notes": "",
          "available": true,
          "needs": [
            "LibreOffice"
          ]
        },
        {
          "id": "odt",
          "name": "OpenDocument Text",
          "extension": "odt",
          "extensions": [
            "odt",
            "fodt"
          ],
          "notes": "",
          "available": true,
          "needs": [
            "LibreOffice"
          ]
        },
        {
          "id": "rtf",
          "name": "Rich Text",
          "extension": "rtf",
          "extensions": [
            "rtf"
          ],
          "notes": "",
          "available": true,
          "needs": [
            "LibreOffice"
          ]
        },
        {
          "id": "txt",
          "name": "Plain text",
          "extension": "txt",
          "extensions": [
            "txt",
            "text",
            "log"
          ],
          "notes": "",
          "available": true,
          "needs": [
            "Pandoc",
            "LibreOffice"
          ]
        },
        {
          "id": "md",
          "name": "Markdown",
          "extension": "md",
          "extensions": [
            "md",
            "markdown",
            "mdown"
          ],
          "notes": "",
          "available": false,
          "needs": [
            "Pandoc"
          ]
        },
        {
          "id": "html",
          "name": "HTML",
          "extension": "html",
          "extensions": [
            "html",
            "htm",
            "xhtml"
          ],
          "notes": "",
          "available": true,
          "needs": [
            "Pandoc",
            "LibreOffice"
          ]
        },
        {
          "id": "epub",
          "name": "EPUB ebook",
          "extension": "epub",
          "extensions": [
            "epub"
          ],
          "notes": "",
          "available": false,
          "needs": [
            "Pandoc"
          ]
        },
        {
          "id": "fb2",
          "name": "FictionBook",
          "extension": "fb2",
          "extensions": [
            "fb2"
          ],
          "notes": "",
          "available": false,
          "needs": [
            "Pandoc"
          ]
        },
        {
          "id": "tex",
          "name": "LaTeX",
          "extension": "tex",
          "extensions": [
            "tex",
            "latex"
          ],
          "notes": "",
          "available": false,
          "needs": [
            "Pandoc"
          ]
        },
        {
          "id": "rst",
          "name": "reStructuredText",
          "extension": "rst",
          "extensions": [
            "rst"
          ],
          "notes": "",
          "available": false,
          "needs": [
            "Pandoc"
          ]
        },
        {
          "id": "pptx",
          "name": "PowerPoint (pptx)",
          "extension": "pptx",
          "extensions": [
            "pptx"
          ],
          "notes": "",
          "available": true,
          "needs": [
            "LibreOffice"
          ]
        },
        {
          "id": "ppt",
          "name": "PowerPoint 97-2003",
          "extension": "ppt",
          "extensions": [
            "ppt"
          ],
          "notes": "",
          "available": true,
          "needs": [
            "LibreOffice"
          ]
        },
        {
          "id": "odp",
          "name": "OpenDocument Presentation",
          "extension": "odp",
          "extensions": [
            "odp"
          ],
          "notes": "",
          "available": true,
          "needs": [
            "LibreOffice"
          ]
        },
        {
          "id": "xlsx",
          "name": "Excel (xlsx)",
          "extension": "xlsx",
          "extensions": [
            "xlsx"
          ],
          "notes": "",
          "available": true,
          "needs": [
            "LibreOffice"
          ]
        },
        {
          "id": "xls",
          "name": "Excel 97-2003",
          "extension": "xls",
          "extensions": [
            "xls"
          ],
          "notes": "",
          "available": true,
          "needs": [
            "LibreOffice"
          ]
        },
        {
          "id": "ods",
          "name": "OpenDocument Spreadsheet",
          "extension": "ods",
          "extensions": [
            "ods",
            "fods"
          ],
          "notes": "",
          "available": true,
          "needs": [
            "LibreOffice"
          ]
        },
        {
          "id": "csv",
          "name": "CSV",
          "extension": "csv",
          "extensions": [
            "csv"
          ],
          "notes": "",
          "available": true,
          "needs": [
            "LibreOffice"
          ]
        },
        {
          "id": "tsv",
          "name": "TSV",
          "extension": "tsv",
          "extensions": [
            "tsv",
            "tab"
          ],
          "notes": "",
          "available": true,
          "needs": [
            "LibreOffice"
          ]
        },
        {
          "id": "json_doc",
          "name": "JSON / YAML data",
          "extension": "json",
          "extensions": [
            "json",
            "yaml",
            "yml"
          ],
          "notes": "Structured text via Pandoc",
          "available": false,
          "needs": [
            "Pandoc"
          ]
        }
      ],
      "outputs": [
        {
          "id": "pdf",
          "name": "PDF",
          "extension": "pdf",
          "extensions": [
            "pdf"
          ],
          "notes": "Universal document output",
          "available": true,
          "needs": [
            "LibreOffice"
          ]
        },
        {
          "id": "docx",
          "name": "Word (docx)",
          "extension": "docx",
          "extensions": [
            "docx"
          ],
          "notes": "",
          "available": true,
          "needs": [
            "LibreOffice"
          ]
        },
        {
          "id": "doc",
          "name": "Word 97-2003",
          "extension": "doc",
          "extensions": [
            "doc"
          ],
          "notes": "",
          "available": true,
          "needs": [
            "LibreOffice"
          ]
        },
        {
          "id": "odt",
          "name": "OpenDocument Text",
          "extension": "odt",
          "extensions": [
            "odt",
            "fodt"
          ],
          "notes": "",
          "available": true,
          "needs": [
            "LibreOffice"
          ]
        },
        {
          "id": "rtf",
          "name": "Rich Text",
          "extension": "rtf",
          "extensions": [
            "rtf"
          ],
          "notes": "",
          "available": true,
          "needs": [
            "LibreOffice"
          ]
        },
        {
          "id": "txt",
          "name": "Plain text",
          "extension": "txt",
          "extensions": [
            "txt",
            "text",
            "log"
          ],
          "notes": "",
          "available": true,
          "needs": [
            "Pandoc",
            "LibreOffice"
          ]
        },
        {
          "id": "md",
          "name": "Markdown",
          "extension": "md",
          "extensions": [
            "md",
            "markdown",
            "mdown"
          ],
          "notes": "",
          "available": false,
          "needs": [
            "Pandoc"
          ]
        },
        {
          "id": "html",
          "name": "HTML",
          "extension": "html",
          "extensions": [
            "html",
            "htm",
            "xhtml"
          ],
          "notes": "",
          "available": true,
          "needs": [
            "Pandoc",
            "LibreOffice"
          ]
        },
        {
          "id": "epub",
          "name": "EPUB ebook",
          "extension": "epub",
          "extensions": [
            "epub"
          ],
          "notes": "",
          "available": false,
          "needs": [
            "Pandoc"
          ]
        },
        {
          "id": "fb2",
          "name": "FictionBook",
          "extension": "fb2",
          "extensions": [
            "fb2"
          ],
          "notes": "",
          "available": false,
          "needs": [
            "Pandoc"
          ]
        },
        {
          "id": "tex",
          "name": "LaTeX",
          "extension": "tex",
          "extensions": [
            "tex",
            "latex"
          ],
          "notes": "",
          "available": false,
          "needs": [
            "Pandoc"
          ]
        },
        {
          "id": "rst",
          "name": "reStructuredText",
          "extension": "rst",
          "extensions": [
            "rst"
          ],
          "notes": "",
          "available": false,
          "needs": [
            "Pandoc"
          ]
        },
        {
          "id": "pptx",
          "name": "PowerPoint (pptx)",
          "extension": "pptx",
          "extensions": [
            "pptx"
          ],
          "notes": "",
          "available": true,
          "needs": [
            "LibreOffice"
          ]
        },
        {
          "id": "ppt",
          "name": "PowerPoint 97-2003",
          "extension": "ppt",
          "extensions": [
            "ppt"
          ],
          "notes": "",
          "available": true,
          "needs": [
            "LibreOffice"
          ]
        },
        {
          "id": "odp",
          "name": "OpenDocument Presentation",
          "extension": "odp",
          "extensions": [
            "odp"
          ],
          "notes": "",
          "available": true,
          "needs": [
            "LibreOffice"
          ]
        },
        {
          "id": "xlsx",
          "name": "Excel (xlsx)",
          "extension": "xlsx",
          "extensions": [
            "xlsx"
          ],
          "notes": "",
          "available": true,
          "needs": [
            "LibreOffice"
          ]
        },
        {
          "id": "xls",
          "name": "Excel 97-2003",
          "extension": "xls",
          "extensions": [
            "xls"
          ],
          "notes": "",
          "available": true,
          "needs": [
            "LibreOffice"
          ]
        },
        {
          "id": "ods",
          "name": "OpenDocument Spreadsheet",
          "extension": "ods",
          "extensions": [
            "ods",
            "fods"
          ],
          "notes": "",
          "available": true,
          "needs": [
            "LibreOffice"
          ]
        },
        {
          "id": "csv",
          "name": "CSV",
          "extension": "csv",
          "extensions": [
            "csv"
          ],
          "notes": "",
          "available": true,
          "needs": [
            "LibreOffice"
          ]
        },
        {
          "id": "tsv",
          "name": "TSV",
          "extension": "tsv",
          "extensions": [
            "tsv",
            "tab"
          ],
          "notes": "",
          "available": true,
          "needs": [
            "LibreOffice"
          ]
        },
        {
          "id": "json_doc",
          "name": "JSON / YAML data",
          "extension": "json",
          "extensions": [
            "json",
            "yaml",
            "yml"
          ],
          "notes": "Structured text via Pandoc",
          "available": false,
          "needs": [
            "Pandoc"
          ]
        }
      ]
    },
    {
      "id": "subtitle",
      "label": "Subtitle",
      "default_target": "srt",
      "suggested_targets": [
        "srt",
        "vtt",
        "ass"
      ],
      "inputs": [
        {
          "id": "srt",
          "name": "SubRip",
          "extension": "srt",
          "extensions": [
            "srt"
          ],
          "notes": "Most widely supported",
          "available": true,
          "needs": []
        },
        {
          "id": "vtt",
          "name": "WebVTT",
          "extension": "vtt",
          "extensions": [
            "vtt"
          ],
          "notes": "HTML5 <track>",
          "available": true,
          "needs": []
        },
        {
          "id": "ass",
          "name": "Advanced SubStation",
          "extension": "ass",
          "extensions": [
            "ass",
            "ssa"
          ],
          "notes": "Styled subtitles",
          "available": true,
          "needs": []
        },
        {
          "id": "sub",
          "name": "MicroDVD / SubViewer",
          "extension": "sub",
          "extensions": [
            "sub",
            "sbv"
          ],
          "notes": "Read only",
          "available": true,
          "needs": []
        },
        {
          "id": "lrc",
          "name": "LRC lyrics",
          "extension": "lrc",
          "extensions": [
            "lrc"
          ],
          "notes": "",
          "available": true,
          "needs": []
        }
      ],
      "outputs": [
        {
          "id": "srt",
          "name": "SubRip",
          "extension": "srt",
          "extensions": [
            "srt"
          ],
          "notes": "Most widely supported",
          "available": true,
          "needs": []
        },
        {
          "id": "vtt",
          "name": "WebVTT",
          "extension": "vtt",
          "extensions": [
            "vtt"
          ],
          "notes": "HTML5 <track>",
          "available": true,
          "needs": []
        },
        {
          "id": "ass",
          "name": "Advanced SubStation",
          "extension": "ass",
          "extensions": [
            "ass",
            "ssa"
          ],
          "notes": "Styled subtitles",
          "available": true,
          "needs": []
        },
        {
          "id": "lrc",
          "name": "LRC lyrics",
          "extension": "lrc",
          "extensions": [
            "lrc"
          ],
          "notes": "",
          "available": true,
          "needs": []
        },
        {
          "id": "ttml",
          "name": "TTML / DFXP",
          "extension": "ttml",
          "extensions": [
            "ttml",
            "dfxp"
          ],
          "notes": "Write only",
          "available": true,
          "needs": []
        }
      ]
    },
    {
      "id": "flash",
      "label": "Flash",
      "default_target": "mp4",
      "suggested_targets": [
        "mp4",
        "gif",
        "png"
      ],
      "inputs": [
        {
          "id": "swf",
          "name": "Flash movie (SWF)",
          "extension": "swf",
          "extensions": [
            "swf"
          ],
          "notes": "Rendered by Ruffle, then encoded to video/GIF",
          "available": false,
          "needs": [
            "Ruffle"
          ]
        }
      ],
      "outputs": []
    }
  ],
  "presets": [
    {
      "id": "web_and_demo",
      "label": "Web & Demo",
      "description": "1080p H.264 · MP3 192k · JPEG 2560px. Plays everywhere."
    },
    {
      "id": "smallest",
      "label": "Smallest file",
      "description": "720p H.265 · Opus 96k · JPEG 1600px. Built for slow uploads."
    },
    {
      "id": "high_quality",
      "label": "High quality",
      "description": "Original size, visually lossless. Bigger files."
    },
    {
      "id": "archive",
      "label": "Lossless / archive",
      "description": "FLAC / PNG / ProRes. No resizing, keeps metadata."
    }
  ],
  "tools": [
    {
      "id": "ffmpeg",
      "label": "FFmpeg (bundled)",
      "bundled": true,
      "available": true,
      "path": "/Applications/Flint.app/Contents/MacOS/ffmpeg",
      "install_hint": "Shipped inside the app bundle."
    },
    {
      "id": "ffprobe",
      "label": "ffprobe (bundled)",
      "bundled": true,
      "available": true,
      "path": "/Applications/Flint.app/Contents/MacOS/ffprobe",
      "install_hint": "Shipped inside the app bundle."
    },
    {
      "id": "libreoffice",
      "label": "LibreOffice",
      "bundled": false,
      "available": true,
      "path": "/Applications/LibreOffice.app/Contents/MacOS/soffice",
      "install_hint": "brew install --cask libreoffice"
    },
    {
      "id": "pandoc",
      "label": "Pandoc",
      "bundled": false,
      "available": false,
      "path": null,
      "install_hint": "brew install pandoc"
    },
    {
      "id": "magick",
      "label": "ImageMagick",
      "bundled": false,
      "available": true,
      "path": "/opt/homebrew/bin/magick",
      "install_hint": "brew install imagemagick"
    },
    {
      "id": "pdftoppm",
      "label": "Poppler (pdftoppm)",
      "bundled": false,
      "available": true,
      "path": "/opt/homebrew/bin/pdftoppm",
      "install_hint": "brew install poppler"
    },
    {
      "id": "pdftotext",
      "label": "Poppler (pdftotext)",
      "bundled": false,
      "available": false,
      "path": null,
      "install_hint": "brew install poppler"
    },
    {
      "id": "pdftohtml",
      "label": "Poppler (pdftohtml)",
      "bundled": false,
      "available": false,
      "path": null,
      "install_hint": "brew install poppler"
    },
    {
      "id": "ruffle",
      "label": "Ruffle",
      "bundled": false,
      "available": false,
      "path": null,
      "install_hint": "brew install --cask ruffle"
    },
    {
      "id": "yt-dlp",
      "label": "yt-dlp",
      "bundled": false,
      "available": true,
      "path": "/opt/homebrew/bin/yt-dlp",
      "install_hint": "brew install yt-dlp"
    },
    {
      "id": "deno",
      "label": "Deno",
      "bundled": false,
      "available": false,
      "path": null,
      "install_hint": "brew install deno"
    },
    {
      "id": "node",
      "label": "Node.js",
      "bundled": false,
      "available": false,
      "path": null,
      "install_hint": "Flint installs Deno instead (brew install deno)."
    },
    {
      "id": "sips",
      "label": "macOS sips",
      "bundled": false,
      "available": true,
      "path": "/usr/bin/sips",
      "install_hint": "Built into macOS."
    }
  ],
  "input_extension_count": 194
};

/** Lower-case extension → format id, mirroring `format::by_extension`. */
export const EXTENSION_TO_FORMAT: Readonly<Record<string, string>> = {
  "264": "h264",
  "265": "h264",
  "3fr": "raw_camera",
  "3g2": "3gp",
  "3gp": "3gp",
  "8svx": "8svx",
  "aac": "aac",
  "ac3": "ac3",
  "adts": "aac",
  "ai": "ai",
  "aif": "aiff",
  "aifc": "aiff",
  "aiff": "aiff",
  "amr": "amr",
  "ape": "ape",
  "apng": "apng",
  "arw": "raw_camera",
  "asf": "asf",
  "ass": "ass",
  "au": "au",
  "avi": "avi",
  "avif": "avif",
  "bmp": "bmp",
  "bw": "sgi",
  "caf": "caf",
  "cr2": "raw_camera",
  "cr3": "raw_camera",
  "crw": "raw_camera",
  "csv": "csv",
  "dds": "dds",
  "dib": "bmp",
  "dif": "dv",
  "divx": "divx",
  "dng": "raw_camera",
  "doc": "doc",
  "docx": "docx",
  "dpx": "dpx",
  "dts": "dts",
  "dv": "dv",
  "eac3": "eac3",
  "ec3": "eac3",
  "eps": "ai",
  "epub": "epub",
  "erf": "raw_camera",
  "exr": "exr",
  "f4v": "f4v",
  "fb2": "fb2",
  "flac": "flac",
  "flv": "flv",
  "fods": "ods",
  "fodt": "odt",
  "gif": "gif",
  "gsm": "gsm",
  "h264": "h264",
  "h265": "h264",
  "hdr": "hdr",
  "heic": "heic",
  "heif": "heic",
  "hevc": "h264",
  "hif": "heic",
  "htm": "html",
  "html": "html",
  "icb": "tga",
  "icns": "icns",
  "ico": "ico",
  "iff": "8svx",
  "j2k": "jp2",
  "jfif": "jpg",
  "jp2": "jp2",
  "jpe": "jpg",
  "jpeg": "jpg",
  "jpf": "jp2",
  "jpg": "jpg",
  "jpx": "jp2",
  "json": "json_doc",
  "latex": "tex",
  "log": "txt",
  "lrc": "lrc",
  "m1v": "mpg",
  "m2t": "ts",
  "m2ts": "ts",
  "m2v": "mpg",
  "m4a": "m4a",
  "m4b": "m4a",
  "m4r": "m4a",
  "m4v": "m4v",
  "markdown": "md",
  "md": "md",
  "mdown": "md",
  "mid": "midi",
  "midi": "midi",
  "mjpeg": "mjpeg",
  "mjpg": "mjpeg",
  "mka": "mka",
  "mkv": "mkv",
  "mos": "raw_camera",
  "mov": "mov",
  "mp2": "mp2",
  "mp3": "mp3",
  "mp4": "mp4",
  "mpa": "mp2",
  "mpeg": "mpg",
  "mpg": "mpg",
  "mrw": "raw_camera",
  "mts": "ts",
  "mxf": "mxf",
  "nef": "raw_camera",
  "nrw": "raw_camera",
  "nut": "nut",
  "odp": "odp",
  "ods": "ods",
  "odt": "odt",
  "oga": "ogg",
  "ogg": "ogg",
  "ogv": "ogv",
  "ogx": "ogv",
  "opus": "opus",
  "orf": "raw_camera",
  "pam": "ppm",
  "pbm": "ppm",
  "pcx": "pcx",
  "pdf": "pdf",
  "pef": "raw_camera",
  "pgm": "ppm",
  "pic": "hdr",
  "png": "png",
  "pnm": "ppm",
  "ppm": "ppm",
  "ppt": "ppt",
  "pptx": "pptx",
  "ps": "ai",
  "psb": "psd",
  "psd": "psd",
  "qoi": "qoi",
  "qt": "mov",
  "ra": "ra",
  "raf": "raw_camera",
  "ras": "sun",
  "raw": "raw_camera",
  "rgb": "sgi",
  "rgba": "sgi",
  "rm": "rm",
  "rmvb": "rm",
  "rst": "rst",
  "rtf": "rtf",
  "rw2": "raw_camera",
  "sbv": "sub",
  "sgi": "sgi",
  "snd": "au",
  "spx": "spx",
  "sr2": "raw_camera",
  "srf": "raw_camera",
  "srt": "srt",
  "srw": "raw_camera",
  "ssa": "ass",
  "sub": "sub",
  "sun": "sun",
  "svg": "svg",
  "svgz": "svg",
  "swf": "swf",
  "tab": "tsv",
  "tex": "tex",
  "text": "txt",
  "tga": "tga",
  "tif": "tiff",
  "tiff": "tiff",
  "ts": "ts",
  "tsv": "tsv",
  "tta": "tta",
  "txt": "txt",
  "vda": "tga",
  "vob": "mpg",
  "voc": "voc",
  "vst": "tga",
  "vtt": "vtt",
  "w64": "w64",
  "wav": "wav",
  "wave": "wav",
  "wbmp": "wbmp",
  "webm": "webm",
  "webp": "webp",
  "wma": "wma",
  "wmv": "wmv",
  "wv": "wv",
  "x3f": "raw_camera",
  "xbm": "xbm",
  "xhtml": "html",
  "xls": "xls",
  "xlsx": "xlsx",
  "xpm": "xpm",
  "xvid": "divx",
  "y4m": "y4m",
  "yaml": "json_doc",
  "yml": "json_doc"
};

/** Format id → display name + category, for the faked `inspect_files`. */
export const FORMAT_META: Readonly<Record<string, { name: string; category: string }>> = {
  "3gp": {
    "category": "video",
    "name": "3GPP Mobile"
  },
  "8svx": {
    "category": "audio",
    "name": "Amiga 8SVX"
  },
  "aac": {
    "category": "audio",
    "name": "Raw AAC (ADTS)"
  },
  "ac3": {
    "category": "audio",
    "name": "Dolby Digital AC-3"
  },
  "ai": {
    "category": "image",
    "name": "Illustrator / EPS / PS"
  },
  "aiff": {
    "category": "audio",
    "name": "AIFF / AIFC"
  },
  "alac": {
    "category": "audio",
    "name": "Apple Lossless"
  },
  "amr": {
    "category": "audio",
    "name": "AMR narrowband"
  },
  "ape": {
    "category": "audio",
    "name": "Monkey's Audio"
  },
  "apng": {
    "category": "image",
    "name": "Animated PNG"
  },
  "asf": {
    "category": "video",
    "name": "Advanced Systems Format"
  },
  "ass": {
    "category": "subtitle",
    "name": "Advanced SubStation"
  },
  "au": {
    "category": "audio",
    "name": "Sun/NeXT AU"
  },
  "avi": {
    "category": "video",
    "name": "AVI"
  },
  "avif": {
    "category": "image",
    "name": "AVIF (still + animated)"
  },
  "bmp": {
    "category": "image",
    "name": "Windows Bitmap"
  },
  "caf": {
    "category": "audio",
    "name": "Core Audio Format"
  },
  "csv": {
    "category": "document",
    "name": "CSV"
  },
  "dds": {
    "category": "image",
    "name": "DirectDraw Surface"
  },
  "divx": {
    "category": "video",
    "name": "DivX / Xvid"
  },
  "doc": {
    "category": "document",
    "name": "Word 97-2003"
  },
  "docx": {
    "category": "document",
    "name": "Word (docx)"
  },
  "dpx": {
    "category": "image",
    "name": "DPX"
  },
  "dts": {
    "category": "audio",
    "name": "DTS"
  },
  "dv": {
    "category": "video",
    "name": "DV / DVCPRO"
  },
  "eac3": {
    "category": "audio",
    "name": "Dolby Digital Plus"
  },
  "epub": {
    "category": "document",
    "name": "EPUB ebook"
  },
  "exr": {
    "category": "image",
    "name": "OpenEXR (HDR)"
  },
  "f4v": {
    "category": "video",
    "name": "Flash MP4 Video"
  },
  "fb2": {
    "category": "document",
    "name": "FictionBook"
  },
  "flac": {
    "category": "audio",
    "name": "FLAC"
  },
  "flv": {
    "category": "video",
    "name": "Flash Video"
  },
  "gif": {
    "category": "image",
    "name": "GIF (animated)"
  },
  "gsm": {
    "category": "audio",
    "name": "GSM 06.10"
  },
  "h264": {
    "category": "video",
    "name": "Raw H.264 / H.265 stream"
  },
  "hdr": {
    "category": "image",
    "name": "Radiance HDR"
  },
  "heic": {
    "category": "image",
    "name": "HEIC / HEIF"
  },
  "html": {
    "category": "document",
    "name": "HTML"
  },
  "icns": {
    "category": "image",
    "name": "Apple Icon Image"
  },
  "ico": {
    "category": "image",
    "name": "Windows Icon"
  },
  "jp2": {
    "category": "image",
    "name": "JPEG 2000"
  },
  "jpg": {
    "category": "image",
    "name": "JPEG"
  },
  "json_doc": {
    "category": "document",
    "name": "JSON / YAML data"
  },
  "lrc": {
    "category": "subtitle",
    "name": "LRC lyrics"
  },
  "m4a": {
    "category": "audio",
    "name": "AAC in MP4 (m4a)"
  },
  "m4v": {
    "category": "video",
    "name": "iTunes Video"
  },
  "md": {
    "category": "document",
    "name": "Markdown"
  },
  "midi": {
    "category": "audio",
    "name": "MIDI (piano)"
  },
  "mjpeg": {
    "category": "video",
    "name": "Motion JPEG"
  },
  "mka": {
    "category": "audio",
    "name": "Matroska Audio"
  },
  "mkv": {
    "category": "video",
    "name": "Matroska"
  },
  "mov": {
    "category": "video",
    "name": "QuickMovie / ProRes"
  },
  "mp2": {
    "category": "audio",
    "name": "MPEG audio layer II"
  },
  "mp3": {
    "category": "audio",
    "name": "MP3"
  },
  "mp4": {
    "category": "video",
    "name": "MP4 (H.264/H.265/AV1)"
  },
  "mpg": {
    "category": "video",
    "name": "MPEG-1/2 Program Stream"
  },
  "mxf": {
    "category": "video",
    "name": "Material Exchange Format"
  },
  "nut": {
    "category": "video",
    "name": "NUT"
  },
  "odp": {
    "category": "document",
    "name": "OpenDocument Presentation"
  },
  "ods": {
    "category": "document",
    "name": "OpenDocument Spreadsheet"
  },
  "odt": {
    "category": "document",
    "name": "OpenDocument Text"
  },
  "ogg": {
    "category": "audio",
    "name": "Ogg Vorbis"
  },
  "ogv": {
    "category": "video",
    "name": "Ogg Theora"
  },
  "opus": {
    "category": "audio",
    "name": "Opus"
  },
  "pcx": {
    "category": "image",
    "name": "PC Paintbrush"
  },
  "pdf": {
    "category": "document",
    "name": "PDF"
  },
  "pdf_page": {
    "category": "image",
    "name": "PDF page (as image)"
  },
  "png": {
    "category": "image",
    "name": "PNG"
  },
  "ppm": {
    "category": "image",
    "name": "Netpbm"
  },
  "ppt": {
    "category": "document",
    "name": "PowerPoint 97-2003"
  },
  "pptx": {
    "category": "document",
    "name": "PowerPoint (pptx)"
  },
  "psd": {
    "category": "image",
    "name": "Photoshop"
  },
  "qoi": {
    "category": "image",
    "name": "QOI"
  },
  "ra": {
    "category": "audio",
    "name": "RealAudio"
  },
  "raw_camera": {
    "category": "image",
    "name": "Camera RAW"
  },
  "rm": {
    "category": "video",
    "name": "RealMedia"
  },
  "rst": {
    "category": "document",
    "name": "reStructuredText"
  },
  "rtf": {
    "category": "document",
    "name": "Rich Text"
  },
  "sgi": {
    "category": "image",
    "name": "SGI / RGB"
  },
  "spx": {
    "category": "audio",
    "name": "Speex"
  },
  "srt": {
    "category": "subtitle",
    "name": "SubRip"
  },
  "sub": {
    "category": "subtitle",
    "name": "MicroDVD / SubViewer"
  },
  "sun": {
    "category": "image",
    "name": "Sun Raster"
  },
  "svg": {
    "category": "image",
    "name": "SVG (vector)"
  },
  "swf": {
    "category": "flash",
    "name": "Flash movie (SWF)"
  },
  "tex": {
    "category": "document",
    "name": "LaTeX"
  },
  "tga": {
    "category": "image",
    "name": "Truevision TGA"
  },
  "tiff": {
    "category": "image",
    "name": "TIFF"
  },
  "ts": {
    "category": "video",
    "name": "MPEG Transport Stream"
  },
  "tsv": {
    "category": "document",
    "name": "TSV"
  },
  "tta": {
    "category": "audio",
    "name": "True Audio"
  },
  "ttml": {
    "category": "subtitle",
    "name": "TTML / DFXP"
  },
  "txt": {
    "category": "document",
    "name": "Plain text"
  },
  "voc": {
    "category": "audio",
    "name": "Creative Voice"
  },
  "vtt": {
    "category": "subtitle",
    "name": "WebVTT"
  },
  "w64": {
    "category": "audio",
    "name": "Sony Wave64"
  },
  "wav": {
    "category": "audio",
    "name": "WAV (PCM)"
  },
  "wbmp": {
    "category": "image",
    "name": "Wireless Bitmap"
  },
  "webm": {
    "category": "video",
    "name": "WebM (VP9/AV1)"
  },
  "webp": {
    "category": "image",
    "name": "WebP (still + animated)"
  },
  "wma": {
    "category": "audio",
    "name": "Windows Media Audio"
  },
  "wmv": {
    "category": "video",
    "name": "Windows Media Video"
  },
  "wv": {
    "category": "audio",
    "name": "WavPack"
  },
  "xbm": {
    "category": "image",
    "name": "X BitMap"
  },
  "xls": {
    "category": "document",
    "name": "Excel 97-2003"
  },
  "xlsx": {
    "category": "document",
    "name": "Excel (xlsx)"
  },
  "xpm": {
    "category": "image",
    "name": "X PixMap"
  },
  "y4m": {
    "category": "video",
    "name": "YUV4MPEG2"
  }
};

/**
 * Format id → the helper *binaries* each direction declares, in the planner's order.
 *
 * `FormatView.needs` above is the *package* names a person installs ("Poppler", once, for three
 * binaries); this is the executable-level list, which is what answers "can this machine do it?".
 */
export const FORMAT_HELPERS: Readonly<
  Record<string, { read: string[]; write: string[] }>
> = {
  "ai": {
    "read": [
      "magick"
    ],
    "write": [
      "magick"
    ]
  },
  "csv": {
    "read": [
      "libreoffice"
    ],
    "write": [
      "libreoffice"
    ]
  },
  "doc": {
    "read": [
      "libreoffice"
    ],
    "write": [
      "libreoffice"
    ]
  },
  "docx": {
    "read": [
      "libreoffice"
    ],
    "write": [
      "libreoffice"
    ]
  },
  "epub": {
    "read": [
      "pandoc"
    ],
    "write": [
      "pandoc"
    ]
  },
  "fb2": {
    "read": [
      "pandoc"
    ],
    "write": [
      "pandoc"
    ]
  },
  "heic": {
    "read": [
      "sips",
      "magick"
    ],
    "write": [
      "sips",
      "magick"
    ]
  },
  "html": {
    "read": [
      "pandoc",
      "libreoffice"
    ],
    "write": [
      "pandoc",
      "libreoffice"
    ]
  },
  "icns": {
    "read": [
      "sips",
      "magick"
    ],
    "write": [
      "sips",
      "magick"
    ]
  },
  "json_doc": {
    "read": [
      "pandoc"
    ],
    "write": [
      "pandoc"
    ]
  },
  "md": {
    "read": [
      "pandoc"
    ],
    "write": [
      "pandoc"
    ]
  },
  "odp": {
    "read": [
      "libreoffice"
    ],
    "write": [
      "libreoffice"
    ]
  },
  "ods": {
    "read": [
      "libreoffice"
    ],
    "write": [
      "libreoffice"
    ]
  },
  "odt": {
    "read": [
      "libreoffice"
    ],
    "write": [
      "libreoffice"
    ]
  },
  "pdf": {
    "read": [
      "libreoffice",
      "pdftoppm",
      "pdftotext",
      "pdftohtml",
      "magick",
      "sips"
    ],
    "write": [
      "libreoffice"
    ]
  },
  "pdf_page": {
    "read": [],
    "write": [
      "magick"
    ]
  },
  "ppt": {
    "read": [
      "libreoffice"
    ],
    "write": [
      "libreoffice"
    ]
  },
  "pptx": {
    "read": [
      "libreoffice"
    ],
    "write": [
      "libreoffice"
    ]
  },
  "psd": {
    "read": [
      "magick"
    ],
    "write": []
  },
  "raw_camera": {
    "read": [
      "sips",
      "magick"
    ],
    "write": []
  },
  "rst": {
    "read": [
      "pandoc"
    ],
    "write": [
      "pandoc"
    ]
  },
  "rtf": {
    "read": [
      "libreoffice"
    ],
    "write": [
      "libreoffice"
    ]
  },
  "svg": {
    "read": [
      "magick"
    ],
    "write": []
  },
  "swf": {
    "read": [
      "ruffle"
    ],
    "write": []
  },
  "tex": {
    "read": [
      "pandoc"
    ],
    "write": [
      "pandoc"
    ]
  },
  "tsv": {
    "read": [
      "libreoffice"
    ],
    "write": [
      "libreoffice"
    ]
  },
  "txt": {
    "read": [
      "pandoc",
      "libreoffice"
    ],
    "write": [
      "pandoc",
      "libreoffice"
    ]
  },
  "xls": {
    "read": [
      "libreoffice"
    ],
    "write": [
      "libreoffice"
    ]
  },
  "xlsx": {
    "read": [
      "libreoffice"
    ],
    "write": [
      "libreoffice"
    ]
  }
};
