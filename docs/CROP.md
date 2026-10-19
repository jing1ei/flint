# Crop & Convert

Choose output formats as usual, then use the arrow beside Convert and select **Crop & convert**.
Enter one selection per input category and start the batch. No previews are opened.
Unchecked categories remain queued. Subtitles have no crop mode.

## Numeric Ranges

| Input | Fields | Example |
| --- | --- | --- |
| Image | Left X, top Y, width, height, in whole pixels | `0, 0, 100, 100` keeps a top-left 100x100 rectangle |
| Audio/video (including links and Flash) | Start seconds and duration, or start and end seconds | Start `10`, end `50` keeps 40 seconds |
| Document | Pages or words, from and through (both inclusive, starting at 1) | Pages `1-5`; words `1-10000` or `2000-10000` |

Image dimensions must be positive and at most 100000; offsets are 0-100000.
Media values may include fractions, with a positive duration and an end within 24 hours.
Document endpoints are whole numbers from 1 to 10000000.

The selection is applied before normal conversion options such as image resizing.
It overrides the ordinary time trim for this batch only. Packet-copy codecs are changed to
automatic encoding so media cuts can be made between keyframes. Media and document ranges
ending past the source stop at its end; starts beyond the source fail with an explanation.
An image rectangle extending outside a source fails rather than being silently shifted.

## Document Fidelity

- **Pages:** PDF pages are selected directly with lopdf. Other document formats are first
  rendered to PDF using an available converter. PDF-to-PDF selection does not require an
  office helper. Converting the selected PDF to editable output can change pagination and
  formatting; some output formats need additional helpers or are unsupported from PDF.
- **Words:** documents are extracted to text, then whitespace-delimited words are selected.
  Original formatting, images, tables and pagination are not retained. TXT-to-TXT selection
  needs no helper. Other targets use escaped plain text as their conversion input.
- Scanned PDFs need OCR before word selection; encrypted PDFs must be unlocked.
- PDF page parsing and extracted text files are limited to 128 MB.
- Image output from a page range requires Poppler or ImageMagick; the ordinary first-page-only
  setting is overridden so selected pages are not silently discarded.

## Images And Batch Behavior

Still images use a lossless intermediate. Supported animated inputs are cropped frame by frame;
helper-decoded animations that cannot retain all frames are refused with a conversion suggestion.
Some image formats need the same optional decoders as ordinary conversion.

Source files are never edited by selection preparation. Results use the existing destination,
conflict and timestamp settings. Invalid or unsupported files fail individually while other
files continue. Stop uses the existing cancellation path and removes job scratch files.

Selections are not saved in application preferences. Retry keeps the failed attempt's selection;
ordinary Convert starts without it. Failed rows with different selections must be retried
separately or submitted through a new Crop & convert batch with one shared selection.

## Verification

- `npm test`: numeric validation, transient settings, retry and modal isolation.
- `python3 scripts/ui_crop.py`: split menu, fields, keyboard focus and mixed-category submission.
- `cargo test -p convert-core --test crop_batch`: real image/media output checks when FFmpeg and
  ffprobe are available, plus helper-free word batches.
- `cargo test -p convert-core crop::tests`: PDF page selection, text selection and cancellation.

macOS CI/release runs the workspace tests with bundled FFmpeg. The Windows build script also runs
`crop_batch` after downloading its sidecars. Browser demo tests validate controls, not real
image or document transformations.
