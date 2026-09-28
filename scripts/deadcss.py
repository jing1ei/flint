"""Precise dead-CSS probe.

    python3 scripts/deadcss.py        # runs from anywhere; paths derive from this file

Left side: every class token that appears in a CSS *selector* in src/styles.css.
Right side: every class token any component can actually put in the DOM.

The right side is deliberately narrow: only *string literals* count. An earlier version collected
every identifier inside a `className={…}` expression, so `className={`pill${phase === "running" ?
" pill--stop" : ""}`}` in ActionBar.tsx reported `.phase` and `.running` as "classes with no rule" —
neither is a class name (one is a local variable, the other a `data-status` value). Comparison
operands (`=== "running"`) are dropped for the same reason, while the literal that really is a class
(`" pill--stop"`) is kept.

Both lists are expected to be **empty**. A class in the DOM with no rule is either a leftover from a
rule that was deleted or a hook that belongs in `data-*`; a rule nothing can render is dead CSS.
"""

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
css = (ROOT / "src/styles.css").read_text()

# --- selectors only ------------------------------------------------------------------------
no_comments = re.sub(r"/\*.*?\*/", " ", css, flags=re.S)
no_urls = re.sub(r"url\([^)]*\)", "url()", no_comments)
sel, depth, buf = [], 0, ""
for ch in no_urls:
    if ch == "{":
        if depth == 0:
            sel.append(buf)
        buf, depth = "", depth + 1
    elif ch == "}":
        depth, buf = depth - 1, ""
    else:
        buf += ch
css_classes = sorted(set(re.findall(r"\.(-?[A-Za-z_][\w-]*)", "\n".join(sel))))


def class_tokens(blob: str) -> list[str]:
    """Class names in one space-separated className literal."""
    return re.findall(r"-?[A-Za-z_][\w-]*", blob)


def literals(expression: str) -> list[str]:
    """Every string literal in a `className={…}` expression, comparison operands excluded.

    Template literals contribute their static chunks; `${…}` interpolations contribute only the
    string literals inside them, never bare identifiers.
    """
    expr = re.sub(r"[=!]==?\s*(\"[^\"]*\"|'[^']*'|`[^`]*`)", " ", expression)
    out: list[str] = []
    for m in re.finditer(r"\"([^\"]*)\"|'([^']*)'", expr):
        out.append(m.group(1) if m.group(1) is not None else m.group(2))
    # Template literal: keep the static text, drop every `${…}` hole (already mined above).
    for m in re.finditer(r"`([^`]*)`", expr):
        out.extend(re.split(r"\$\{[^}]*\}", m.group(1)))
    return out


# --- what components can render ------------------------------------------------------------
rendered = set()
for p in sorted(ROOT.glob("src/**/*.tsx")) + [ROOT / "index.html"]:
    text = p.read_text()
    text = re.sub(r"\{/\*.*?\*/\}", " ", text, flags=re.S)  # jsx comments
    text = re.sub(r"^\s*//.*$", " ", text, flags=re.M)  # line comments
    text = re.sub(r"/\*.*?\*/", " ", text, flags=re.S)  # block comments
    for m in re.finditer(r"class(?:Name)?=(?:\"([^\"]*)\"|'([^']*)'|\{)", text):
        if m.group(1) is not None or m.group(2) is not None:
            rendered.update(class_tokens(m.group(1) or m.group(2) or ""))
            continue
        # `className={…}`: take the balanced expression, then only its string literals.
        depth, start = 0, m.end() - 1
        for i in range(start, len(text)):
            if text[i] == "{":
                depth += 1
            elif text[i] == "}":
                depth -= 1
                if depth == 0:
                    break
        for blob in literals(text[start + 1 : i]):
            rendered.update(class_tokens(blob))

# dialog-polyfill inserts this sibling when native showModal is unavailable.
polyfill = (ROOT / "node_modules/dialog-polyfill/dist/dialog-polyfill.esm.js").read_text()
if "className = 'backdrop'" not in polyfill:
    raise RuntimeError("dialog-polyfill's backdrop contract changed")
rendered.add("backdrop")

dead = [c for c in css_classes if c not in rendered]
orphans = sorted(rendered - set(css_classes))
print(f"selector classes: {len(css_classes)}   renderable classes: {len(rendered)}")
print("\nCSS classes nothing can render:")
for c in dead:
    print("   .", c, sep="")
print("\nrenderable classes with no CSS rule:")
for c in orphans:
    print("   .", c, sep="")

# Both lists are expected to be empty, so say so with the exit status too: CI runs this file, and a
# probe that always succeeds is a probe that reports nothing. Printing the names was never the point
# - failing the build the moment styles.css and the components disagree is.
if dead or orphans:
    print(
        f"\nFAIL: {len(dead)} rule(s) nothing can render, "
        f"{len(orphans)} class(es) with no rule.",
        file=sys.stderr,
    )
    sys.exit(1)
print("\nok: every rule is renderable and every rendered class has a rule")
