import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import dialogPolyfill from "dialog-polyfill";
import { X } from "lucide-react";
import { cropApplies, cropError } from "../lib/crop";
import type { CropSettings } from "../lib/types";
import { convertibleRows, useStore } from "../state/store";

export function CropDialog(): React.JSX.Element {
  const dialog = useRef<HTMLDialogElement>(null);
  const open = useStore((s) => s.cropOpen);
  const setOpen = useStore((s) => s.setCropOpen);
  const start = useStore((s) => s.startCrop);
  const files = useStore((s) => s.files);
  const order = useStore((s) => s.order);
  const rows = convertibleRows({ files, order });
  const categories = new Set(rows.map((row) => row.info.category));
  const [image, setImage] = useState(true);
  const [media, setMedia] = useState(true);
  const [document, setDocument] = useState(true);
  const [values, setValues] = useState({
    x: "0", y: "0", width: "100", height: "100", start: "0", length: "60", end: "60",
    docStart: "1", docEnd: "5",
  });
  const [mode, setMode] = useState<"length" | "end">("length");
  const [unit, setUnit] = useState<"pages" | "words">("pages");
  const hasImage = categories.has("image");
  const hasMedia = categories.has("video") || categories.has("audio") || categories.has("flash");
  const hasDocument = categories.has("document");
  const number = (key: keyof typeof values): number => values[key].trim() === "" ? NaN : Number(values[key]);
  const crop: CropSettings = {
    image: image && hasImage ? { x: number("x"), y: number("y"), width: number("width"), height: number("height") } : null,
    media: media && hasMedia ? { start_secs: number("start"),
      length_secs: mode === "length" ? number("length") : number("end") - number("start") } : null,
    document: document && hasDocument ? { unit, start: number("docStart"), end: number("docEnd") } : null,
  };
  const error = cropError(crop);
  const count = rows.filter((row) => cropApplies(crop, row.info.category)).length;

  useEffect(() => {
    const element = dialog.current;
    if (!element) return;
    if (typeof element.showModal !== "function") {
      dialogPolyfill.registerDialog(element);
    }
    if (!open) return;
    const opener = window.document.activeElement;
    element.showModal();
    return () => {
      if (element.open) element.close();
      if (opener instanceof HTMLElement && opener.isConnected) opener.focus({ preventScroll: true });
    };
  }, [open, setOpen]);
  useEffect(() => () => useStore.getState().setCropOpen(false), []);

  const field = (key: keyof typeof values, label: string, min: number, step = "1"): React.JSX.Element => (
    <label className="crop__field">
      <span>{label}</span>
      <input type="number" min={min} step={step} value={values[key]}
        onChange={(event) => setValues((old) => ({ ...old, [key]: event.target.value }))} />
    </label>
  );

  // A body portal keeps the legacy dialog out of the action bar's animated stacking context.
  return createPortal(
    <dialog ref={dialog} className="crop-dialog" aria-labelledby="crop-title"
      aria-modal="true"
      onKeyDown={(event) => {
        if (event.key !== "Tab") return;
        const controls = Array.from(event.currentTarget.querySelectorAll<HTMLElement>(
          "button:not(:disabled), input:not(:disabled), select:not(:disabled)"
        ));
        if (controls.length === 0) return;
        // macOS can skip buttons in native Tab navigation, including the final
        // action. Traverse explicitly so both directions remain inside the dialog.
        event.preventDefault();
        const index = controls.findIndex((control) => control === window.document.activeElement);
        const next = index < 0
          ? (event.shiftKey ? controls.length - 1 : 0)
          : (index + (event.shiftKey ? -1 : 1) + controls.length) % controls.length;
        controls[next]?.focus();
      }}
      onCancel={() => setOpen(false)} onClose={() => setOpen(false)}
      onClick={(event) => { if (event.target === event.currentTarget) {
        const r = event.currentTarget.getBoundingClientRect();
        if (event.clientX < r.left || event.clientX > r.right || event.clientY < r.top || event.clientY > r.bottom) setOpen(false);
      } }}>
      <form onSubmit={(event) => {
        event.preventDefault();
        if (error || count === 0) return;
        setOpen(false);
        void start(crop);
      }}>
        <header className="crop__header">
          <h2 id="crop-title">Crop &amp; convert</h2>
          <button type="button" className="iconbutton" title="Close crop options" aria-label="Close crop options"
            onClick={() => setOpen(false)}><X size={16} aria-hidden="true" /></button>
        </header>
        <div className="crop__body">
        {hasImage && <section className="crop__section">
          <label className="crop__toggle"><input type="checkbox" checked={image}
            onChange={(e) => setImage(e.target.checked)} /> Images</label>
          <fieldset disabled={!image} className="crop__fields crop__fields--four">
            {field("x", "Left X (px)", 0)}
            {field("y", "Top Y (px)", 0)}
            {field("width", "Width (px)", 1)}
            {field("height", "Height (px)", 1)}
          </fieldset>
        </section>}
        {hasMedia && <section className="crop__section">
          <label className="crop__toggle"><input type="checkbox" checked={media}
            onChange={(e) => setMedia(e.target.checked)} /> Audio / video</label>
          <fieldset disabled={!media} className="crop__fields">
            {field("start", "Start (seconds)", 0, "any")}
            <label className="crop__field"><span>Range</span><select aria-label="Range" value={mode}
              onChange={(e) => setMode(e.target.value as "length" | "end")}>
              <option value="length">Duration</option><option value="end">End time</option>
            </select></label>
            {mode === "length" ? field("length", "Duration (seconds)", 0, "any") : field("end", "End (seconds)", 0, "any")}
          </fieldset>
          {crop.media && Number.isFinite(crop.media.length_secs) && crop.media.length_secs > 0 &&
            <output className="crop__range">{crop.media.start_secs}s to {Number((crop.media.start_secs + crop.media.length_secs).toFixed(6))}s · {Number(crop.media.length_secs.toFixed(6))}s kept</output>}
        </section>}
        {hasDocument && <section className="crop__section">
          <label className="crop__toggle"><input type="checkbox" checked={document}
            onChange={(e) => setDocument(e.target.checked)} /> Documents</label>
          <fieldset disabled={!document} className="crop__fields">
            <label className="crop__field"><span>Unit</span><select aria-label="Unit" value={unit} onChange={(e) => {
              setUnit(e.target.value as "pages" | "words");
              setValues((old) => ({ ...old, docStart: "1", docEnd: e.target.value === "words" ? "10000" : "5" }));
            }}><option value="pages">Pages</option><option value="words">Words</option></select></label>
            {field("docStart", "From (inclusive)", 1)}
            {field("docEnd", "Through (inclusive)", 1)}
          </fieldset>
          <p className="crop__range">{unit === "words"
            ? "Extracted text only; original formatting is not retained. Words are separated by whitespace."
            : "Rendered pages; original pagination may change when converting the selected PDF to an editable format."}</p>
        </section>}
        <div className="crop__status" aria-live="polite">
          {error ? <span role="alert">{error}</span> : <span>{count} files selected for this batch{rows.length > count ? ` · ${rows.length - count} unchanged in queue` : ""}</span>}
        </div>
        </div>
        <footer className="crop__actions">
          <button type="button" className="button" onClick={() => setOpen(false)}>Cancel</button>
          <button type="submit" className="button" disabled={error !== null || count === 0}>Crop &amp; convert {count} files</button>
        </footer>
      </form>
    </dialog>, window.document.body
  );
}
