"""Regenerate curated Flint screenshots using synthetic preview data.

Run with the preview server: python3 scripts/shots_readme.py [hero settings ...]
Captures the default light theme at 960×660, 2× pixel density.
"""

from __future__ import annotations

import asyncio
import pathlib
import sys
import urllib.error
import urllib.request

from playwright.async_api import Page, async_playwright

ROOT = pathlib.Path(__file__).resolve().parents[1]
OUT = ROOT / "docs" / "screenshots"
URL = "http://127.0.0.1:1420/"

WIDTH, HEIGHT, SCALE = 960, 660, 2

# The DOM bridge the mock listens on. The mock records
# `file.size` for every dropped path and reports it as the row's weight, and a 742 MB Uint8Array is
# not a thing to allocate in a browser. Defining `size` on a 1 KB File gives the rows believable
# weights without believable memory.
DROP = """
(files) => {
  const dt = new DataTransfer();
  for (const f of files) {
    const file = new File([new Uint8Array(1024)], f.name);
    Object.defineProperty(file, 'size', { value: f.size });
    dt.items.add(file);
  }
  window.dispatchEvent(new DragEvent('drop', { dataTransfer: dt, bubbles: true, cancelable: true }));
}
"""

# A batch someone might really have: two long takes and a cutaway, room tone, two stills off the
# same shoot, the deck that goes with them, and the captions. Five of the app's six categories in
# eight rows - which is what "mixed queue" in the brief means - and no name that needs an apology.
# Eight is also what fills the 660px canvas without the list needing to scroll.
BATCH = [
    {"name": "product-launch-keynote.mov", "size": 742_000_000},
    {"name": "conference-panel-full.mkv", "size": 1_240_000_000},
    {"name": "studio-b-roll-0142.mov", "size": 318_000_000},
    {"name": "interview-room-tone.wav", "size": 96_000_000},
    {"name": "cover-artwork.HEIC", "size": 12_400_000},
    {"name": "press-kit-spread.tiff", "size": 48_000_000},
    {"name": "brand-guidelines.docx", "size": 3_400_000},
    {"name": "captions-en.vtt", "size": 42_000},
]

# Short enough to leave the settings sheet room to be the subject, long enough that the queue is
# visibly a queue through the scrim behind it.
SHORT_BATCH = BATCH[:4]

SCROLL_TO_TOOLS = "() => document.querySelector('.tools__header').scrollIntoView({block: 'start'})"

# A paste for `links.png`: two links the box accepts and one it refuses, because the refusal is the
# half of this feature a screenshot can show. Ids are placeholders of the right shape - the mock
# checks the host and the path, never that a video exists - and the third line is the mistake people
# actually make, a playlist URL copied instead of the video's own.
LINKS_PASTE = "\n".join(
    [
        "https://www.youtube.com/watch?v=Jv8LmQrTx6A",
        "https://www.bilibili.com/video/BV1qE41167aP",
        "https://www.youtube.com/playlist?list=PLh9Kx2Fn4qTd",
    ]
)

# Two links for `walkthrough.png`, both YouTube because YouTube is the host that puts a sign-in wall
# in front of a fetch, and two because the batch's question counts them.
GATED_PASTE = "\n".join(
    [
        "https://www.youtube.com/watch?v=Jv8LmQrTx6A",
        "https://www.youtube.com/watch?v=Kd7cMxQ91zR",
    ]
)


async def settle(page: Page, ms: int = 480) -> None:
    await page.wait_for_timeout(ms)


async def rest(page: Page) -> None:
    """Pointer off-window, no focus ring: nothing in the frame that a photograph should not own."""
    await page.mouse.move(-60, -60)
    await page.evaluate("() => document.activeElement?.blur?.()")
    # …and the window scrolled back to where the app keeps it. `scroll_into_view_if_needed` on a
    # field inside the settings sheet scrolls the *shell* sideways when it is asked while the sheet
    # is still sliding in from the right, and closing the sheet does not undo that: one run in three
    # produced a frame with the left 366px — the titlebar, every filename — scrolled out of the
    # picture. The app never scrolls itself horizontally, so this is a no-op in every other shot,
    # and only `scrollLeft` is touched because two shots depend on a vertical scroll they chose.
    await page.evaluate(
        "() => { window.scrollTo(0, 0); const a = document.querySelector('.app');"
        " if (a !== null) a.scrollLeft = 0; }"
    )
    await settle(page, 420)


async def shoot(page: Page, name: str) -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    path = OUT / f"{name}.png"
    await page.screenshot(path=str(path))
    size = path.stat().st_size
    print(f"    {path.relative_to(ROOT)}  {size // 1024} KB")


async def open_page(browser, query: str = "") -> Page:
    context = await browser.new_context(
        viewport={"width": WIDTH, "height": HEIGHT},
        device_scale_factor=SCALE,
        color_scheme="light",
    )
    page = await context.new_page()
    page.on("pageerror", lambda e: print(f"    [pageerror] {e}"))
    page.on("console", lambda m: m.type == "error" and print(f"    [console] {m.text}"))
    await page.goto(URL + query, wait_until="networkidle")
    await settle(page, 620)
    return page


async def drop_batch(page: Page, files: list[dict[str, object]]) -> None:
    await page.evaluate(DROP, files)
    await page.wait_for_function(
        "(n) => document.querySelectorAll('.row').length === n", arg=len(files)
    )
    await settle(page, 360)


async def open_settings(page: Page) -> None:
    await page.keyboard.press("Control+,")
    await page.wait_for_selector(".drawer[data-open]")
    await settle(page, 700)


async def close_settings(page: Page) -> None:
    await page.keyboard.press("Escape")
    await page.wait_for_selector(".drawer[data-open]", state="detached")
    await settle(page, 520)


async def set_parallel(page: Page, jobs: int) -> None:
    """Turn "Parallel conversions" up, through the control a user would use.

    `converting.png` is supposed to show several files genuinely in flight, and the mock's default
    is the same "Automatic" two workers the real backend picks. Setting it in the sheet and closing
    the sheet again leaves the state a user could have reached, with nothing faked in the store.
    """
    await open_settings(page)
    # The sections are <details>, collapsed on open, so the field has to be revealed the same way a
    # user would reveal it.
    section = page.locator("details.section", has_text="Parallel conversions")
    await section.locator("summary").click()
    field = section.locator(".field", has_text="Parallel conversions").locator("input")
    await field.scroll_into_view_if_needed()
    await field.fill(str(jobs))
    await settle(page, 360)
    await close_settings(page)


# A frame worth keeping: enough rows finished that the "done" treatment (output name + Reveal) is
# on screen, enough running that progress is, and at least one running row past its first few
# percent with an ETA next to it. Anything less is a photograph of a batch that just started.
def mid_batch(done: int, running: int, min_percent: int) -> str:
    return f"""
    () => {{
      const rows = (s) => [...document.querySelectorAll(`.row[data-status="${{s}}"]`)];
      const live = rows('running');
      if (rows('done').length < {done} || live.length < {running}) return false;
      const pct = live.map((r) => parseInt(r.querySelector('.row__status')?.textContent ?? '0', 10));
      const eta = live.some((r) => (r.querySelector('.row__status')?.textContent ?? '').includes('left'));
      const bar = document.querySelector('.actionbar__progressfill');
      return eta && bar !== null && Math.max(0, ...pct) >= {min_percent};
    }}
    """


async def hero(browser) -> None:
    """The money shot: a mixed batch mid-flight, rows done above rows still going."""
    print("hero.png")
    page = await open_page(browser, "?missing=")
    await drop_batch(page, BATCH)
    await page.click(".pill")
    await page.wait_for_function(mid_batch(done=3, running=2, min_percent=25), timeout=60_000)
    await rest(page)
    await shoot(page, "hero")
    await page.context.close()


async def empty(browser) -> None:
    """The window with nothing in it: the engraved `+` in its hairline mat, and no action bar."""
    print("empty.png")
    page = await open_page(browser, "?missing=")
    await page.wait_for_selector(".dropzone")
    await rest(page)
    await shoot(page, "empty")
    await page.context.close()


async def converting(browser) -> None:
    """Four conversions at once, each row carrying its own percentage, ETA and accent line."""
    print("converting.png")
    page = await open_page(browser, "?missing=")
    await set_parallel(page, 4)
    await drop_batch(page, BATCH)
    await page.click(".pill")
    await page.wait_for_function(mid_batch(done=1, running=4, min_percent=20), timeout=60_000)
    await rest(page)
    await shoot(page, "converting")
    await page.context.close()


async def links(browser) -> None:
    """The paste box: two links accepted, one playlist refused in its own words.

    Opened with ⌘L, which is the same box ⌘V opens — the shortcut is used here because a synthetic
    clipboard event would photograph the identical card while needing a `DataTransfer` to do it.
    `?missing=` keeps yt-dlp present, so the shot shows the normal case and not the install hint.
    The queue behind the veil is `settings.png`'s, since these three sheet shots sit together and a
    black void behind one of them reads as a different app.
    """
    print("links.png")
    page = await open_page(browser, "?missing=")
    await drop_batch(page, SHORT_BATCH)
    await page.keyboard.press("Control+l")
    await page.wait_for_selector(".links__where")
    await page.fill(".links__box", LINKS_PASTE)
    # The box asks the backend 200ms after the last keystroke, so the count and the refusal are
    # waited for rather than slept into: both are on screen before the shutter, or neither is.
    await page.wait_for_function(
        """
        () => {
          const count = document.querySelector('.links__count');
          const refused = document.querySelectorAll('.links__refusal');
          return count?.textContent === '2 of 20 links' && refused.length === 1;
        }
        """
    )
    await settle(page, 360)
    await rest(page)
    await shoot(page, "links")
    await page.context.close()


async def settings(browser) -> None:
    """The sheet slid in from the right, at the top: the four presets and what they mean."""
    print("settings.png")
    page = await open_page(browser, "?missing=")
    await drop_batch(page, SHORT_BATCH)
    await open_settings(page)
    # `Video` is the one section that starts expanded, and at 660px its fields run off the bottom
    # edge mid-label. Collapsed, the sheet is the composition it is meant to be: the four presets,
    # then the list of sections. The seventh section (Links) pushed `Output` onto the fold, which is
    # left as it falls — the sheet does scroll, and a shot that hid that would be arranging the app.
    await page.locator("details.section[open] > summary").first.click()
    await page.evaluate("() => document.querySelector('.drawer__body').scrollTo({ top: 0 })")
    await rest(page)
    await shoot(page, "settings")
    await page.context.close()


async def signin(browser) -> None:
    """Settings → Links with a browser chosen: the sign-in a pasted link can borrow.

    Driven through the two controls a user drives, not a knob: choosing *From a browser* is what
    wakes the browser select, so a shot taken any other way would show a live control nobody had
    switched on. Chrome is the pick because it is the browser most people have signed in.
    """
    print("signin.png")
    page = await open_page(browser, "?missing=")
    await drop_batch(page, SHORT_BATCH)
    await open_settings(page)
    section = page.locator("details.section", has_text="Sign-in for pasted links")
    await section.locator("summary").click()
    selects = section.locator("select")
    await selects.first.select_option("browser")
    await settle(page, 320)
    await selects.nth(1).select_option("chrome")
    # The group is the second-to-last thing in a sheet that scrolls, so it is brought to the top of
    # the frame. `Video` is left expanded above it on purpose: collapsing everything makes the
    # sheet shorter than its own scroll and the group lands half off the bottom edge instead.
    await section.locator("summary").scroll_into_view_if_needed()
    await settle(page, 420)
    await rest(page)
    await shoot(page, "signin")
    await page.context.close()


async def walkthrough(browser) -> None:
    """The guided sign-in sheet, reached the way a stopped link reaches it.

    Not a knob and not Settings: two gated links are pasted, queued and run against `?linkfail=signin`
    — a Mac whose YouTube fetches come back wanting a sign-in — and the sheet is opened from the
    **Set up a sign-in…** on the row that stopped. The batch's own question is dismissed with Escape
    first, because two modals in one frame photograph an app arguing with itself, and the question
    already has its words quoted in the README. `?browsers=chrome,safari` is the ordinary Mac the
    steps are written for: Chrome to sign in with, Safari present so the Full Disk Access step is
    honestly on screen. The failed rows stay visible behind the veil, which is the whole point of
    the sheet.
    """
    print("walkthrough.png")
    page = await open_page(browser, "?missing=&linkfail=signin&browsers=chrome,safari&linkms=20")
    await page.keyboard.press("Control+l")
    await page.wait_for_selector(".links__where")
    await page.fill(".links__box", GATED_PASTE)
    await page.wait_for_function(
        "() => document.querySelector('.links__count')?.textContent === '2 of 20 links'"
    )
    await page.click(".links .promptbutton")
    await page.wait_for_function("() => document.querySelectorAll('.row').length === 2")
    await page.click(".pill")
    # The question is what a settled batch of gated links ends on, so it is what has to be waited
    # for: it means both fetches are finished and both rows carry their fix link.
    await page.wait_for_selector(".prompt", timeout=60_000)
    await page.keyboard.press("Escape")
    await page.wait_for_selector(".prompt", state="detached")
    await page.click(".row__fix")
    await page.wait_for_selector(".signin")
    await settle(page, 520)
    await rest(page)
    await shoot(page, "walkthrough")
    await page.context.close()


async def helpers(browser) -> None:
    """Settings -> Helper apps with Pandoc absent, so the Install button is on screen."""
    print("helpers.png")
    page = await open_page(browser, "?missing=pandoc")
    # Same queue behind the scrim as `settings.png`: the two sheet shots sit next to each other in
    # the README table and a black void behind one of them reads as a different app.
    await drop_batch(page, SHORT_BATCH)
    await open_settings(page)
    await page.evaluate(SCROLL_TO_TOOLS)
    await page.wait_for_selector('.tool[data-tool="pandoc"] .toolbutton')
    await settle(page, 420)
    await rest(page)
    await shoot(page, "helpers")
    await page.context.close()


SHOTS = {
    "hero": hero,
    "empty": empty,
    "converting": converting,
    "links": links,
    "settings": settings,
    "signin": signin,
    "walkthrough": walkthrough,
    "helpers": helpers,
}


def dev_server_is_up() -> bool:
    try:
        with urllib.request.urlopen(URL, timeout=3) as response:
            return response.status == 200
    except (urllib.error.URLError, OSError):
        return False


async def main() -> int:
    wanted = sys.argv[1:] or list(SHOTS)
    unknown = [name for name in wanted if name not in SHOTS]
    if unknown:
        print(f"unknown shot(s): {', '.join(unknown)}", file=sys.stderr)
        print(f"known: {', '.join(SHOTS)}", file=sys.stderr)
        return 2

    if not dev_server_is_up():
        # Skipping is the honest outcome, not a failure: this driver is run by hand next to a dev
        # server, and there is nothing here for CI to gate on.
        print(f"no dev server on {URL} — run `npm run dev` first. Nothing written.")
        return 0

    async with async_playwright() as p:
        browser = await p.chromium.launch()
        for name in wanted:
            await SHOTS[name](browser)
        await browser.close()
    return 0


sys.exit(asyncio.run(main()))
