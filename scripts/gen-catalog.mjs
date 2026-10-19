// Generator for src/lib/mock-catalog.ts — the data the browser preview needs so the mock backend
// answers `get_catalog` / `inspect_files` exactly like the real Rust side does.
//
//   node scripts/gen-catalog.mjs            # rewrites src/lib/mock-catalog.ts
//   node scripts/gen-catalog.mjs --check    # fail on a stale catalog, writes nothing
//
// Everything factual is read out of the Rust sources, never restated here:
//
//   format.rs   const CATALOG            the format table (id, name, extensions, category, support)
//               impl Tool                id / label / install_hint / is_bundled
//               impl Category            ALL order + id / label
//               const SIPS_OR_MAGICK …   the `Support` aliases the catalog uses
//   tools.rs    ALL_TOOLS                which helpers exist, in UI order
//   package.rs  ALL_PACKAGES             what a *user* installs, and which binaries each provides
//   plan.rs     default_target_for       per-category default + one-click chips
//               suggested_targets_for
//   settings.rs impl Preset              ALL order + id / label / description
//
// Only the *fixture* half is local: which helpers this imaginary dev machine has installed and
// where they live. Two earlier bugs are the reason for that rule — the catalog was moved behind
// `catalog()` and the marker this script sliced on (`pub const CATALOG`) stopped existing, and
// Ghostscript was dropped from `ALL_TOOLS` while a copy of the tool table here kept it alive.
// Every lookup below throws loudly rather than silently emitting a smaller catalog.
import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const CORE = path.join(ROOT, "crates/convert-core/src");
const OUT = path.join(ROOT, "src/lib/mock-catalog.ts");
const CHECK_ONLY = process.argv.includes("--check");

const rust = (name) => readFileSync(path.join(CORE, name), "utf8");
const FORMAT_RS = rust("format.rs");
const TOOLS_RS = rust("tools.rs");
const PACKAGE_RS = rust("package.rs");
const PLAN_RS = rust("plan.rs");
const SETTINGS_RS = rust("settings.rs");

// ---------------------------------------------------------------- tiny Rust readers -------------

/** Body of a bracketed literal, found by balancing from `openIndex` (string-aware). */
function balancedAt(source, openIndex, open, close) {
  let depth = 0;
  let inStr = false;
  for (let i = openIndex; i < source.length; i += 1) {
    const ch = source[i];
    if (inStr) {
      if (ch === "\\") i += 1;
      else if (ch === '"') inStr = false;
      continue;
    }
    if (ch === '"') inStr = true;
    else if (ch === open) depth += 1;
    else if (ch === close) {
      depth -= 1;
      if (depth === 0) return source.slice(openIndex + 1, i);
    }
  }
  throw new Error("unbalanced literal");
}

const stripComments = (s) => s.replace(/\/\/[^\n]*/g, "").replace(/\/\*[\s\S]*?\*\//g, "");

/** Body of `impl Type { … }` — needed because several enums here own an `id()` and a `label()`. */
function implBody(source, typeName) {
  const at = source.search(new RegExp(`impl\\s+${typeName}\\s*\\{`));
  if (at < 0) throw new Error(`cannot find impl ${typeName}`);
  return balancedAt(source, source.indexOf("{", at), "{", "}");
}

/** Body of `fn name(…) -> … { … }`, searched inside `scope`. */
function fnBody(scope, name) {
  const at = scope.search(new RegExp(`fn\\s+${name}\\s*\\(`));
  if (at < 0) throw new Error(`cannot find fn ${name}`);
  return stripComments(balancedAt(scope, scope.indexOf("{", at), "{", "}"));
}

/** `Enum::A | Enum::B => <rhs>,` arms of a match, as [variants, rhs] pairs. */
function matchArms(body, enumName) {
  const arms = [];
  const re = new RegExp(
    `((?:${enumName}::\\w+\\s*\\|\\s*)*${enumName}::\\w+)\\s*=>\\s*([\\s\\S]*?),\\s*(?=${enumName}::|\\}|$)`,
    "g",
  );
  for (const m of body.matchAll(re)) {
    const variants = [...m[1].matchAll(new RegExp(`${enumName}::(\\w+)`, "g"))].map((v) => v[1]);
    arms.push([variants, m[2].trim()]);
  }
  if (arms.length === 0) throw new Error(`no ${enumName} match arms found`);
  return arms;
}

/** Variant -> string literal, from a `const fn x(self) -> &'static str` accessor. */
function stringAccessor(source, enumName, fn) {
  const table = {};
  for (const [variants, rhs] of matchArms(fnBody(implBody(source, enumName), fn), enumName)) {
    for (const v of variants) table[v] = JSON.parse(rhs);
  }
  return table;
}

/** Variant order of a `const ALL…` array. */
function variantOrder(source, constName, enumName) {
  const at = source.search(new RegExp(`const\\s+${constName}\\s*:`));
  if (at < 0) throw new Error(`cannot find const ${constName}`);
  const body = balancedAt(source, source.indexOf("[", source.indexOf("=", at)), "[", "]");
  const order = [...body.matchAll(new RegExp(`${enumName}::(\\w+)`, "g"))].map((m) => m[1]);
  if (order.length === 0) throw new Error(`const ${constName} is empty`);
  return order;
}

const pick = (table, key, what) => {
  const v = table[key];
  if (v === undefined) throw new Error(`${what} has no entry for ${key}`);
  return v;
};

// ---------------------------------------------------------------- helper tools --------------------

const TOOL_ORDER = variantOrder(TOOLS_RS, "ALL_TOOLS", "Tool");
const TOOL_ID = stringAccessor(FORMAT_RS, "Tool", "id");
const TOOL_LABEL = stringAccessor(FORMAT_RS, "Tool", "label");
const TOOL_HINT = stringAccessor(FORMAT_RS, "Tool", "install_hint");
const BUNDLED = new Set(
  [...fnBody(implBody(FORMAT_RS, "Tool"), "is_bundled").matchAll(/Tool::(\w+)/g)].map((m) => m[1]),
);

// Fixture, not fact: a believable dev machine — bundled engine plus a couple of helpers installed,
// the rest missing, so the preview shows both the enabled and the greyed-out paths. yt-dlp is one of
// the present ones, so the pasted-link path works in the preview by default; `?missing=yt-dlp` is
// how the "these need yt-dlp" state gets looked at.
const AVAILABLE = new Set([
  "Ffmpeg",
  "Ffprobe",
  "LibreOffice",
  "Magick",
  "PdfToPpm",
  "Sips",
  "YtDlp",
]);
const MOCK_PATH = {
  Ffmpeg: "/Applications/Flint.app/Contents/MacOS/ffmpeg",
  Ffprobe: "/Applications/Flint.app/Contents/MacOS/ffprobe",
  LibreOffice: "/Applications/LibreOffice.app/Contents/MacOS/soffice",
  Magick: "/opt/homebrew/bin/magick",
  PdfToPpm: "/opt/homebrew/bin/pdftoppm",
  Sips: "/usr/bin/sips",
  YtDlp: "/opt/homebrew/bin/yt-dlp",
};
for (const name of [...AVAILABLE, ...Object.keys(MOCK_PATH)]) {
  if (!TOOL_ORDER.includes(name)) throw new Error(`fixture names a tool Rust dropped: ${name}`);
}

const tools = TOOL_ORDER.map((t) => ({
  id: pick(TOOL_ID, t, "Tool::id"),
  label: pick(TOOL_LABEL, t, "Tool::label"),
  bundled: BUNDLED.has(t),
  available: AVAILABLE.has(t),
  path: MOCK_PATH[t] ?? null,
  install_hint: pick(TOOL_HINT, t, "Tool::install_hint"),
}));

// ---------------------------------------------------------------- packages ------------------------

// `pub const POPPLER: Package = Package { id: "poppler", name: "Poppler", tools: &[…] };`
//
// A package is what a *person* installs and a tool is what we spawn, so this table is what turns a
// format's support list into words: Poppler's three binaries are one name, said once. Read from Rust
// like everything else here — a sixth package, or a fourth Poppler binary, is one line in
// package.rs and nothing in this file.
const PACKAGE_DECL = {};
for (const m of PACKAGE_RS.matchAll(/pub const (\w+)\s*:\s*Package\s*=\s*Package\s*\{/g)) {
  const body = stripComments(
    balancedAt(PACKAGE_RS, m.index + m[0].length - 1, "{", "}"),
  );
  const id = /id\s*:\s*"([^"]*)"/.exec(body);
  const name = /name\s*:\s*"([^"]*)"/.exec(body);
  const members = [...body.matchAll(/Tool::(\w+)/g)].map((t) => t[1]);
  if (!id || !name) throw new Error(`package.rs const ${m[1]} has no id/name`);
  if (members.length === 0) throw new Error(`package ${id[1]} provides no tools`);
  for (const t of members) if (!TOOL_ORDER.includes(t)) throw new Error(`unknown Tool::${t}`);
  PACKAGE_DECL[m[1]] = { id: id[1], name: name[1], tools: members };
}

const ALL_PACKAGES_LITERAL = (() => {
  const at = PACKAGE_RS.search(/const\s+ALL_PACKAGES\s*:/);
  if (at < 0) throw new Error("cannot find const ALL_PACKAGES in package.rs");
  return balancedAt(PACKAGE_RS, PACKAGE_RS.indexOf("[", PACKAGE_RS.indexOf("=", at)), "[", "]");
})();
const packages = [...ALL_PACKAGES_LITERAL.matchAll(/&(\w+)/g)].map((m) =>
  pick(PACKAGE_DECL, m[1], "package.rs"),
);
if (packages.length === 0) throw new Error("const ALL_PACKAGES is empty");

// The invariants `package.rs`'s own tests hold, restated where a *generated* file could break them.
for (const t of TOOL_ORDER) {
  const owners = packages.filter((p) => p.tools.includes(t)).map((p) => p.id);
  if (owners.length > 1) throw new Error(`${t} is claimed by ${owners.join(", ")}`);
}

const packageOfTool = (variant) => packages.find((p) => p.tools.includes(variant));

/** `Tool::user_facing_name`: the package's name, or — for a tool nothing installs — its own label. */
const userFacingName = (variant) =>
  packageOfTool(variant)?.name ?? pick(TOOL_LABEL, variant, "Tool::label");

for (const t of TOOL_ORDER) {
  if (userFacingName(t).includes("pdfto")) throw new Error(`${t} names a binary to a user`);
}

// ---------------------------------------------------------------- the catalog table ---------------

// `const CATALOG: &[Format] = &[ … ];` — private, `catalog()` is the only way in. rustfmt spreads
// most entries over several lines, so walk balanced parens instead of assuming one entry per line.
const decl = /const CATALOG\s*:\s*&\[Format\]\s*=\s*&(\[)/.exec(FORMAT_RS);
if (!decl) throw new Error("cannot find `const CATALOG: &[Format] = &[` in format.rs");
const catalogBody = stripComments(
  balancedAt(FORMAT_RS, decl.index + decl[0].lastIndexOf("["), "[", "]"),
);

// `const SIPS_OR_MAGICK: Support = AnyOf(&[Tool::Sips, Tool::Magick]);` -> alias table.
const ALIASES = {};
for (const m of FORMAT_RS.matchAll(/const\s+([A-Z][A-Z0-9_]*)\s*:\s*Support\s*=\s*([^;]+);/g)) {
  ALIASES[m[1]] = m[2].trim();
}

function splitArgs(s) {
  const out = [];
  let depth = 0,
    inStr = false,
    cur = "";
  for (const ch of s) {
    if (inStr) {
      cur += ch;
      if (ch === '"') inStr = false;
      continue;
    }
    if (ch === '"') {
      inStr = true;
      cur += ch;
      continue;
    }
    if ("([{".includes(ch)) depth++;
    if (")]}".includes(ch)) depth--;
    if (ch === "," && depth === 0) {
      out.push(cur.trim());
      cur = "";
      continue;
    }
    cur += ch;
  }
  if (cur.trim()) out.push(cur.trim());
  return out;
}

function parseSupport(raw) {
  const s = ALIASES[raw] ?? raw;
  if (s === "Bundled") return { kind: "bundled", tools: [] };
  if (s === "Unsupported") return { kind: "unsupported", tools: [] };
  const m = s.match(/AnyOf\(&\[(.*)\]\)/s);
  if (!m) throw new Error("unknown support: " + raw);
  const list = m[1]
    .split(",")
    .map((t) => t.trim().replace("Tool::", ""))
    .filter(Boolean);
  for (const t of list) if (!TOOL_ORDER.includes(t)) throw new Error(`unknown Tool::${t}`);
  return { kind: "any_of", tools: list };
}

const formats = [];
for (let i = 0; i < catalogBody.length; i += 1) {
  if (catalogBody[i] !== "f" || catalogBody[i + 1] !== "(") continue;
  if (i > 0 && /[\w:]/.test(catalogBody[i - 1])) continue; // tail of a longer identifier
  const inner = balancedAt(catalogBody, i + 1, "(", ")");
  i += inner.length + 1;
  const [id, name, exts, category, read, write, notes] = splitArgs(inner);
  formats.push({
    id: JSON.parse(id),
    name: JSON.parse(name),
    extensions: splitArgs(exts.replace(/^&\[/, "").replace(/\]$/, "")).map((e) => JSON.parse(e)),
    category: category.trim(),
    read: parseSupport(read),
    write: parseSupport(write),
    notes: JSON.parse(notes),
  });
}
if (formats.length === 0) throw new Error("parsed 0 formats — the CATALOG literal shape changed");

// ---------------------------------------------------------------- categories & presets ------------

const CAT_ORDER = variantOrder(implBody(FORMAT_RS, "Category"), "ALL", "Category");
const CAT_ID = stringAccessor(FORMAT_RS, "Category", "id");
const CAT_LABEL = stringAccessor(FORMAT_RS, "Category", "label");

const DEFAULT_TARGET = {};
for (const [variants, rhs] of matchArms(fnBody(PLAN_RS, "default_target_for"), "Category")) {
  for (const v of variants) DEFAULT_TARGET[v] = JSON.parse(rhs);
}
const SUGGESTED = {};
for (const [variants, rhs] of matchArms(fnBody(PLAN_RS, "suggested_targets_for"), "Category")) {
  const list = [...rhs.matchAll(/"([^"]*)"/g)].map((m) => m[1]);
  for (const v of variants) SUGGESTED[v] = list;
}

// `output_extension` in plan.rs: APNG is the single exception, everything else uses extensions[0].
const APNG_ONLY = /"apng"\s*=>\s*"png"/.test(fnBody(PLAN_RS, "output_extension"));
if (!APNG_ONLY) throw new Error("plan::output_extension no longer special-cases apng only");
const outputExtension = (f) => (f.id === "apng" ? "png" : f.extensions[0]);

const supportAvailable = (s) =>
  s.kind === "bundled" || (s.kind === "any_of" && s.tools.some((t) => AVAILABLE.has(t)));

/** `lib.rs::format_view`: the helpers a format needs, named as they are installed, each named once. */
const needsOf = (support) => {
  const names = [];
  for (const t of support.tools) {
    const name = userFacingName(t);
    if (!names.includes(name)) names.push(name);
  }
  return names;
};

const view = (f, support) => ({
  id: f.id,
  name: f.name,
  extension: outputExtension(f),
  extensions: f.extensions,
  notes: f.notes,
  available: supportAvailable(support),
  needs: needsOf(support),
});

const unknown = [...new Set(formats.map((f) => f.category))].filter((c) => !CAT_ORDER.includes(c));
if (unknown.length) throw new Error("format.rs uses categories not in Category::ALL: " + unknown);

const categories = CAT_ORDER.map((variant) => ({
  id: pick(CAT_ID, variant, "Category::id"),
  label: pick(CAT_LABEL, variant, "Category::label"),
  default_target: pick(DEFAULT_TARGET, variant, "default_target_for"),
  suggested_targets: pick(SUGGESTED, variant, "suggested_targets_for"),
  inputs: formats
    .filter((f) => f.category === variant && f.read.kind !== "unsupported")
    .map((f) => view(f, f.read)),
  outputs: formats
    .filter((f) => f.category === variant && f.write.kind !== "unsupported")
    .map((f) => view(f, f.write)),
}));

const presets = variantOrder(implBody(SETTINGS_RS, "Preset"), "ALL", "Preset").map((p) => ({
  id: pick(stringAccessor(SETTINGS_RS, "Preset", "id"), p, "Preset::id"),
  label: pick(stringAccessor(SETTINGS_RS, "Preset", "label"), p, "Preset::label"),
  description: pick(stringAccessor(SETTINGS_RS, "Preset", "description"), p, "Preset::description"),
}));

// `format::readable_extension_count()` — the "N input extensions" figure on the drop zone.
const inputExtensionCount = formats
  .filter((f) => f.read.kind !== "unsupported")
  .reduce((n, f) => n + f.extensions.length, 0);

const catalog = { categories, presets, tools, input_extension_count: inputExtensionCount };

// Extension -> format id map, so the mock can identify dropped file names like the Rust side does.
// `format::by_extension` scans the catalog in order and takes the first *readable* match; images
// deliberately re-use extensions other categories own, hence the same preference here.
const byExtension = {};
for (const f of formats) {
  if (f.read.kind === "unsupported") continue;
  for (const ext of f.extensions) {
    const existing = byExtension[ext];
    const heldByImage = formats.find((x) => x.id === existing)?.category === "Image";
    if (existing === undefined || (heldByImage && f.category !== "Image")) byExtension[ext] = f.id;
  }
}

const meta = {};
for (const f of formats) {
  meta[f.id] = { name: f.name, category: pick(CAT_ID, f.category, "Category::id") };
}

// Format id -> the helper *binaries* each direction declares (`Support::AnyOf`, in the planner's
// order of preference).
//
// `needs` above is the package names a person installs, which is what the real `catalog_view` sends
// and therefore what the UI must render. But "can this machine do it?" is a question about binaries —
// a Mac with `pdftoppm` and no `pdftohtml` has a partly-installed Poppler and can open a PDF but not
// write HTML — and expanding a package name back into its members would answer that too generously.
// So the mock gets the binary-level list here and the user-facing names there.
const formatHelpers = {};
for (const f of formats) {
  const ids = (support) =>
    support.kind === "any_of" ? support.tools.map((t) => pick(TOOL_ID, t, "Tool::id")) : [];
  const read = ids(f.read);
  const write = ids(f.write);
  if (read.length > 0 || write.length > 0) formatHelpers[f.id] = { read, write };
}

// The two lookup maps are emitted with their keys sorted: the file is a generated artefact that
// lives in git, so a regeneration should produce a stable diff rather than reshuffling 300 lines
// whenever a format is inserted in the middle of the catalog. `MOCK_CATALOG` keeps catalog order,
// which is the order the UI renders.
const sortedJson = (obj) =>
  JSON.stringify(
    obj,
    (_k, v) =>
      v && typeof v === "object" && !Array.isArray(v)
        ? Object.fromEntries(Object.keys(v).sort().map((k) => [k, v[k]]))
        : v,
    2,
  );

const banner = `// Generated from crates/convert-core/src/format.rs — do not edit by hand.
// Data only: consumed by src/lib/mock.ts to make the browser preview behave like the real app.
import type { CatalogView } from "./types";

`;

const src =
  banner +
  `export const MOCK_CATALOG: CatalogView = ${JSON.stringify(catalog, null, 2)};\n\n` +
  `/** Lower-case extension → format id, mirroring \`format::by_extension\`. */\n` +
  `export const EXTENSION_TO_FORMAT: Readonly<Record<string, string>> = ${sortedJson(byExtension)};\n\n` +
  `/** Format id → display name + category, for the faked \`inspect_files\`. */\n` +
  `export const FORMAT_META: Readonly<Record<string, { name: string; category: string }>> = ${sortedJson(meta)};\n\n` +
  `/**\n` +
  ` * Format id → the helper *binaries* each direction declares, in the planner's order.\n` +
  ` *\n` +
  ` * \`FormatView.needs\` above is the *package* names a person installs ("Poppler", once, for three\n` +
  ` * binaries); this is the executable-level list, which is what answers "can this machine do it?".\n` +
  ` */\n` +
  `export const FORMAT_HELPERS: Readonly<\n` +
  `  Record<string, { read: string[]; write: string[] }>\n` +
  `> = ${sortedJson(formatHelpers)};\n`;

if (CHECK_ONLY) {
  if (readFileSync(OUT, "utf8") !== src) {
    console.error("src/lib/mock-catalog.ts is stale. Run npm run gen:catalog.");
    process.exitCode = 1;
  }
} else writeFileSync(OUT, src);
console.log(
  `formats=${formats.length} inputExt=${inputExtensionCount} ` +
    `outputs=${categories.reduce((n, c) => n + c.outputs.length, 0)} tools=${tools.length} ` +
    `packages=${packages.length} presets=${presets.length} bytes=${src.length}` +
    (CHECK_ONLY ? "  (--check: nothing written)" : `  -> ${path.relative(ROOT, OUT)}`),
);
