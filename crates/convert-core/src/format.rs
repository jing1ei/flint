//! The format catalog: the single source of truth for "what can Flint convert".
//!
//! The UI, the planner and `FORMATS.md` are all generated from this table, so the app can never
//! advertise a format it cannot actually handle.

use serde::{Deserialize, Serialize};

/// External helper binaries. Bundled formats use FFmpeg or the built-in MIDI piano renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tool {
    /// Bundled static FFmpeg (video, audio, images, animation, subtitles).
    Ffmpeg,
    /// Bundled ffprobe, used for duration/stream detection.
    Ffprobe,
    /// LibreOffice (`soffice`) - office documents & spreadsheets.
    LibreOffice,
    /// Pandoc - markup, ebooks, LaTeX.
    Pandoc,
    /// ImageMagick (`magick`) - vector/layered/RAW images, image -> pdf.
    Magick,
    /// Poppler `pdftoppm` - fast, high quality PDF rasterising.
    PdfToPpm,
    /// Poppler `pdftotext` - PDF text extraction. Same package as `pdftoppm`, its own binary.
    PdfToText,
    /// Poppler `pdftohtml` - PDF to a single self-contained HTML file.
    PdfToHtml,
    /// Ruffle exporter - Flash `.swf` frame extraction.
    Ruffle,
    /// yt-dlp - fetches a YouTube/Bilibili link into a file the rest of the pipeline can convert.
    /// The only tool here that is not a *converter*: it is how a link becomes a source at all, so
    /// it unlocks no catalog entry (see [`crate::install::unlocks`]).
    YtDlp,
    /// Deno - the JavaScript runtime yt-dlp hands YouTube's challenges to, and the only one it
    /// enables by default (`yt-dlp --help`, 2026.08.19: "Supported runtimes are (in order of
    /// priority, from highest to lowest): deno, node, quickjs, bun. Only \"deno\" is enabled by
    /// default").
    ///
    /// Converts nothing, like [`Tool::YtDlp`]: it is what makes a *fetch* work at all. What a
    /// machine without one actually sees is written once, where it is diagnosed - see
    /// [`crate::link::FetchFailure::NoJsRuntime`] - and said to the user once, in
    /// [`crate::install::unlocks`].
    Deno,
    /// Node - yt-dlp's second-choice JavaScript runtime, accepted under exactly this name (the
    /// `--help` text above lists it second) and used when Deno is not there.
    ///
    /// Never installed by this app: a converter has no business installing a general-purpose
    /// toolchain, so Node is looked for on behalf of people who already have it and nothing else.
    Node,
    /// macOS built-in `sips` - HEIC/ICNS/RAW fallback, always present on macOS.
    Sips,
}

impl Tool {
    pub const fn id(self) -> &'static str {
        match self {
            Tool::Ffmpeg => "ffmpeg",
            Tool::Ffprobe => "ffprobe",
            Tool::LibreOffice => "libreoffice",
            Tool::Pandoc => "pandoc",
            Tool::Magick => "magick",
            Tool::PdfToPpm => "pdftoppm",
            Tool::PdfToText => "pdftotext",
            Tool::PdfToHtml => "pdftohtml",
            Tool::Ruffle => "ruffle",
            Tool::YtDlp => "yt-dlp",
            Tool::Deno => "deno",
            Tool::Node => "node",
            Tool::Sips => "sips",
        }
    }

    /// Human label shown in the "missing helper" hint.
    pub const fn label(self) -> &'static str {
        match self {
            Tool::Ffmpeg => "FFmpeg (bundled)",
            Tool::Ffprobe => "ffprobe (bundled)",
            Tool::LibreOffice => "LibreOffice",
            Tool::Pandoc => "Pandoc",
            Tool::Magick => "ImageMagick",
            Tool::PdfToPpm => "Poppler (pdftoppm)",
            Tool::PdfToText => "Poppler (pdftotext)",
            Tool::PdfToHtml => "Poppler (pdftohtml)",
            Tool::Ruffle => "Ruffle",
            Tool::YtDlp => "yt-dlp",
            Tool::Deno => "Deno",
            Tool::Node => "Node.js",
            Tool::Sips => "macOS sips",
        }
    }

    /// One-liner we show so a user can fix a missing helper without leaving the app.
    pub const fn install_hint(self) -> &'static str {
        if cfg!(windows) {
            return match self {
                Tool::Ffmpeg | Tool::Ffprobe => "Shipped inside the app bundle.",
                Tool::LibreOffice => "In PowerShell: winget install --exact --id TheDocumentFoundation.LibreOffice",
                Tool::Pandoc => "In PowerShell: winget install --exact --id JohnMacFarlane.Pandoc",
                Tool::Magick => "Install ImageMagick from imagemagick.org, enable Add to PATH, then restart the app.",
                Tool::PdfToPpm | Tool::PdfToText | Tool::PdfToHtml =>
                    "Install Poppler for Windows from github.com/oschwartz10612/poppler-windows, add Library\\bin to PATH, then restart the app.",
                Tool::YtDlp => "In PowerShell: winget install --exact --id yt-dlp.yt-dlp",
                Tool::Deno => "In PowerShell: winget install --exact --id DenoLand.Deno",
                Tool::Node => "Install Node.js from nodejs.org or install Deno, then restart the app.",
                Tool::Ruffle => "Install a compatible ruffle_exporter and add it to PATH; the ordinary Ruffle player cannot export frames.",
                Tool::Sips => "Available only on macOS. Install ImageMagick for these image formats.",
            };
        }
        match self {
            Tool::Ffmpeg | Tool::Ffprobe => "Shipped inside the app bundle.",
            Tool::LibreOffice => "brew install --cask libreoffice",
            Tool::Pandoc => "brew install pandoc",
            Tool::Magick => "brew install imagemagick",
            // One package, three binaries: whichever one is missing, `brew install poppler` is the
            // answer.
            Tool::PdfToPpm | Tool::PdfToText | Tool::PdfToHtml => "brew install poppler",
            Tool::Ruffle => "brew install --cask ruffle",
            Tool::YtDlp => "brew install yt-dlp",
            Tool::Deno => "brew install deno",
            // Deliberately not a command of its own: this app installs a JavaScript *runtime* for
            // yt-dlp, not a JavaScript toolchain, so a machine without Node is pointed at the one
            // package we do install rather than told to set up nodejs.org.
            Tool::Node => "Flint installs Deno instead (brew install deno).",
            Tool::Sips => "Built into macOS.",
        }
    }

    pub const fn is_bundled(self) -> bool {
        matches!(self, Tool::Ffmpeg | Tool::Ffprobe)
    }

    /// The name yt-dlp knows this helper by in `--js-runtimes RUNTIME[:PATH]`, or `None` for
    /// everything that is not a JavaScript runtime.
    ///
    /// These strings are yt-dlp's vocabulary, not ours, which is why they are written here rather
    /// than assumed to equal [`Tool::id`]: `deno` and `node` happen to match today, and a runtime
    /// whose formula and flag value disagree would otherwise be a silent no-op flag.
    pub const fn js_runtime_name(self) -> Option<&'static str> {
        match self {
            Tool::Deno => Some("deno"),
            Tool::Node => Some("node"),
            _ => None,
        }
    }
}

/// The JavaScript runtimes yt-dlp can be pointed at, in *its* order of preference.
///
/// Same order as `yt-dlp --help` ("in order of priority, from highest to lowest: deno, node, …"),
/// so handing it the first one we found is handing it the one it would have chosen. QuickJS and Bun
/// are left out deliberately: nothing installs them for the user, and a helper this app can neither
/// find in a well-known place nor install is a row in Settings that does nothing.
pub const JS_RUNTIMES: &[Tool] = &[Tool::Deno, Tool::Node];

/// How well a format is supported in one direction (read or write).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Support {
    /// Not supported in this direction.
    Unsupported,
    /// Works out of the box with the bundled engine.
    Bundled,
    /// Needs any *one* of these helper tools to be installed.
    AnyOf(&'static [Tool]),
}

impl Support {
    pub const fn is_supported(self) -> bool {
        !matches!(self, Support::Unsupported)
    }
    pub const fn needs_helper(self) -> bool {
        matches!(self, Support::AnyOf(_))
    }
    pub fn helpers(self) -> &'static [Tool] {
        match self {
            Support::AnyOf(tools) => tools,
            _ => &[],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Video,
    Audio,
    Image,
    Document,
    Subtitle,
    Flash,
}

impl Category {
    pub const ALL: [Category; 6] = [
        Category::Video,
        Category::Audio,
        Category::Image,
        Category::Document,
        Category::Subtitle,
        Category::Flash,
    ];

    pub const fn id(self) -> &'static str {
        match self {
            Category::Video => "video",
            Category::Audio => "audio",
            Category::Image => "image",
            Category::Document => "document",
            Category::Subtitle => "subtitle",
            Category::Flash => "flash",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Category::Video => "Video",
            Category::Audio => "Audio",
            Category::Image => "Image",
            Category::Document => "Document",
            Category::Subtitle => "Subtitle",
            Category::Flash => "Flash",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Format {
    /// Stable identifier, also the default output extension.
    pub id: &'static str,
    pub name: &'static str,
    pub extensions: &'static [&'static str],
    pub category: Category,
    pub read: Support,
    pub write: Support,
    pub notes: &'static str,
}

impl Format {
    pub fn matches_extension(&self, ext: &str) -> bool {
        let ext = ext.trim_start_matches('.').to_ascii_lowercase();
        self.extensions.iter().any(|e| *e == ext)
    }
}

use Category::{Audio, Document, Flash, Image, Subtitle, Video};
use Support::{AnyOf, Bundled, Unsupported};

const fn f(
    id: &'static str,
    name: &'static str,
    extensions: &'static [&'static str],
    category: Category,
    read: Support,
    write: Support,
    notes: &'static str,
) -> Format {
    Format { id, name, extensions, category, read, write, notes }
}

/// Formats Apple's `sips` and ImageMagick both handle. `sips` comes first because it is present
/// on every Mac, needs no install, and is the only one of the two that reliably reads iPhone HEIC
/// and camera RAW; ImageMagick is the fallback for the rest of the world.
const SIPS_OR_MAGICK: Support = AnyOf(&[Tool::Sips, Tool::Magick]);
const OFFICE: Support = AnyOf(&[Tool::LibreOffice]);
/// Plain text and HTML: Pandoc keeps the structure, LibreOffice can open either one on its own.
/// Markdown is deliberately *not* here - see the `md` entry.
const MARKUP: Support = AnyOf(&[Tool::Pandoc, Tool::LibreOffice]);
/// Only Pandoc reads or writes Markdown; LibreOffice has no Markdown filter, and the planner has
/// never had a route that used it for one.
const MARKDOWN: Support = AnyOf(&[Tool::Pandoc]);

/// The complete catalog. `read` = can be used as input, `write` = can be produced as output.
///
/// Private: [`catalog`] is the one way in, so a caller cannot end up holding a stale copy of a
/// slice that the accessor might one day filter.
const CATALOG: &[Format] = &[
    // ---------------------------------------------------------------- video
    f(
        "mp4",
        "MP4 (H.264/H.265/AV1)",
        &["mp4"],
        Video,
        Bundled,
        Bundled,
        "Best all-round web + demo format",
    ),
    f("m4v", "iTunes Video", &["m4v"], Video, Bundled, Bundled, ""),
    f(
        "mov",
        "QuickMovie / ProRes",
        &["mov", "qt"],
        Video,
        Bundled,
        Bundled,
        "ProRes output for editing round-trips",
    ),
    f("mkv", "Matroska", &["mkv"], Video, Bundled, Bundled, "Keeps multiple audio/subtitle tracks"),
    f("webm", "WebM (VP9/AV1)", &["webm"], Video, Bundled, Bundled, "Smallest files for web pages"),
    f("avi", "AVI", &["avi"], Video, Bundled, Bundled, "Legacy container; H.264 on output"),
    f(
        "wmv",
        "Windows Media Video",
        &["wmv"],
        Video,
        Bundled,
        Bundled,
        "H.264 inside an ASF container",
    ),
    f("flv", "Flash Video", &["flv"], Video, Bundled, Bundled, "Flash era video, read + write"),
    f("f4v", "Flash MP4 Video", &["f4v"], Video, Bundled, Bundled, ""),
    f(
        "mpg",
        "MPEG-1/2 Program Stream",
        &["mpg", "mpeg", "m1v", "m2v", "vob"],
        Video,
        Bundled,
        Bundled,
        "DVD / VOB sources",
    ),
    f(
        "ts",
        "MPEG Transport Stream",
        &["ts", "m2ts", "mts", "m2t"],
        Video,
        Bundled,
        Bundled,
        "Camcorder / broadcast captures",
    ),
    f("3gp", "3GPP Mobile", &["3gp", "3g2"], Video, Bundled, Bundled, "Old phone video"),
    f("ogv", "Ogg Theora", &["ogv", "ogx"], Video, Bundled, Bundled, ""),
    f("asf", "Advanced Systems Format", &["asf"], Video, Bundled, Bundled, ""),
    f("rm", "RealMedia", &["rm", "rmvb"], Video, Bundled, Unsupported, "Decode only"),
    f("divx", "DivX / Xvid", &["divx", "xvid"], Video, Bundled, Unsupported, "Decode only"),
    f(
        "dv",
        "DV / DVCPRO",
        &["dv", "dif"],
        Video,
        Bundled,
        Unsupported,
        "Decode only; DV needs exact 720×480/576 frames",
    ),
    f("mxf", "Material Exchange Format", &["mxf"], Video, Bundled, Bundled, "Broadcast masters"),
    f("y4m", "YUV4MPEG2", &["y4m"], Video, Bundled, Bundled, "Raw intermediate"),
    f(
        "h264",
        "Raw H.264 / H.265 stream",
        &["h264", "264", "h265", "265", "hevc"],
        Video,
        Bundled,
        Bundled,
        "Elementary streams",
    ),
    f("nut", "NUT", &["nut"], Video, Bundled, Bundled, ""),
    f("mjpeg", "Motion JPEG", &["mjpeg", "mjpg"], Video, Bundled, Bundled, ""),
    // ---------------------------------------------------------------- audio
    f(
        "midi",
        "MIDI (piano)",
        &["mid", "midi"],
        Audio,
        Bundled,
        Unsupported,
        "Render notes with the built-in basic piano; type 0/1; audio output only",
    ),
    f("mp3", "MP3", &["mp3"], Audio, Bundled, Bundled, "Universal audio default"),
    f(
        "m4a",
        "AAC in MP4 (m4a)",
        &["m4a", "m4b", "m4r"],
        Audio,
        Bundled,
        Bundled,
        "Best quality-per-byte for Apple devices",
    ),
    f("aac", "Raw AAC (ADTS)", &["aac", "adts"], Audio, Bundled, Bundled, ""),
    f("wav", "WAV (PCM)", &["wav", "wave"], Audio, Bundled, Bundled, "Lossless, uncompressed"),
    f("flac", "FLAC", &["flac"], Audio, Bundled, Bundled, "Lossless, compressed"),
    f(
        "alac",
        "Apple Lossless",
        &["m4a"],
        Audio,
        Unsupported,
        Bundled,
        "Written into an .m4a container",
    ),
    f("opus", "Opus", &["opus"], Audio, Bundled, Bundled, "Best small-size speech/music"),
    f("ogg", "Ogg Vorbis", &["ogg", "oga"], Audio, Bundled, Bundled, ""),
    f("aiff", "AIFF / AIFC", &["aiff", "aif", "aifc"], Audio, Bundled, Bundled, ""),
    f("caf", "Core Audio Format", &["caf"], Audio, Bundled, Bundled, "macOS native"),
    f("wma", "Windows Media Audio", &["wma"], Audio, Bundled, Bundled, "Output uses wmav2"),
    f("ac3", "Dolby Digital AC-3", &["ac3"], Audio, Bundled, Bundled, ""),
    f("eac3", "Dolby Digital Plus", &["eac3", "ec3"], Audio, Bundled, Bundled, ""),
    f("dts", "DTS", &["dts"], Audio, Bundled, Bundled, "Experimental encoder"),
    f("mka", "Matroska Audio", &["mka"], Audio, Bundled, Bundled, ""),
    // Encoding AMR-NB needs an external library (`--enable-libopencore-amrnb`); FFmpeg has no
    // encoder of its own, and the Apple Silicon build we ship is not configured with one. The
    // muxer is there, so the job got as far as "Output file does not contain any stream".
    f("amr", "AMR narrowband", &["amr"], Audio, Bundled, Unsupported, "Decode only (8 kHz voice)"),
    f("mp2", "MPEG audio layer II", &["mp2", "mpa"], Audio, Bundled, Bundled, ""),
    f("ape", "Monkey's Audio", &["ape"], Audio, Bundled, Unsupported, "Decode only"),
    f("wv", "WavPack", &["wv"], Audio, Bundled, Bundled, ""),
    f("tta", "True Audio", &["tta"], Audio, Bundled, Bundled, ""),
    f("au", "Sun/NeXT AU", &["au", "snd"], Audio, Bundled, Bundled, ""),
    f("voc", "Creative Voice", &["voc"], Audio, Bundled, Bundled, ""),
    f("w64", "Sony Wave64", &["w64"], Audio, Bundled, Bundled, ""),
    // Same story as AMR: the GSM 06.10 *encoder* is libgsm (`--enable-libgsm`), which the Apple
    // Silicon FFmpeg we ship does not have. Decoding is native, so `.gsm` input still works.
    f("gsm", "GSM 06.10", &["gsm"], Audio, Bundled, Unsupported, "Decode only"),
    f("spx", "Speex", &["spx"], Audio, Bundled, Unsupported, "Decode only"),
    f("ra", "RealAudio", &["ra"], Audio, Bundled, Unsupported, "Decode only"),
    f("8svx", "Amiga 8SVX", &["8svx", "iff"], Audio, Bundled, Unsupported, "Decode only"),
    // ---------------------------------------------------------------- image
    f(
        "jpg",
        "JPEG",
        &["jpg", "jpeg", "jpe", "jfif"],
        Image,
        Bundled,
        Bundled,
        "Default photo output",
    ),
    f("png", "PNG", &["png"], Image, Bundled, Bundled, "Lossless, transparency"),
    f(
        "webp",
        "WebP (still + animated)",
        &["webp"],
        Image,
        Bundled,
        Bundled,
        "~30% smaller than JPEG",
    ),
    f(
        "avif",
        "AVIF (still + animated)",
        &["avif"],
        Image,
        Bundled,
        Bundled,
        "Smallest modern web image",
    ),
    f("gif", "GIF (animated)", &["gif"], Image, Bundled, Bundled, "Palette-optimised output"),
    f("apng", "Animated PNG", &["apng"], Image, Bundled, Bundled, ""),
    f("tiff", "TIFF", &["tiff", "tif"], Image, Bundled, Bundled, "Print / archival"),
    f("bmp", "Windows Bitmap", &["bmp", "dib"], Image, Bundled, Bundled, ""),
    f("ico", "Windows Icon", &["ico"], Image, Bundled, Bundled, "Multi-size favicons"),
    f(
        "heic",
        "HEIC / HEIF",
        &["heic", "heif", "hif"],
        Image,
        SIPS_OR_MAGICK,
        SIPS_OR_MAGICK,
        "iPhone photos",
    ),
    f(
        "icns",
        "Apple Icon Image",
        &["icns"],
        Image,
        SIPS_OR_MAGICK,
        SIPS_OR_MAGICK,
        "macOS app icons",
    ),
    f(
        "svg",
        "SVG (vector)",
        &["svg", "svgz"],
        Image,
        AnyOf(&[Tool::Magick]),
        Unsupported,
        "Rasterised on input; no raster->vector",
    ),
    f(
        "pdf_page",
        "PDF page (as image)",
        &["pdf"],
        Image,
        Unsupported,
        AnyOf(&[Tool::Magick]),
        "Image → PDF page (the PDF → image direction lives in Document)",
    ),
    f(
        "psd",
        "Photoshop",
        &["psd", "psb"],
        Image,
        AnyOf(&[Tool::Magick]),
        Unsupported,
        "Flattened composite",
    ),
    f(
        "ai",
        "Illustrator / EPS / PS",
        &["ai", "eps", "ps"],
        Image,
        AnyOf(&[Tool::Magick]),
        AnyOf(&[Tool::Magick]),
        "",
    ),
    f("tga", "Truevision TGA", &["tga", "icb", "vda", "vst"], Image, Bundled, Bundled, ""),
    f("ppm", "Netpbm", &["ppm", "pgm", "pbm", "pnm", "pam"], Image, Bundled, Bundled, ""),
    f("pcx", "PC Paintbrush", &["pcx"], Image, Bundled, Bundled, ""),
    f(
        "dds",
        "DirectDraw Surface",
        &["dds"],
        Image,
        Bundled,
        Unsupported,
        "Game textures, decode only",
    ),
    f("exr", "OpenEXR (HDR)", &["exr"], Image, Bundled, Bundled, "VFX / HDR"),
    f("hdr", "Radiance HDR", &["hdr", "pic"], Image, Bundled, Bundled, ""),
    f("jp2", "JPEG 2000", &["jp2", "j2k", "jpf", "jpx"], Image, Bundled, Bundled, ""),
    f("xpm", "X PixMap", &["xpm"], Image, Bundled, Unsupported, "Decode only"),
    f("xbm", "X BitMap", &["xbm"], Image, Bundled, Bundled, ""),
    f("dpx", "DPX", &["dpx"], Image, Bundled, Bundled, "Film scans"),
    f("sgi", "SGI / RGB", &["sgi", "rgb", "rgba", "bw"], Image, Bundled, Bundled, ""),
    f("sun", "Sun Raster", &["ras", "sun"], Image, Bundled, Bundled, ""),
    f("wbmp", "Wireless Bitmap", &["wbmp"], Image, Bundled, Bundled, ""),
    f("qoi", "QOI", &["qoi"], Image, Bundled, Bundled, ""),
    f(
        "raw_camera",
        "Camera RAW",
        &[
            "cr2", "cr3", "crw", "nef", "nrw", "arw", "srf", "sr2", "dng", "orf", "rw2", "raf",
            "pef", "srw", "x3f", "3fr", "erf", "mos", "mrw", "raw",
        ],
        Image,
        SIPS_OR_MAGICK,
        Unsupported,
        "Decode only (Canon, Nikon, Sony, Fuji, Olympus, Panasonic, Pentax, Samsung, Sigma...)",
    ),
    // ------------------------------------------------------------- document
    f(
        "pdf",
        "PDF",
        &["pdf"],
        Document,
        // Poppler's `pdftotext`/`pdftohtml` extract a PDF's text (15 MB of helper instead of 800),
        // `pdftoppm`/`magick`/`sips` rasterise its pages, and LibreOffice re-imports the layout.
        AnyOf(&[
            Tool::LibreOffice,
            Tool::PdfToPpm,
            Tool::PdfToText,
            Tool::PdfToHtml,
            Tool::Magick,
            Tool::Sips,
        ]),
        OFFICE,
        "Universal document output",
    ),
    f("docx", "Word (docx)", &["docx"], Document, OFFICE, OFFICE, ""),
    f("doc", "Word 97-2003", &["doc"], Document, OFFICE, OFFICE, ""),
    f("odt", "OpenDocument Text", &["odt", "fodt"], Document, OFFICE, OFFICE, ""),
    f("rtf", "Rich Text", &["rtf"], Document, OFFICE, OFFICE, ""),
    f("txt", "Plain text", &["txt", "text", "log"], Document, MARKUP, MARKUP, ""),
    f("md", "Markdown", &["md", "markdown", "mdown"], Document, MARKDOWN, MARKDOWN, ""),
    f("html", "HTML", &["html", "htm", "xhtml"], Document, MARKUP, MARKUP, ""),
    f(
        "epub",
        "EPUB ebook",
        &["epub"],
        Document,
        AnyOf(&[Tool::Pandoc]),
        AnyOf(&[Tool::Pandoc]),
        "",
    ),
    f("fb2", "FictionBook", &["fb2"], Document, AnyOf(&[Tool::Pandoc]), AnyOf(&[Tool::Pandoc]), ""),
    f(
        "tex",
        "LaTeX",
        &["tex", "latex"],
        Document,
        AnyOf(&[Tool::Pandoc]),
        AnyOf(&[Tool::Pandoc]),
        "",
    ),
    f(
        "rst",
        "reStructuredText",
        &["rst"],
        Document,
        AnyOf(&[Tool::Pandoc]),
        AnyOf(&[Tool::Pandoc]),
        "",
    ),
    f("pptx", "PowerPoint (pptx)", &["pptx"], Document, OFFICE, OFFICE, ""),
    f("ppt", "PowerPoint 97-2003", &["ppt"], Document, OFFICE, OFFICE, ""),
    f("odp", "OpenDocument Presentation", &["odp"], Document, OFFICE, OFFICE, ""),
    f("xlsx", "Excel (xlsx)", &["xlsx"], Document, OFFICE, OFFICE, ""),
    f("xls", "Excel 97-2003", &["xls"], Document, OFFICE, OFFICE, ""),
    f("ods", "OpenDocument Spreadsheet", &["ods", "fods"], Document, OFFICE, OFFICE, ""),
    f("csv", "CSV", &["csv"], Document, OFFICE, OFFICE, ""),
    f("tsv", "TSV", &["tsv", "tab"], Document, OFFICE, OFFICE, ""),
    f(
        "json_doc",
        "JSON / YAML data",
        &["json", "yaml", "yml"],
        Document,
        AnyOf(&[Tool::Pandoc]),
        AnyOf(&[Tool::Pandoc]),
        "Structured text via Pandoc",
    ),
    // ------------------------------------------------------------- subtitle
    f("srt", "SubRip", &["srt"], Subtitle, Bundled, Bundled, "Most widely supported"),
    f("vtt", "WebVTT", &["vtt"], Subtitle, Bundled, Bundled, "HTML5 <track>"),
    f(
        "ass",
        "Advanced SubStation",
        &["ass", "ssa"],
        Subtitle,
        Bundled,
        Bundled,
        "Styled subtitles",
    ),
    // FFmpeg ships `microdvd`/`subviewer` *decoders* only - there has never been an encoder for
    // either, so a `.sub` output dies with "Output file does not contain any stream".
    f("sub", "MicroDVD / SubViewer", &["sub", "sbv"], Subtitle, Bundled, Unsupported, "Read only"),
    f("lrc", "LRC lyrics", &["lrc"], Subtitle, Bundled, Bundled, ""),
    // The mirror image: FFmpeg has a TTML encoder + muxer but no demuxer or decoder
    // (trac #4859 is still open), so TTML can only ever be a target.
    f("ttml", "TTML / DFXP", &["ttml", "dfxp"], Subtitle, Unsupported, Bundled, "Write only"),
    // ---------------------------------------------------------------- flash
    f(
        "swf",
        "Flash movie (SWF)",
        &["swf"],
        Flash,
        AnyOf(&[Tool::Ruffle]),
        Unsupported,
        "Rendered by Ruffle, then encoded to video/GIF",
    ),
];

/// Every format the app knows, in catalog order.
pub fn catalog() -> &'static [Format] {
    CATALOG
}

pub fn by_id(id: &str) -> Option<&'static Format> {
    CATALOG.iter().find(|f| f.id == id)
}

/// Detect the format of a path from its extension.
///
/// Output-only pseudo-formats (`pdf_page`, `alac`) deliberately re-use an extension that a real
/// input format already owns, so detection only ever looks at formats we can *read* - which is
/// also the only question being asked here. `readable_extensions_are_unique` keeps that honest.
pub fn by_extension(ext: &str) -> Option<&'static Format> {
    let ext = ext.trim_start_matches('.').to_ascii_lowercase();
    CATALOG.iter().find(|f| f.read.is_supported() && f.matches_extension(&ext))
}

pub fn by_path(path: &std::path::Path) -> Option<&'static Format> {
    path.extension().and_then(|e| e.to_str()).and_then(by_extension)
}

/// All formats that can be read for a category.
pub fn readable_in(category: Category) -> Vec<&'static Format> {
    CATALOG.iter().filter(|f| f.category == category && f.read.is_supported()).collect()
}

/// All formats that can be produced for a category.
pub fn writable_in(category: Category) -> Vec<&'static Format> {
    CATALOG.iter().filter(|f| f.category == category && f.write.is_supported()).collect()
}

/// Number of distinct input extensions we accept - used in the UI ("1 800+ conversions").
pub fn readable_extension_count() -> usize {
    CATALOG.iter().filter(|f| f.read.is_supported()).map(|f| f.extensions.len()).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_format_has_at_least_one_extension_and_is_useful() {
        for f in CATALOG {
            assert!(!f.extensions.is_empty(), "{} has no extension", f.id);
            assert!(
                f.read.is_supported() || f.write.is_supported(),
                "{} is neither readable nor writable",
                f.id
            );
        }
    }

    #[test]
    fn ids_are_unique() {
        let mut ids: Vec<_> = CATALOG.iter().map(|f| f.id).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(before, ids.len(), "duplicate format id in catalog");
    }

    #[test]
    fn extensions_are_lowercase_and_dotless() {
        for f in CATALOG {
            for e in f.extensions {
                assert!(!e.starts_with('.'), "{}: {} starts with a dot", f.id, e);
                assert_eq!(*e, e.to_ascii_lowercase(), "{}: {} not lowercase", f.id, e);
            }
        }
    }

    /// Two readable formats claiming one extension would make input detection order-dependent.
    #[test]
    fn readable_extensions_are_unique() {
        let mut seen: Vec<(&str, &str)> = Vec::new();
        for f in CATALOG.iter().filter(|f| f.read.is_supported()) {
            for e in f.extensions {
                if let Some((other, _)) = seen.iter().find(|(_, ext)| ext == e) {
                    panic!("`.{e}` is claimed by both `{other}` and `{}`", f.id);
                }
                seen.push((f.id, e));
            }
        }
    }

    #[test]
    fn output_only_pseudo_formats_never_win_input_detection() {
        assert_eq!(by_extension("pdf").unwrap().id, "pdf");
        assert_eq!(by_extension("m4a").unwrap().id, "m4a");
        // ...but they are still real, writable catalog entries.
        assert!(by_id("pdf_page").unwrap().write.is_supported());
        assert!(by_id("alac").unwrap().write.is_supported());
    }

    #[test]
    fn detects_common_inputs() {
        assert_eq!(by_extension("MOV").unwrap().id, "mov");
        assert_eq!(by_extension(".Mp3").unwrap().id, "mp3");
        assert_eq!(by_extension("cr3").unwrap().id, "raw_camera");
        assert_eq!(by_extension("pdf").unwrap().category, Category::Document);
        assert_eq!(by_extension("swf").unwrap().category, Category::Flash);
        assert!(by_extension("zip").is_none());
    }

    #[test]
    fn catalog_is_broad() {
        assert!(readable_extension_count() > 150, "catalog unexpectedly small");
        for c in Category::ALL {
            if c == Category::Flash {
                continue; // input-only category
            }
            assert!(!writable_in(c).is_empty(), "{} has no output format", c.id());
        }
    }
}
