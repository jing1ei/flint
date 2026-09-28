"""Browser regressions for layout, keyboard access, queue recovery and settings.

Run against the local mock preview with: python3 scripts/ui_behaviour.py.
Assertions below describe each scenario; screenshots are written to ignored shots/.
"""

from __future__ import annotations

import asyncio
import re
import sys
import time

from playwright.async_api import Page, TimeoutError as PWTimeout, async_playwright

URL = "http://127.0.0.1:1420/"
EASE = "cubic-bezier(0.22, 1, 0.36, 1)"

passed = 0
failures: list[str] = []
ERRORS: dict = {}
"""Every file chooser a page has opened, in order — see `fresh` for why they are collected."""
CHOOSERS: dict = {}


def check(name: str, ok: bool, detail: object = "") -> None:
    global passed
    if ok:
        passed += 1
        print(f"  ok   {name}" + (f"  [{detail}]" if detail != "" else ""))
    else:
        failures.append(f"{name}  [{detail}]")
        print(f"  FAIL {name}  [{detail}]")


def near(a: float, b: float, tol: float = 1.0) -> bool:
    return abs(a - b) <= tol


def rgba(value: str) -> tuple[float, float, float, float]:
    nums = [float(n) for n in re.findall(r"[\d.]+", value)]
    while len(nums) < 4:
        nums.append(1.0)
    return nums[0], nums[1], nums[2], nums[3]


def ink(value: str) -> float:
    """How much ink a colour puts on the page: alpha, ignoring hue."""
    return rgba(value)[3]


def _channel(value: float) -> float:
    """One sRGB channel, linearised — WCAG 2.1's own expression, byte for byte."""
    c = value / 255
    return c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4


def luminance(colour: tuple[float, float, float, float]) -> float:
    r, g, b, _ = colour
    return 0.2126 * _channel(r) + 0.7152 * _channel(g) + 0.0722 * _channel(b)


def over(top: str, bottom: tuple[float, float, float, float]) -> tuple[float, float, float, float]:
    """`top` composited on an opaque `bottom`. Ink in this app is alpha, so this is most of the work."""
    r, g, b, a = rgba(top)
    br, bg_, bb, _ = bottom
    return (r * a + br * (1 - a), g * a + bg_ * (1 - a), b * a + bb * (1 - a), 1.0)


def contrast(colour: str, backdrop: list[str]) -> float:
    """Contrast ratio of one text colour against the stack of backgrounds actually behind it.

    `backdrop` is every ancestor's `background-color` from the element outwards, so it is composited
    from `<html>` inwards — a row's translucent hover wash sits on the body's paper, not on nothing,
    and a token's real ratio is the one it has after that flattening.
    """
    base: tuple[float, float, float, float] = (255.0, 255.0, 255.0, 1.0)
    for layer in reversed(backdrop):
        if ink(layer) > 0:
            base = over(layer, base)
    a, b = luminance(over(colour, base)), luminance(base)
    hi, lo = max(a, b), min(a, b)
    return round((hi + 0.05) / (lo + 0.05), 2)


async def box(page: Page, selector: str) -> dict:
    return await page.evaluate(
        "(s) => { const r = document.querySelector(s).getBoundingClientRect();"
        " return {x: r.x, y: r.y, w: r.width, h: r.height,"
        " cx: r.x + r.width / 2, cy: r.y + r.height / 2, right: r.right, bottom: r.bottom}; }",
        selector,
    )


async def css(page: Page, selector: str, prop: str) -> str:
    return await page.evaluate(
        "([s, p]) => getComputedStyle(document.querySelector(s)).getPropertyValue(p)",
        [selector, prop],
    )


async def fresh(browser, scheme: str = "dark", width: int = 960, height: int = 660,
                motion: str = "no-preference", query: str = "") -> Page:
    """A new window. `query` reaches the mock's preview knobs (`?missing=`, `?nobrew`, …)."""
    context = await browser.new_context(
        viewport={"width": width, "height": height},
        color_scheme=scheme,
        reduced_motion=motion,
    )
    page = await context.new_page()
    errors: list[str] = []
    page.on("pageerror", lambda e: errors.append(str(e)))
    page.on("console", lambda m: m.type == "error" and errors.append(m.text))
    ERRORS[page] = errors
    # Chromium only reports a file chooser while Playwright has asked it to intercept them, and
    # Playwright only asks while a `filechooser` listener exists — with a fire-and-forget request it
    # does not wait for. A per-assertion `expect_file_chooser` therefore turns interception on in the
    # same tick as the keystroke it is about, and on a loaded machine the keystroke wins: the page
    # opens the picker, nothing is intercepting yet, and the wait times out (this is what failed
    # assertion 5). One listener for the life of the page turns interception on once, seconds before
    # the first assertion needs it, and records every chooser for `opens_chooser` to observe.
    chosen: list = []
    CHOOSERS[page] = chosen
    page.on("filechooser", lambda fc: chosen.append(fc))
    await page.goto(URL + query, wait_until="networkidle")
    await page.wait_for_selector(".dropzone")
    await page.wait_for_timeout(500)
    return page


async def unhover(page: Page) -> None:
    await page.mouse.move(-40, -40)
    await page.evaluate("() => document.activeElement?.blur?.()")
    await page.wait_for_timeout(500)


async def opens_chooser(page: Page, action, timeout: float = 2500) -> bool:
    """Did `action` open a file chooser?

    Polls the choosers `fresh` has recorded for this page rather than arming a fresh interception
    around the action, so the only thing being waited on is the observable event itself.
    """
    seen = len(CHOOSERS[page])
    await action()
    deadline = time.monotonic() + timeout / 1000
    while time.monotonic() < deadline:
        if len(CHOOSERS[page]) > seen:
            return True
        await asyncio.sleep(0.02)
    return False


async def hover_centre(page: Page) -> None:
    size = page.viewport_size
    await page.mouse.move(size["width"] / 2, size["height"] / 2)
    await page.wait_for_timeout(560)


DRAG_ENTER = """
() => {
  const dt = new DataTransfer();
  dt.items.add(new File([new Uint8Array(8)], 'clip.mov'));
  window.dispatchEvent(new DragEvent('dragenter', { dataTransfer: dt, bubbles: true, cancelable: true }));
}
"""

DROP = """
(names) => {
  const dt = new DataTransfer();
  for (const n of names) dt.items.add(new File([new Uint8Array(4096)], n));
  window.dispatchEvent(new DragEvent('drop', { dataTransfer: dt, bubbles: true, cancelable: true }));
}
"""


async def run(browser) -> None:
    # =========================================================== the control still behaves ====
    page = await fresh(browser)
    await unhover(page)

    check("1  <main>'s surface is role=button", await page.get_attribute(".dropzone", "role") == "button")
    check("2  it is in the tab order", await page.get_attribute(".dropzone", "tabindex") == "0")
    check(
        "3  it is labelled",
        await page.get_attribute(".dropzone", "aria-label") == "Choose files to convert",
    )

    check("4  click anywhere opens the chooser", await opens_chooser(page, lambda: page.mouse.click(760, 520)))
    await page.keyboard.press("Escape")
    await page.focus(".dropzone")
    check("5  Enter opens the chooser", await opens_chooser(page, lambda: page.keyboard.press("Enter")))
    check("6  Space opens the chooser", await opens_chooser(page, lambda: page.keyboard.press(" ")))

    zone = await box(page, ".dropzone")
    check("7  the traffic-light strip is not part of the target", zone["y"] >= 38, f"y={zone['y']}")
    check(
        "8  a point over the traffic lights hits the drag strip, not the target",
        await page.evaluate(
            "() => document.elementFromPoint(30, 16)?.closest('.titlebar') !== null"
            " && document.elementFromPoint(30, 16)?.closest('.dropzone') === null"
        ),
    )

    # ================================================================== the mat: geometry =====
    await unhover(page)
    frame = await box(page, ".dropzone__frame")
    check("9  the mat exists and is decorative", await page.get_attribute(".dropzone__frame", "aria-hidden") == "true")
    check("10 side inset is 28-34px", 28 <= frame["x"] <= 34, f"{frame['x']}px")
    check(
        "11 side insets are equal",
        near(frame["x"], 960 - frame["right"]),
        f"{frame['x']} / {960 - frame['right']}",
    )
    check("12 top inset clears the traffic lights", frame["y"] >= 44, f"{frame['y']}px")
    check("13 top inset stays in the mat band", frame["y"] <= 52, f"{frame['y']}px")
    check(
        "14 bottom inset is deeper than the sides and shallower than the top",
        frame["x"] < (660 - frame["bottom"]) < frame["y"],
        f"side={frame['x']} bottom={660 - frame['bottom']} top={frame['y']}",
    )
    widths = [await css(page, ".dropzone__frame", f"border-{s}-width") for s in ("top", "right", "bottom", "left")]
    check("15 the mat is 1px on all four sides", set(widths) == {"1px"}, widths)
    styles = [await css(page, ".dropzone__frame", f"border-{s}-style") for s in ("top", "right", "bottom", "left")]
    check("16 the mat is never dashed", set(styles) == {"solid"}, styles)
    radius = float((await css(page, ".dropzone__frame", "border-top-left-radius")).replace("px", ""))
    check("17 radius is 2px at most", radius <= 2, f"{radius}px")
    check(
        "18 nothing else draws a second frame while empty",
        await page.evaluate(
            "() => getComputedStyle(document.querySelector('.app'), '::after').display === 'none'"
        ),
    )

    # ====================================================================== the mark: size ====
    plus = await box(page, ".dropzone__plus")
    check("19 the mark is 32-40px", 32 <= plus["w"] <= 40, f"{plus['w']}px")
    check("20 the mark is square", near(plus["w"], plus["h"], 0.5), f"{plus['w']}x{plus['h']}")
    stroke_w = await page.evaluate(
        "() => getComputedStyle(document.querySelector('.dropzone__plus line')).strokeWidth"
    )
    check("21 the mark's stroke is a true hairline", stroke_w in ("1", "1px"), stroke_w)
    check(
        "22 the mark lands on the pixel grid",
        plus["x"] % 1 == 0 and plus["y"] % 1 == 0,
        f"x={plus['x']} y={plus['y']}",
    )
    check("23 it is inline SVG", await page.evaluate("() => document.querySelector('.dropzone__plus').tagName") == "svg")
    check(
        "24 the mark is centred on the mat horizontally",
        near(plus["cx"], frame["cx"], 1),
        f"{plus['cx']} vs {frame['cx']}",
    )
    check(
        "25 the mark is optically centred: 4-16px above the mat's true centre",
        4 <= frame["cy"] - plus["cy"] <= 16,
        f"{round(frame['cy'] - plus['cy'], 1)}px above centre",
    )

    # ================================================================= rest / hover / drag ====
    rest_plus = float(await css(page, ".dropzone__plus", "opacity"))
    rest_mat = await css(page, ".dropzone__frame", "border-top-color")
    check("26 the mark reads at rest on dark (0.28-0.5)", 0.28 <= rest_plus <= 0.5, rest_plus)
    check("27 the first step is readable before hover", float(await css(page, ".dropzone__hint", "opacity")) == 1)

    await hover_centre(page)
    hover_plus = float(await css(page, ".dropzone__plus", "opacity"))
    hover_mat = await css(page, ".dropzone__frame", "border-top-color")
    hint_op = float(await css(page, ".dropzone__hint", "opacity"))
    check("28 hover lifts the mark by opacity alone", hover_plus > rest_plus + 0.05, f"{rest_plus} -> {hover_plus}")
    check(
        "29 hover brightens the mat with it",
        ink(hover_mat) > ink(rest_mat),
        f"{rest_mat} -> {hover_mat}",
    )
    check("30 hover keeps the guidance readable", hint_op == 1, hint_op)
    hint = await box(page, ".dropzone__hint")
    gap = hint["y"] - plus["bottom"]
    check("31 the micro-copy belongs to the mark (12-26px below it)", 12 <= gap <= 26, f"{round(gap, 1)}px")
    check(
        "32 the micro-copy is still 10px / uppercase / 0.28em",
        (await css(page, ".dropzone__hint", "font-size") == "10px"
         and await css(page, ".dropzone__hint", "text-transform") == "uppercase"
         and await css(page, ".dropzone__hint", "letter-spacing") == "2.8px"),
        await css(page, ".dropzone__hint", "letter-spacing"),
    )

    await page.evaluate(DRAG_ENTER)
    await page.wait_for_timeout(560)
    drag_stroke = await css(page, ".dropzone__plus", "stroke")
    drag_mat = await css(page, ".dropzone__frame", "border-top-color")
    accent = (await page.evaluate("() => getComputedStyle(document.documentElement).getPropertyValue('--accent')")).strip()
    check(
        "33 drag-over turns the mark blue",
        rgba(drag_stroke)[2] > rgba(drag_stroke)[0] + 40,
        f"{drag_stroke} (--accent {accent})",
    )
    check(
        "34 drag-over turns the mat blue with it",
        rgba(drag_mat)[2] > rgba(drag_mat)[0] + 20,
        drag_mat,
    )
    check("35 drag-over hides the micro-copy", float(await css(page, ".dropzone__hint", "opacity")) == 0)
    check(
        "36 drag-over does not scale the mark",
        await css(page, ".dropzone__plus", "transform") in ("none", "matrix(1, 0, 0, 1, 0, 0)"),
        await css(page, ".dropzone__plus", "transform"),
    )

    # ============================================================================= motion =====
    durations = []
    for sel in (".dropzone__plus", ".dropzone__frame", ".dropzone__hint"):
        raw = await css(page, sel, "transition-duration")
        durations += [float(d.strip().replace("s", "")) * 1000 for d in raw.split(",")]
    check("37 every transition is 240-420ms", all(240 <= d <= 420 for d in durations), sorted(set(durations)))
    eases = []
    for sel in (".dropzone__plus", ".dropzone__frame", ".dropzone__hint"):
        eases += re.findall(r"cubic-bezier\([^)]*\)|[a-z-]+", await css(page, sel, "transition-timing-function"))
    check("38 on the one easing curve", set(eases) == {EASE}, set(eases))

    # =============================================================================== busy =====
    await page.evaluate("() => document.querySelector('.dropzone').setAttribute('data-busy', '')")
    await page.wait_for_timeout(200)
    check(
        "39 busy breathes the mark",
        (await css(page, ".dropzone__plus", "animation-name")) == "plus-breathe"
        and (await css(page, ".dropzone__plus", "animation-iteration-count")) == "infinite",
        await css(page, ".dropzone__plus", "animation-name"),
    )
    check("40 busy changes the cursor", await css(page, ".dropzone", "cursor") == "progress")
    check("41 no page errors", ERRORS[page] == [], ERRORS[page])
    await page.context.close()

    # ======================================================================== light scheme ====
    page = await fresh(browser, scheme="light")
    await unhover(page)
    light_plus = float(await css(page, ".dropzone__plus", "opacity"))
    light_mat = await css(page, ".dropzone__frame", "border-top-color")
    check("42 the mark is tuned separately for paper (0.28-0.5)", 0.28 <= light_plus <= 0.5, light_plus)
    check("43 the mat is dark ink on paper", rgba(light_mat)[0] < 60 and ink(light_mat) > 0.05, light_mat)
    await hover_centre(page)
    check(
        "44 hover works on paper too",
        float(await css(page, ".dropzone__plus", "opacity")) > light_plus,
    )
    await page.context.close()

    # ====================================================================== reduced motion ====
    page = await fresh(browser, motion="reduce")
    reduced = [
        float(d.strip().replace("s", "")) * 1000
        for sel in (".dropzone__plus", ".dropzone__frame")
        for d in (await css(page, sel, "transition-duration")).split(",")
    ]
    check("45 reduced motion collapses transitions to ~1ms", all(d <= 1 for d in reduced), sorted(set(reduced)))
    await page.context.close()

    # ================================================= the window minimum, and the queue ======
    page = await fresh(browser, width=720, height=520)
    await unhover(page)
    frame = await box(page, ".dropzone__frame")
    plus = await box(page, ".dropzone__plus")
    check("46 the mat survives 720x520", frame["w"] > 600 and frame["h"] > 380, f"{frame['w']}x{frame['h']}")
    check("47 the mark keeps its size when the room shrinks", 32 <= plus["w"] <= 40, plus["w"])
    check(
        "48 the mark still sits above the mat's centre",
        4 <= frame["cy"] - plus["cy"] <= 16,
        round(frame["cy"] - plus["cy"], 1),
    )
    await page.context.close()

    page = await fresh(browser)
    await page.evaluate(DROP, ["a.mov", "b.HEIC", "c.m4a"])
    await page.wait_for_selector(".row")
    await page.wait_for_function("() => document.querySelectorAll('.row').length === 3")
    check("49 files replace the empty state", await page.query_selector(".dropzone") is None)
    check("50 the queue and its action bar appear", await page.query_selector(".actionbar") is not None)
    check(
        "51 nothing overflows the window",
        await page.evaluate("() => document.body.scrollWidth - document.body.clientWidth") == 0,
    )
    check("52 the queue produced no errors", ERRORS[page] == [], ERRORS[page])
    await page.context.close()

    await batch_machine(browser)
    await stale_stream(browser)
    await settings_sheet(browser)
    await queue_motion(browser)
    await helpers_quiet(browser)
    await helper_install(browser)
    await helper_install_fails(browser)
    await helper_without_homebrew(browser)
    await failed_row_to_helper(browser)
    await a_package_is_what_the_user_installs(browser)
    await install_motion(browser)
    await helper_prompt(browser)
    await helper_prompt_once(browser)
    await helper_prompt_aggregates(browser)
    await helper_prompt_to_settings(browser)
    await helper_prompt_stays_quiet(browser)
    await prompt_hands_the_keyboard_back(browser)
    await inherited_batch(browser)
    await stale_activity(browser)
    await half_made_destination(browser)
    await refused_batch_keeps_the_last_run(browser)
    await prompt_does_not_stack_with_settings(browser)
    await prompt_about_a_different_helper(browser)
    await inherited_install(browser)
    await capped_drop(browser)
    await one_row_per_package(browser)
    await keyboard_pruning(browser)
    await spoken_progress(browser)
    await installer_log_is_quiet(browser)
    await settings_number_copy(browser)
    await emptying_the_queue(browser)
    await bulk_picker_label(browser)
    await pickers_on_demand(browser)
    await interactive_contrast(browser)
    await engine_missing_is_said_on_the_canvas(browser)
    await paste_box(browser)
    await paste_anywhere(browser)
    await the_cap_is_the_backends(browser)
    await a_mixed_paste(browser)
    await links_in_the_queue(browser)
    await both_halves_of_a_link(browser)
    await without_yt_dlp(browser)
    await one_thing_over_the_window(browser)
    await the_trim_card(browser)
    await the_trim_where_the_files_land(browser)
    await a_trim_past_the_end(browser)
    await a_trimmed_link(browser)
    await the_links_group(browser)
    await a_sign_in_half_chosen(browser)
    await a_sign_in_the_backend_refuses(browser)
    await a_link_with_no_javascript_runtime(browser)
    await safari_needs_a_permission(browser)
    await the_installer_says_what_it_can_do_itself(browser)
    await a_link_stopped_by_a_sign_in_wall(browser)
    await the_question_does_the_rest(browser)
    await a_sign_in_the_check_disproves(browser)
    await a_sign_in_that_is_already_the_one_failing(browser)
    await two_macs_where_one_click_would_be_a_lie(browser)
    await the_walkthrough_itself(browser)
    await checking_a_sign_in_without_converting_anything(browser)
    await settings_links_says_what_is_not_here(browser)
    await the_sheet_answers_the_keyboard_like_the_paste_box(browser)
    await a_failure_no_sign_in_would_have_fixed(browser)
    await a_save_still_in_the_timer(browser)
    await the_question_hands_the_keyboard_on(browser)
    await two_clocks_that_have_to_agree(browser)
    await a_row_says_how_it_is_getting_on(browser)
    await the_bar_names_a_folder_not_a_file(browser)
    await a_job_nobody_can_measure(browser)
    await a_batch_that_ended_in_a_panic(browser)
    await an_install_that_ended_in_a_panic(browser)
    await the_browser_the_sign_in_is_actually_in(browser)
    await safari_answers_from_the_permission(browser)
    await edited_links_cannot_submit_stale_results(browser)


# ==================================================================== the batch state machine ====
#
# Everything below drives a real run through the mock backend. Row ids are the mock's own
# (`mock-1`, `mock-2`, … in drop order, one counter per page load), which is what lets these
# checks post the awkward events a UI cannot produce by hand: a duplicate terminal event, an event
# after `batch_finished`, an id from a run that is over. `__ceMockEmit` is installed by
# `src/lib/mock.ts` and exists only in the browser preview.

EMIT = "(event) => window.__ceMockEmit(event)"

STOP_AND_READ = """
async () => {
  const pill = () => document.querySelector('.pill');
  // The precondition, restored inside this one evaluate rather than over a round trip of its own:
  // the mock's four files can settle while the checks above are talking to the page, and a Stop
  // that lands on a batch which has just settled is a Convert. A batch that has only just been
  // started cannot settle underneath the read below — its shortest item is twelve 70ms ticks long.
  if (pill().textContent !== 'Stop') {
    pill().click();
    for (let i = 0; i < 600 && pill().textContent !== 'Stop'; i += 1) {
      await new Promise((r) => requestAnimationFrame(r));
    }
  }
  const armed = pill().textContent;
  pill().click();
  // React flushes a click's state update in a microtask, so draining the microtask queue is enough
  // to see the button the click produced — and no timer, and therefore no `batch_finished`, can run
  // at a microtask checkpoint. Waiting for animation frames instead let the batch settle first and
  // the button read `Convert 2 files`, which is what made this check flaky.
  for (let i = 0; i < 4; i += 1) await Promise.resolve();
  const now = pill();
  return { armed, text: now.textContent, disabled: now.disabled };
}
"""

STATUSES = "() => [...document.querySelectorAll('.row')].map((r) => r.dataset.status)"


async def batch_machine(browser) -> None:
    page = await fresh(browser)
    await page.evaluate(DROP, ["one.mov", "two.mov", "broken-take.mov", "three.m4a"])
    await page.wait_for_function("() => document.querySelectorAll('.row').length === 4")
    await page.click(".pill")
    await page.wait_for_function("() => document.querySelector('.pill').textContent === 'Stop'")

    check(
        "53 a running batch locks every target picker it owns",
        await page.evaluate("() => [...document.querySelectorAll('.target__select')].every((s) => s.disabled)"),
    )
    check(
        "54 a running batch locks the × on the rows it owns",
        await page.evaluate("() => [...document.querySelectorAll('.row__remove')].every((b) => b.disabled)"),
    )
    check(
        "55 Clear all is unavailable while a batch runs",
        await page.evaluate(
            "() => [...document.querySelectorAll('.filelist__header .microlink')]"
            ".every((b) => b.textContent === 'Clear all' && b.disabled)"
        ),
    )

    await page.click(".row")
    await page.keyboard.press("Backspace")
    await page.wait_for_timeout(120)
    check(
        "56 backspace cannot delete a row the batch is converting",
        len(await page.evaluate(STATUSES)) == 4,
        await page.evaluate(STATUSES),
    )

    # First verdict wins: a duplicate terminal event must not turn a finished row into a failure.
    await page.wait_for_selector('.row[data-status="done"]', timeout=30000)
    done_index = await page.evaluate(
        "() => [...document.querySelectorAll('.row')].findIndex((r) => r.dataset.status === 'done')"
    )
    await page.evaluate(EMIT, {"type": "failed", "id": f"mock-{done_index + 1}", "message": "late failure"})
    await page.wait_for_timeout(120)
    check(
        "57 a duplicate terminal event cannot overwrite a row's verdict",
        (await page.evaluate(STATUSES))[done_index] == "done",
        await page.evaluate(STATUSES),
    )
    await page.evaluate(EMIT, {"type": "progress", "id": "mock-999", "fraction": 0.5, "speed": 1, "eta_secs": 9})
    await page.wait_for_timeout(120)
    check("58 an event for an unknown row is dropped, not thrown", ERRORS[page] == [], ERRORS[page])

    # "Click Stop" is only a question you can ask a batch that is still running, and the mock's
    # four files can finish while the checks above are talking to the page. `STOP_AND_READ` restores
    # that precondition itself and reads the button without ever letting go of the task the click
    # was made in, so neither the arming nor the reading can be overtaken by `batch_finished`.
    stopping = await page.evaluate(STOP_AND_READ)
    check(
        "59 stopping says so and cannot be clicked twice",
        stopping["armed"] == "Stop"
        and stopping["text"] == "Stopping…"
        and stopping["disabled"] is True,
        stopping,
    )

    await page.wait_for_function(
        "() => document.querySelector('.pill').textContent.startsWith('Convert')", timeout=30000
    )
    statuses = await page.evaluate(STATUSES)
    check(
        "60 a cancelled batch leaves no row mid-flight",
        all(s in ("done", "failed", "skipped") for s in statuses),
        statuses,
    )
    check(
        "61 the cancelled rows say why",
        await page.evaluate(
            "() => [...document.querySelectorAll('.row__skipped')].some((s) => s.textContent === 'Cancelled')"
        ),
    )
    check(
        "62 no progress line survives the run",
        await page.query_selector(".actionbar__progress") is None
        and await page.query_selector(".row__progress") is None,
    )
    check(
        "63 the primary action is offered again once the batch is over",
        await page.evaluate("() => !document.querySelector('.pill').disabled"),
    )

    # Events that arrive after `batch_finished` belong to a run the window has retired.
    before = await page.evaluate(STATUSES)
    await page.evaluate(EMIT, {"type": "started", "id": "mock-1", "output": "/tmp/x.mp4", "summary": "FFmpeg"})
    await page.evaluate(EMIT, {"type": "progress", "id": "mock-1", "fraction": 0.4, "speed": 1, "eta_secs": 3})
    await page.wait_for_timeout(150)
    check(
        "64 an event after batch_finished cannot restart a row",
        await page.evaluate(STATUSES) == before,
        f"{before} -> {await page.evaluate(STATUSES)}",
    )
    check(
        "65 and cannot put the window back into a running state",
        await page.evaluate("() => document.querySelector('.pill').textContent") != "Stop"
        and await page.query_selector(".row__progress") is None,
    )

    # Re-picking the format on a finished row throws the old result away rather than keeping a
    # filename that describes a file this row is no longer going to produce.
    changed = await page.evaluate(
        """
        (index) => {
          const row = document.querySelectorAll('.row')[index];
          const select = row.querySelector('.target__select');
          // A picker builds its option list when it is used (223 onwards), and this reads that list
          // rather than clicking through it — so it opens the picker first, the way a hand would.
          // `focus` is dispatched synchronously, and the list is built in that same event.
          select.focus();
          const other = [...select.options].find((o) => o.value !== select.value && !o.disabled);
          select.value = other.value;
          select.dispatchEvent(new Event('change', { bubbles: true }));
          return true;
        }
        """,
        done_index,
    )
    await page.wait_for_timeout(150)
    check(
        "66 retargeting a finished row clears its result",
        changed
        and (await page.evaluate(STATUSES))[done_index] == "queued"
        and await page.evaluate(
            "(i) => document.querySelectorAll('.row')[i].querySelector('.row__output') === null",
            done_index,
        ),
        (await page.evaluate(STATUSES))[done_index],
    )

    # Let the whole queue run to the end this time: the file with "broken" in its name is the
    # mock's deterministic failure, which is the only way to reach the error + Retry path.
    await page.click(".pill")
    await page.wait_for_function("() => document.querySelector('.pill').textContent === 'Stop'")
    await page.wait_for_function(
        "() => document.querySelector('.pill').textContent.startsWith('Convert')", timeout=60000
    )
    failed_index = await page.evaluate(
        "() => [...document.querySelectorAll('.row')].findIndex((r) => r.dataset.status === 'failed')"
    )
    check("67 the broken file really did fail", failed_index >= 0, await page.evaluate(STATUSES))
    check(
        "68 a failed row explains itself",
        await page.evaluate(
            "(i) => (document.querySelectorAll('.row')[i].querySelector('.row__error')?.textContent ?? '')"
            ".includes('exited with status')",
            failed_index,
        ),
        await page.evaluate(
            "(i) => document.querySelectorAll('.row')[i].querySelector('.row__error')?.textContent",
            failed_index,
        ),
    )
    await page.evaluate(
        "(i) => [...document.querySelectorAll('.row')[i].querySelectorAll('.microlink')]"
        ".find((b) => b.textContent === 'Retry').click()",
        failed_index,
    )
    await page.wait_for_timeout(250)
    check(
        "69 retry clears the previous error before it runs again",
        (await page.evaluate(STATUSES))[failed_index] in ("queued", "running")
        and await page.evaluate(
            "(i) => document.querySelectorAll('.row')[i].querySelector('.row__error') === null",
            failed_index,
        ),
        (await page.evaluate(STATUSES))[failed_index],
    )
    await page.wait_for_function(
        "() => document.querySelector('.pill').textContent.startsWith('Convert')", timeout=60000
    )
    check("70 the whole run produced no page errors", ERRORS[page] == [], ERRORS[page])
    await page.context.close()


async def stale_stream(browser) -> None:
    """A `batch_finished` from a run this window never started must not summarise the queue."""
    page = await fresh(browser)
    await page.evaluate(DROP, ["one.mov", "two.mov"])
    await page.wait_for_function("() => document.querySelectorAll('.row').length === 2")
    before = await page.evaluate(STATUSES)
    await page.evaluate(EMIT, {"type": "batch_finished", "ok": 0, "failed": 0, "skipped": 2})
    await page.wait_for_timeout(200)
    check(
        "71 a batch_finished nobody asked for leaves the queue alone",
        await page.evaluate(STATUSES) == before == ["queued", "queued"],
        f"{before} -> {await page.evaluate(STATUSES)}",
    )
    check(
        "72 and does not report a batch that never ran",
        "converted" not in (await page.text_content(".actionbar__text") or ""),
        await page.text_content(".actionbar__text"),
    )
    check("73 no page errors from the stale stream", ERRORS[page] == [], ERRORS[page])
    await page.context.close()


# ============================================================================ the settings sheet ==

FOCUS_INSIDE_DRAWER = "() => document.querySelector('.drawer').contains(document.activeElement)"


async def settings_sheet(browser) -> None:
    page = await fresh(browser)
    await page.evaluate(DROP, ["one.mov"])
    await page.wait_for_selector(".row")
    await page.click(".row")

    # One keystroke, one toggle: the webview handler and the mock's menu bridge both listen.
    await page.keyboard.press("Control+,")
    await page.wait_for_timeout(500)
    check(
        "74 the menu shortcut opens the sheet exactly once",
        await page.get_attribute(".drawer", "data-open") is not None,
        await page.get_attribute(".drawer", "data-open"),
    )
    check(
        "75 focus moves into the sheet",
        await page.evaluate("() => document.activeElement?.getAttribute('aria-label')") == "Close settings",
        await page.evaluate("() => document.activeElement?.getAttribute('aria-label')"),
    )

    for _ in range(14):
        await page.keyboard.press("Tab")
    check("76 Tab cannot walk out of the sheet", await page.evaluate(FOCUS_INSIDE_DRAWER))
    await page.keyboard.press("Shift+Tab")
    await page.keyboard.press("Shift+Tab")
    check("77 Shift+Tab cannot either", await page.evaluate(FOCUS_INSIDE_DRAWER))

    rows_before = len(await page.query_selector_all(".row"))
    await page.keyboard.press("Backspace")
    await page.wait_for_timeout(150)
    check(
        "78 backspace behind the sheet does not delete the selected row",
        len(await page.query_selector_all(".row")) == rows_before,
    )

    # The archive preset promises it keeps metadata — for video as well as for stills.
    await page.evaluate(
        "() => [...document.querySelectorAll('.preset')]"
        ".find((p) => p.textContent.includes('archive')).querySelector('input').click()"
    )
    await page.wait_for_timeout(300)
    strip = await page.evaluate(
        """
        () => {
          const video = [...document.querySelectorAll('.section')]
            .find((s) => s.querySelector('.section__title').textContent === 'Video');
          const field = [...video.querySelectorAll('.check')]
            .find((l) => l.textContent.includes('Strip metadata'));
          return field.querySelector('input').checked;
        }
        """
    )
    check("79 the archive preset keeps video metadata", strip is False, f"checked={strip}")

    await page.keyboard.press("Escape")
    await page.wait_for_timeout(500)
    check("80 Escape closes the sheet", await page.get_attribute(".drawer", "data-open") is None)
    check(
        "81 focus goes back where it came from",
        await page.evaluate("() => document.activeElement?.classList.contains('row')"),
        await page.evaluate("() => document.activeElement?.className"),
    )
    check("82 the sheet produced no page errors", ERRORS[page] == [], ERRORS[page])
    await page.context.close()


async def queue_motion(browser) -> None:
    """Reduced motion has to reach the queue too, including the per-row entrance stagger."""
    page = await fresh(browser, motion="reduce")
    await page.evaluate(DROP, ["one.mov", "two.mov", "three.mov"])
    await page.wait_for_function("() => document.querySelectorAll('.row').length === 3")
    delays = await page.evaluate(
        "() => [...document.querySelectorAll('.row')].map((r) => getComputedStyle(r).animationDelay)"
    )
    durations = await page.evaluate(
        "() => [...document.querySelectorAll('.row')].map((r) => getComputedStyle(r).animationDuration)"
    )
    check("83 reduced motion drops the row stagger", set(delays) == {"0s"}, delays)
    check(
        "84 reduced motion collapses the row entrance",
        all(float(d.replace("s", "")) <= 0.001 for d in durations),
        durations,
    )
    await page.context.close()


# ============================================================== helper apps, and installing them ==
#
# The mock's preview knobs are what make the sad paths reachable in a browser tab: `?missing=` says
# which helpers are absent, `?nobrew` removes Homebrew itself, `?installfail=pandoc` makes that
# install fail, `?installms=` sets how fast the log streams. See `readKnobs` in src/lib/mock.ts.

TOOL_STATES = """
() => [...document.querySelectorAll('.tool')].map((t) =>
  [t.dataset.tool, t.querySelector('.tool__state').textContent])
"""

LOG_TEXT = "() => document.querySelector('.install__log')?.textContent ?? ''"

# The rows that offer something to install, and the name each one says out loud. A *package* is what
# a user installs, so three Poppler binaries are one row called "Poppler" — the list must not grow a
# row per binary, and no row may name one.
INSTALLABLES = """
() => [...document.querySelectorAll('.tool[data-installable]')].map((t) =>
  [t.dataset.tool, t.querySelector('.tool__label').textContent])
"""

# The docx row's route to PDF: disabled while LibreOffice is missing, live once it is there. This is
# the assertion that a successful install really did reach the catalog and not just the tool list.
PDF_OPTION = """
() => {
  const select = document.querySelector('.row .target__select');
  if (select === null) return null;
  const option = [...select.options].find((o) => o.value === 'pdf');
  return option === undefined ? null : { disabled: option.disabled, text: option.textContent };
}
"""


def tool(tool_id: str) -> str:
    return f'.tool[data-tool="{tool_id}"]'


async def open_settings(page: Page) -> None:
    await page.keyboard.press("Control+,")
    await page.wait_for_selector(".tools__list")
    await page.wait_for_timeout(450)


async def helpers_quiet(browser) -> None:
    """Nothing missing: the list is reassurance, so it must not open a single detail block."""
    page = await fresh(browser, query="?missing=")
    await open_settings(page)
    states = await page.evaluate(TOOL_STATES)
    check(
        "85 with every helper present the list stays one line per helper",
        await page.eval_on_selector_all(".tool__detail", "els => els.length") == 0
        and await page.eval_on_selector_all(".toolbutton", "els => els.length") == 0,
        states,
    )
    check(
        "86 and each one just says Included or Found",
        len(states) > 4 and {s for _, s in states} <= {"Included", "Found"},
        states,
    )
    installables = await page.evaluate(INSTALLABLES)
    check(
        "87 exactly seven things can be installed, and each is named as the user would buy it",
        [i for i, _ in installables]
        == ["libreoffice", "pandoc", "imagemagick", "poppler", "ruffle", "yt-dlp", "deno"]
        and [n for _, n in installables]
        == ["LibreOffice", "Pandoc", "ImageMagick", "Poppler", "Ruffle", "yt-dlp", "Deno"]
        and not any("pdfto" in n for _, n in installables)
        # Node.js is a helper the app *uses* and never installs: it belongs to no package, so it has
        # no plan, so this list cannot contain it however the machine looks. Asserted here because
        # "what can be installed" is the question this list answers, and Node's absence from it
        # while it is still a row above is the whole of the claim.
        and any(i == "node" for i, _ in states)
        and not any(i == "node" for i, _ in installables),
        installables,
    )
    check("88 the quiet list produced no page errors", ERRORS[page] == [], ERRORS[page])
    await page.context.close()


async def helper_install(browser) -> None:
    """The happy path, end to end: what it unlocks → Install → streamed log → the format works."""
    page = await fresh(browser, query="?missing=libreoffice,pandoc&installms=90")
    await page.context.grant_permissions(["clipboard-read", "clipboard-write"])
    await page.evaluate(DROP, ["quarterly-report.docx"])
    await page.wait_for_selector(".row")
    # `PDF_OPTION` reads the row picker's option list, which is built when the picker is opened
    # (223 onwards). Opened once here, it stays open for the rest of this walk — including the wait
    # on the `pdf` option coming back to life at 105.
    await prime_picker(page)

    before = await page.evaluate(PDF_OPTION)
    check(
        "89 while LibreOffice is missing the docx row cannot reach PDF, and says why",
        before is not None and before["disabled"] and "LibreOffice" in before["text"],
        before,
    )

    await open_settings(page)
    unlocks = await page.text_content(f"{tool('libreoffice')} .tool__unlocks")
    check(
        "90 a missing helper says what it unlocks in plain words, not format ids",
        "Open and save documents" in (unlocks or "") and "PDF" in (unlocks or ""),
        unlocks,
    )
    caution = await page.text_content(f"{tool('libreoffice')} .tool__caution")
    check(
        "91 a needs_admin helper warns about the password before the click",
        "password" in (caution or "") and "Terminal" in (caution or ""),
        caution,
    )
    command = await page.text_content(f"{tool('libreoffice')} .tool__hint")
    check(
        "92 the exact command is shown as the fallback that always works",
        command == "brew install --cask libreoffice",
        command,
    )
    await page.click(f"{tool('libreoffice')} .tool__command .microlink")
    await page.wait_for_timeout(200)
    check(
        "93 Copy copies that command, verbatim",
        await page.evaluate("() => navigator.clipboard.readText()") == command
        and await page.text_content(f"{tool('libreoffice')} .tool__command .microlink") == "Copied",
        await page.evaluate("() => navigator.clipboard.readText()"),
    )

    await page.click(f"{tool('libreoffice')} .toolbutton")
    await page.wait_for_selector(".install__log")
    await page.wait_for_function(
        "() => (document.querySelector('.install__log')?.textContent ?? '').split('\\n').length >= 4"
    )
    state = await page.text_content(".install__state")
    check(
        "94 the install streams the installer's own output while it runs",
        "==>" in await page.evaluate(LOG_TEXT),
        (await page.evaluate(LOG_TEXT)).split("\n")[0],
    )
    check(
        "95 and says out loud that it is running, and that it will take a while",
        (state or "").startswith("Installing LibreOffice") and "minutes" in (state or ""),
        state,
    )
    check(
        "96 the status line is a live region, so a screen reader hears it",
        await page.get_attribute(".install__state", "role") == "status"
        and await page.get_attribute(".install__state", "aria-live") == "polite",
        await page.get_attribute(".install__state", "aria-live"),
    )
    log = await page.evaluate(
        """
        () => {
          const el = document.querySelector('.install__log');
          const s = getComputedStyle(el);
          return {
            mono: s.fontFamily.toLowerCase(),
            overflow: s.overflowY,
            height: el.clientHeight,
            scrollable: el.scrollHeight > el.clientHeight,
            atBottom: el.scrollHeight - el.clientHeight - el.scrollTop <= 2,
            focusable: el.tabIndex === 0,
            role: el.getAttribute('role'),
          };
        }
        """
    )
    check(
        "97 the log is a small scrollable monospace pane, not a terminal emulator",
        ("mono" in log["mono"] or "menlo" in log["mono"])
        and log["overflow"] == "auto"
        and log["height"] <= 120
        and log["scrollable"],
        log,
    )
    check("98 it auto-scrolls to the newest line", log["atBottom"], log)
    check(
        "99 and the keyboard can reach it to read back",
        log["focusable"] and log["role"] == "log",
        log,
    )
    others = await page.evaluate(
        "() => [...document.querySelectorAll('.tool')]"
        ".filter((t) => t.dataset.tool !== 'libreoffice')"
        ".flatMap((t) => [...t.querySelectorAll('.toolbutton')]).map((b) => b.disabled)"
    )
    check(
        "100 one install at a time: every other Install is disabled while this one runs",
        len(others) > 0 and all(others),
        others,
    )
    check(
        "101 and the other helpers say why they are disabled rather than failing silently",
        await page.evaluate(
            "() => [...document.querySelectorAll('.tool__waiting')]"
            ".some((s) => s.textContent === 'Another helper is installing')"
        ),
    )
    check(
        "102 the running helper's own button reads Installing… and cannot be clicked again",
        await page.evaluate(
            "() => { const b = document.querySelector('.tool[data-tool=\"libreoffice\"] .toolbutton');"
            " return b.textContent === 'Installing…' && b.disabled; }"
        ),
    )

    await page.wait_for_selector('.install[data-status="ok"]', timeout=30000)
    check(
        "103 success is stated in the backend's own words",
        (await page.text_content(".install__state"))
        == "LibreOffice is installed. Flint can use it now.",
        await page.text_content(".install__state"),
    )
    await page.wait_for_function(
        "() => document.querySelector('.tool[data-tool=\"libreoffice\"] .tool__state')"
        ".textContent === 'Found'",
        timeout=15000,
    )
    check(
        "104 the helper becomes Found and stops being something to install",
        await page.query_selector(f"{tool('libreoffice')} .toolbutton") is None,
        await page.evaluate(TOOL_STATES),
    )
    await page.wait_for_function(
        "() => { const s = document.querySelector('.row .target__select');"
        " const o = [...s.options].find((x) => x.value === 'pdf'); return o && !o.disabled; }",
        timeout=15000,
    )
    after = await page.evaluate(PDF_OPTION)
    check(
        "105 and the format it unlocked is selectable, with the warning gone from its label",
        after is not None and not after["disabled"] and "needs" not in after["text"],
        after,
    )
    check("106 the whole install produced no page errors", ERRORS[page] == [], ERRORS[page])
    await page.context.close()


async def helper_install_fails(browser) -> None:
    """A failed install must hand the user the installer's last words, not a shrug."""
    page = await fresh(browser, query="?missing=pandoc&installfail=pandoc&installms=40")
    await open_settings(page)
    await page.click(f"{tool('pandoc')} .toolbutton")
    await page.wait_for_selector('.install[data-status="failed"]', timeout=30000)

    message = await page.text_content(".install__state")
    check(
        "107 a failed install surfaces the backend's message verbatim",
        "Could not install Pandoc" in (message or "")
        and 'No available formula with the name "pandoc"' in (message or "")
        and "in Terminal" in (message or ""),
        message,
    )
    check(
        "108 the failure keeps the log open, so the last brew line is right there",
        "Please tap Homebrew/core" in await page.evaluate(LOG_TEXT),
        (await page.evaluate(LOG_TEXT)).split("\n")[-1],
    )
    check(
        "109 the helper is still Missing and Install is offered again",
        await page.text_content(f"{tool('pandoc')} .tool__state") == "Missing"
        and await page.evaluate(
            "() => { const b = document.querySelector('.tool[data-tool=\"pandoc\"] .toolbutton');"
            " return b.textContent === 'Install' && !b.disabled; }"
        ),
    )
    check(
        "110 a failure never quietly marks the helper as present",
        await page.get_attribute(tool("pandoc"), "data-available") == "false"
        and await page.get_attribute(tool("pandoc"), "data-missing") is not None,
        await page.get_attribute(tool("pandoc"), "data-available"),
    )
    await page.evaluate(
        "() => [...document.querySelectorAll('.install__meta .microlink')]"
        ".find((b) => b.textContent === 'Dismiss').click()"
    )
    await page.wait_for_timeout(200)
    check(
        "111 Dismiss puts the helper back to its quiet missing state",
        await page.query_selector(".install") is None
        and await page.query_selector(f"{tool('pandoc')} .tool__hint") is not None,
    )
    check("112 the failed install produced no page errors", ERRORS[page] == [], ERRORS[page])
    await page.context.close()


async def helper_without_homebrew(browser) -> None:
    """No Homebrew: say so once, show its command, and never offer a button we cannot honour."""
    page = await fresh(browser, query="?missing=libreoffice,pandoc&nobrew")
    await open_settings(page)

    check(
        "113 without Homebrew there is no Install button anywhere",
        await page.eval_on_selector_all(".toolbutton", "els => els.length") == 0,
    )
    managers = await page.eval_on_selector_all(".tools__manager", "els => els.length")
    text = await page.text_content(".tools__managertext")
    check(
        "114 the reason is given once for the whole list, not on every row",
        managers == 1 and "Homebrew is not installed" in (text or ""),
        f"{managers} notice(s): {text}",
    )
    check(
        "115 and it does not pretend the app can install Homebrew itself",
        "cannot install these for you" in (text or "") and "brew.sh" in (text or ""),
        text,
    )
    brew_command = await page.text_content(".tools__manager .tool__hint")
    check(
        "116 Homebrew's own command is there to copy",
        (brew_command or "").startswith("/bin/bash -c") and "install.sh" in (brew_command or ""),
        brew_command,
    )
    check(
        "117 every missing helper still shows the command that would fix it by hand",
        await page.evaluate(
            "() => [...document.querySelectorAll('.tool[data-missing]')]"
            ".every((t) => (t.querySelector('.tool__hint')?.textContent ?? '').startsWith('brew '))"
        ),
        await page.evaluate(
            "() => [...document.querySelectorAll('.tool[data-missing] .tool__hint')]"
            ".map((c) => c.textContent)"
        ),
    )
    check("118 the Homebrew-missing state produced no page errors", ERRORS[page] == [], ERRORS[page])
    await page.context.close()


async def failed_row_to_helper(browser) -> None:
    """The join that matters: from the row that failed to the helper that fixes it."""
    page = await fresh(browser, query="?missing=libreoffice&installms=40")
    await page.evaluate(DROP, ["quarterly-report.docx", "broken-take.mov"])
    await page.wait_for_function("() => document.querySelectorAll('.row').length === 2")
    await page.click(".pill")
    await page.wait_for_function(
        "() => document.querySelector('.pill').textContent.startsWith('Convert')", timeout=60000
    )

    fixes = await page.evaluate(
        "() => [...document.querySelectorAll('.row')].map((r) =>"
        " [r.dataset.status, r.querySelector('.row__fix')?.textContent ?? null])"
    )
    check(
        "119 the row that failed for want of a helper names the helper",
        fixes[0] == ["failed", "Install LibreOffice"],
        fixes,
    )
    check(
        "120 a row that failed for its own reasons is left alone",
        fixes[1][0] == "failed" and fixes[1][1] is None,
        fixes,
    )
    # The batch also *asked* about LibreOffice on the way out (see 125 onwards). This section is
    # about the row's own route, so the question is answered with "Not now" and the row is used
    # exactly as a user who ignored the prompt would use it.
    await page.click(".prompt .microlink")
    await page.wait_for_timeout(300)
    await page.click(".row__fix")
    await page.wait_for_selector(".tools__list")
    await page.wait_for_timeout(500)
    check(
        "121 it opens settings on that helper, with the keyboard already on Install",
        await page.get_attribute(tool("libreoffice"), "data-highlight") is not None
        and await page.evaluate("() => document.activeElement?.getAttribute('aria-label')")
        == "Install LibreOffice",
        await page.evaluate("() => document.activeElement?.getAttribute('aria-label')"),
    )
    check(
        "122 one helper is singled out, not the whole list",
        await page.eval_on_selector_all(".tool[data-highlight]", "els => els.length") == 1,
    )
    await page.keyboard.press("Escape")
    await page.wait_for_timeout(500)
    await page.keyboard.press("Control+,")
    await page.wait_for_timeout(500)
    check(
        "123 and the highlight belongs to that one trip, not to every later visit",
        await page.eval_on_selector_all(".tool[data-highlight]", "els => els.length") == 0,
    )
    check("124 the row-to-helper path produced no page errors", ERRORS[page] == [], ERRORS[page])
    await page.context.close()


async def a_package_is_what_the_user_installs(browser) -> None:
    """The binary is ours to know; the package is what the user is told about.

    `pdftohtml` is the executable PDF → HTML needs, and naming it is right in a diagnostic and wrong
    in the sentence a person reads: nobody installs `pdftohtml`, they install Poppler. This is that
    conversion failing on a machine that has `pdftoppm` but neither of the other two Poppler
    binaries — which is also the awkward half-installed package, still offerable and still not
    "Found".
    """
    page = await fresh(browser, query="?missing=libreoffice,pdftotext,pdftohtml")
    await page.evaluate(DROP, ["report.pdf"])
    await page.wait_for_selector(".row")
    # `select_option` sets the option it finds without ever touching the control, so the list has to
    # be asked for first — see `prime_picker`.
    await prime_picker(page)
    await page.select_option(".row .target__select", "html")
    await convert_and_settle(page)

    message = await page.text_content(".row__error")
    check(
        "125 a PDF → HTML failure names the package, never the binary it could not run",
        (await page.evaluate(STATUSES)) == ["failed"]
        and "needs Poppler installed" in (message or "")
        and "pdfto" not in (message or ""),
        message,
    )

    await open_settings(page)
    row = await page.evaluate(
        "() => { const t = document.querySelector('.tool[data-tool=\"poppler\"]');"
        " return t === null ? null : { state: t.querySelector('.tool__state').textContent,"
        " available: t.dataset.available, missing: t.dataset.missing ?? null,"
        " button: t.querySelector('.toolbutton')?.textContent ?? null,"
        " command: t.querySelector('.tool__hint')?.textContent ?? null }; }"
    )
    check(
        "126 a half-installed package does not claim to be installed, and can still be installed",
        row is not None
        and row["state"] == "Incomplete"
        and row["available"] == "false"
        and row["missing"] is not None
        and row["button"] == "Install"
        and row["command"] == "brew install poppler"
        and ERRORS[page] == [],
        [row, ERRORS[page]],
    )
    await page.context.close()


async def install_motion(browser) -> None:
    """The install's proof of life is an animation, so reduced motion has to reach it too."""
    page = await fresh(browser, motion="reduce", query="?missing=pandoc&installms=1200")
    await open_settings(page)
    await page.click(f"{tool('pandoc')} .toolbutton")
    await page.wait_for_selector(".tool__pulse")
    sweep = await page.evaluate(
        "() => getComputedStyle(document.querySelector('.tool__pulse'), '::after')"
        ".animationDuration"
    )
    check(
        "127 reduced motion collapses the install pulse",
        float(sweep.replace("s", "")) <= 0.001,
        sweep,
    )
    await page.context.close()



PROMPT_SEMANTICS = """
() => {
  const card = document.querySelector('.prompt');
  if (card === null) return null;
  const label = document.getElementById(card.getAttribute('aria-labelledby') ?? '');
  const body = document.getElementById(card.getAttribute('aria-describedby') ?? '');
  return {
    role: card.getAttribute('role'),
    modal: card.getAttribute('aria-modal'),
    label: label?.textContent ?? null,
    body: body?.textContent ?? null,
    cards: document.querySelectorAll('.prompt').length,
    native: window.__ceNativeConfirms ?? 0,
  };
}
"""

PROMPT_ITEMS = """
() => [...document.querySelectorAll('.prompt__item')].map((li) =>
  [li.querySelector('.prompt__stake').textContent, li.querySelector('.prompt__count').textContent])
"""

# `window.confirm` is what this feature must never be. Counting calls is cheaper than trusting that
# a dialog we can see is the one we built.
COUNT_NATIVE = """
() => {
  window.__ceNativeConfirms = 0;
  window.confirm = () => { window.__ceNativeConfirms += 1; return false; };
  window.alert = () => { window.__ceNativeConfirms += 1; };
}
"""

PROMPT_LOOK = """
() => {
  const card = document.querySelector('.prompt');
  const veil = document.querySelector('.promptveil');
  const cs = getComputedStyle(card);
  const vs = getComputedStyle(veil);
  const box = card.getBoundingClientRect();
  const surface = getComputedStyle(document.documentElement).getPropertyValue('--surface').trim();
  return {
    borders: [cs.borderTopWidth, cs.borderRightWidth, cs.borderBottomWidth, cs.borderLeftWidth],
    style: cs.borderTopStyle,
    background: cs.backgroundColor,
    surface,
    transform: cs.transform,
    animation: vs.animationName,
    iterations: vs.animationIterationCount,
    duration: vs.animationDuration,
    blurred: (vs.backdropFilter || vs.webkitBackdropFilter || '').includes('blur'),
    centred: Math.abs((box.x + box.width / 2) - window.innerWidth / 2) <= 1,
    accent: getComputedStyle(document.querySelector('.promptbutton')).borderTopColor,
    width: box.width,
  };
}
"""


async def convert_and_settle(page: Page) -> None:
    """Run the queue and wait for the batch to be over, prompt or no prompt."""
    await page.click(".pill")
    await page.wait_for_function(
        "() => document.querySelector('.pill').textContent.startsWith('Convert')", timeout=60000
    )
    await page.wait_for_timeout(700)


async def helper_prompt(browser) -> None:
    """The question itself: raised once, said in the app's own voice, and dismissible two ways."""
    page = await fresh(browser, query="?missing=libreoffice,magick")
    await page.evaluate(COUNT_NATIVE)
    await page.evaluate(DROP, ["notes.md", "spec.md", "changelog.md"])
    await page.wait_for_function("() => document.querySelectorAll('.row').length === 3")
    await page.click(".pill")
    await page.wait_for_selector(".prompt", timeout=60000)
    await page.wait_for_timeout(400)

    semantics = await page.evaluate(PROMPT_SEMANTICS)
    check(
        "128 a batch blocked by a missing helper asks about it, in a real dialog",
        semantics is not None
        and semantics["role"] == "dialog"
        and semantics["modal"] == "true"
        and (semantics["label"] or "") != ""
        and semantics["native"] == 0,
        semantics,
    )
    check(
        "129 it names the helper and the stake, and counts what it cost",
        "3 files" in (semantics["label"] or "")
        and "couldn’t be converted" in (semantics["label"] or "")
        and "PDF needs LibreOffice" in (semantics["body"] or ""),
        [semantics["label"], semantics["body"]],
    )
    check(
        "130 and it promises the install is still the user's to start",
        # The copy keeps "click Install" on one line with an nbsp; compare against plain spaces.
        "Nothing is installed until you click Install"
        in (semantics["body"] or "").replace("\u00a0", " ")
        and (await page.text_content(".promptbutton") or "").startswith("Install LibreOffice"),
        await page.text_content(".promptbutton"),
    )

    look = await page.evaluate(PROMPT_LOOK)
    check(
        "131 it is a hairline card on a dimmed, blurred window, not a browser alert",
        set(look["borders"]) == {"1px"}
        and look["style"] == "solid"
        and look["background"] != "rgba(0, 0, 0, 0)"
        and look["blurred"]
        and look["centred"]
        and look["width"] <= 380,
        look,
    )
    check(
        "132 one soft fade carries it in, and nothing moves",
        look["animation"] == "fade-in"
        and look["iterations"] == "1"
        and 0.24 <= float(look["duration"].replace("s", "")) <= 0.42
        and look["transform"] in ("none", "matrix(1, 0, 0, 1, 0, 0)"),
        look,
    )
    check(
        "133 the primary action is the accent action in it",
        rgba(look["accent"])[2] > rgba(look["accent"])[0] + 40,
        look["accent"],
    )

    check(
        "134 the keyboard starts on the primary action",
        await page.evaluate("() => document.activeElement?.className") == "promptbutton",
        await page.evaluate("() => document.activeElement?.className"),
    )
    for _ in range(6):
        await page.keyboard.press("Tab")
    check(
        "135 Tab cannot walk out of the card",
        await page.evaluate("() => document.querySelector('.prompt').contains(document.activeElement)"),
        await page.evaluate("() => document.activeElement?.className"),
    )

    await page.keyboard.press("Escape")
    await page.wait_for_timeout(400)
    check("136 Esc dismisses it", await page.query_selector(".prompt") is None)
    check(
        "137 and focus goes back to whatever held it before",
        await page.evaluate("() => document.activeElement?.classList.contains('pill')"),
        await page.evaluate("() => document.activeElement?.className"),
    )

    # Told once is enough. The same batch, run again, must not raise the same question.
    await convert_and_settle(page)
    check(
        "138 a helper that has been asked about is never asked about again",
        await page.query_selector(".prompt") is None,
        await page.text_content(".prompt__title") if await page.query_selector(".prompt") else "",
    )
    check(
        "139 the row's own microlink is still the way in",
        await page.evaluate(
            "() => [...document.querySelectorAll('.row__fix')].every((b) =>"
            " b.textContent === 'Install LibreOffice')"
        )
        and await page.eval_on_selector_all(".row__fix", "els => els.length") == 3,
        await page.eval_on_selector_all(".row__fix", "els => els.map((b) => b.textContent)"),
    )

    # A *different* missing helper is a different question, and it still gets asked.
    await page.evaluate(DROP, ["logo.svg"])
    await page.wait_for_function("() => document.querySelectorAll('.row').length === 4")
    await page.click(".pill")
    await page.wait_for_selector(".prompt", timeout=60000)
    await page.wait_for_timeout(300)
    body = await page.text_content(".prompt__body")
    check(
        "140 a helper nobody has been asked about yet still asks",
        "ImageMagick" in (body or "") and "LibreOffice" not in (body or ""),
        body,
    )
    await page.click(".prompt .microlink")
    await page.wait_for_timeout(400)
    check(
        "141 Not now dismisses it, and takes nothing else with it",
        await page.query_selector(".prompt") is None
        and await page.eval_on_selector_all(".row", "els => els.length") == 4,
    )
    check("142 the whole exchange produced no page errors", ERRORS[page] == [], ERRORS[page])
    await page.context.close()


async def helper_prompt_once(browser) -> None:
    """Ten failing files are one missing helper, and therefore one question."""
    page = await fresh(browser, query="?missing=libreoffice")
    await page.evaluate(DROP, [f"chapter-{n}.md" for n in range(1, 11)])
    await page.wait_for_function("() => document.querySelectorAll('.row').length === 10")
    await page.click(".pill")
    await page.wait_for_selector(".prompt", timeout=60000)
    await page.wait_for_timeout(500)
    check(
        "143 ten files blocked by one helper raise exactly one prompt, for all ten",
        await page.eval_on_selector_all(".prompt", "els => els.length") == 1
        and "10 files" in (await page.text_content(".prompt__title") or ""),
        await page.text_content(".prompt__title"),
    )
    check("144 the ten-file batch produced no page errors", ERRORS[page] == [], ERRORS[page])
    await page.context.close()


async def helper_prompt_aggregates(browser) -> None:
    """Two helpers, one question: each named with its own toll, worst blocker in the button."""
    page = await fresh(browser, query="?missing=libreoffice,magick")
    await page.evaluate(DROP, ["notes.md", "spec.md", "changelog.md", "logo.svg"])
    await page.wait_for_function("() => document.querySelectorAll('.row').length === 4")
    await page.click(".pill")
    await page.wait_for_selector(".prompt", timeout=60000)
    await page.wait_for_timeout(400)

    items = await page.evaluate(PROMPT_ITEMS)
    check(
        "145 two missing helpers are aggregated into one question, each with its file count",
        len(items) == 2
        and items[0] == ["PDF needs LibreOffice", "3 files"]
        and items[1] == ["SVG (vector) needs ImageMagick", "1 file"],
        items,
    )
    check(
        "146 the total counts every blocked file, across both helpers",
        "4 files" in (await page.text_content(".prompt__title") or ""),
        await page.text_content(".prompt__title"),
    )
    check(
        "147 and the primary action targets whichever helper is blocking the most files",
        (await page.text_content(".promptbutton") or "").startswith("Install LibreOffice"),
        await page.text_content(".promptbutton"),
    )
    check("148 the aggregated question produced no page errors", ERRORS[page] == [], ERRORS[page])
    await page.context.close()


async def helper_prompt_to_settings(browser) -> None:
    """Yes is the row microlink's trip, said out loud — and it installs nothing on the way."""
    page = await fresh(browser, query="?missing=libreoffice")
    await page.evaluate(DROP, ["notes.md", "spec.md"])
    await page.wait_for_function("() => document.querySelectorAll('.row').length === 2")
    await page.click(".pill")
    await page.wait_for_selector(".prompt", timeout=60000)
    await page.wait_for_timeout(300)
    # Enter, not a click: the primary action already has the keyboard, so Return is the whole
    # interaction — and it is the one a user who never touches the trackpad will use.
    await page.keyboard.press("Enter")
    await page.wait_for_selector(".tool[data-highlight]")
    await page.wait_for_timeout(700)

    check(
        "149 Enter confirms: the card closes, settings opens on that helper, Install has the keyboard",
        await page.query_selector(".prompt") is None
        and await page.get_attribute(".drawer", "data-open") is not None
        and await page.get_attribute(tool("libreoffice"), "data-highlight") is not None
        and await page.eval_on_selector_all(".tool[data-highlight]", "els => els.length") == 1
        and await page.evaluate("() => document.activeElement?.getAttribute('aria-label')")
        == "Install LibreOffice",
        await page.evaluate("() => document.activeElement?.getAttribute('aria-label')"),
    )
    check(
        "150 and nothing has been installed: the button is still there, unpressed",
        await page.query_selector(".install") is None
        and await page.text_content(f"{tool('libreoffice')} .tool__state") == "Missing"
        and await page.evaluate(
            "() => { const b = document.querySelector('.tool[data-tool=\"libreoffice\"] .toolbutton');"
            " return b.textContent === 'Install' && !b.disabled; }"
        ),
        await page.text_content(f"{tool('libreoffice')} .tool__state"),
    )
    check("151 the trip into settings produced no page errors", ERRORS[page] == [], ERRORS[page])
    await page.context.close()


async def helper_prompt_stays_quiet(browser) -> None:
    """The four states in which asking would be noise, or a lie."""
    # 1. A deliberate stop is not a failure.
    page = await fresh(browser, query="?missing=libreoffice")
    await page.evaluate(DROP, ["notes.md", "spec.md", "keynote-recording.mov"])
    await page.wait_for_function("() => document.querySelectorAll('.row').length === 3")
    await page.click(".pill")
    await page.wait_for_selector('.row[data-status="failed"]', timeout=60000)
    if await page.evaluate("() => document.querySelector('.pill').textContent") == "Stop":
        await page.click(".pill")
    await page.wait_for_function(
        "() => document.querySelector('.pill').textContent.startsWith('Convert')", timeout=60000
    )
    await page.wait_for_timeout(700)
    check(
        "152 a batch the user stopped is never answered with a prompt",
        await page.query_selector(".prompt") is None,
        await page.evaluate(STATUSES),
    )
    await page.context.close()

    # 2. A failure with no missing helper behind it gets the row's message and nothing more.
    page = await fresh(browser, query="?missing=libreoffice")
    await page.evaluate(DROP, ["broken-take.mov"])
    await page.wait_for_selector(".row")
    await convert_and_settle(page)
    check(
        "153 a failure with no missing helper behind it raises nothing",
        await page.query_selector(".prompt") is None
        and await page.evaluate(STATUSES) == ["failed"],
        await page.evaluate(STATUSES),
    )
    check("154 and it produced no page errors either", ERRORS[page] == [], ERRORS[page])
    await page.context.close()

    # 3. An install already running is about to answer the question by itself.
    page = await fresh(browser, query="?missing=libreoffice&installms=1500")
    await open_settings(page)
    await page.click(f"{tool('libreoffice')} .toolbutton")
    await page.wait_for_selector(".install__log")
    await page.keyboard.press("Escape")
    await page.wait_for_timeout(400)
    await page.evaluate(DROP, ["notes.md"])
    await page.wait_for_selector(".row")
    await convert_and_settle(page)
    check(
        "155 no prompt while that helper is already installing",
        await page.query_selector(".prompt") is None
        # The precondition: the install really was still in flight when the batch settled.
        and await page.evaluate("() => document.querySelector('.install')?.dataset.status")
        == "running",
        await page.evaluate("() => document.querySelector('.install')?.dataset.status"),
    )
    await page.context.close()

    # 4. Settings already open: point at the helper where the user is looking.
    page = await fresh(browser, query="?missing=libreoffice")
    await page.evaluate(DROP, ["notes.md", "spec.md"])
    await page.wait_for_function("() => document.querySelectorAll('.row').length === 2")
    await open_settings(page)
    await page.keyboard.press("Control+Enter")
    await page.wait_for_function(
        "() => document.querySelector('.pill').textContent.startsWith('Convert')", timeout=60000
    )
    await page.wait_for_timeout(700)
    check(
        "156 with settings already open the helper is highlighted in place, with no modal over it",
        await page.query_selector(".prompt") is None
        and await page.get_attribute(tool("libreoffice"), "data-highlight") is not None
        and await page.eval_on_selector_all(".tool[data-highlight]", "els => els.length") == 1,
        await page.eval_on_selector_all(".tool[data-highlight]", "els => els.length"),
    )
    check("157 none of the quiet paths produced page errors", ERRORS[page] == [], ERRORS[page])
    await page.context.close()


async def prompt_hands_the_keyboard_back(browser) -> None:
    """The card's keyboard handling is the card's, and it leaves with it.

    The prompt confirms on Return and intercepts ⌫ so the queue behind it cannot be edited through
    the scrim. Both belong to the modal: once it has been and gone, plain Return must again be the
    empty state's own "open the chooser" — which is assertion 5, one page later. This is that
    assertion asked of a window that has had the prompt up, which is the state it can regress in.
    """
    page = await fresh(browser, query="?missing=libreoffice")
    await page.evaluate(DROP, ["notes.md", "spec.md"])
    await page.wait_for_function("() => document.querySelectorAll('.row').length === 2")
    await page.click(".pill")
    await page.wait_for_selector(".prompt", timeout=60000)
    await page.keyboard.press("Escape")
    await page.wait_for_function("() => document.querySelector('.prompt') === null")
    # ⇧⌘⌫ empties the queue, so the window is back to the one click target it started as.
    await page.keyboard.press("Control+Shift+Backspace")
    await page.wait_for_selector(".dropzone")
    await page.focus(".dropzone")
    check(
        "158 the drop state answers Enter again once the prompt has been and gone",
        await opens_chooser(page, lambda: page.keyboard.press("Enter"))
        and ERRORS[page] == [],
        ERRORS[page],
    )
    await page.context.close()


# ============================================================== work this window did not start ==
#
# A webview reload throws away the store and every listener, but not the batch or the install: Rust
# holds one slot for each, and the window that comes back cannot see either of them. `get_activity`
# is the one question that finds them, `?activity=` is how the mock stages the answer, and the
# events that follow carry row ids from the page load that is gone.

INHERITED_STATUS = "() => document.querySelector('.actionbar__text')?.textContent ?? null"
TOAST = "() => document.querySelector('.toast__text')?.textContent ?? null"
CLEAR_ALL = (
    "() => [...document.querySelectorAll('.filelist__header .microlink')]"
    ".find((b) => b.textContent === 'Clear all')"
)


async def inherited_batch(browser) -> None:
    """The state a reload lands in: the shell is still converting rows this page has no rows for."""
    page = await fresh(browser, query="?activity=converting")
    check(
        "159 a window that reloaded mid-batch says so, and offers Stop for a queue it cannot show",
        await page.evaluate("() => document.querySelector('.pill')?.textContent") == "Stop"
        and "started before this window" in (await page.evaluate(INHERITED_STATUS) or ""),
        [await page.evaluate("() => document.querySelector('.pill')?.textContent"),
         await page.evaluate(INHERITED_STATUS)],
    )
    check(
        "160 and draws no aggregate bar, having nothing of its own to measure",
        await page.query_selector(".actionbar__progress") is None,
    )

    # The inherited batch has been emitting `started`/`progress` for an id from before the reload
    # since the page loaded. None of it may become a row.
    await page.evaluate(DROP, ["one.mov", "two.mov"])
    await page.wait_for_function("() => document.querySelectorAll('.row').length === 2")
    await page.wait_for_timeout(700)
    check(
        "161 the ids in that stream belong to a page that is gone: no phantom rows, no crash",
        await page.eval_on_selector_all(".row", "els => els.length") == 2
        and await page.evaluate(STATUSES) == ["queued", "queued"]
        and ERRORS[page] == [],
        [await page.evaluate(STATUSES), ERRORS[page]],
    )
    check(
        "162 the queue is still the user's while that batch runs: Clear all is live and clears it",
        await page.evaluate(f"{CLEAR_ALL}.disabled") is False,
    )
    await page.evaluate(f"{CLEAR_ALL}.click()")
    await page.wait_for_timeout(300)
    check(
        "163 and emptying it does not pretend the conversion stopped",
        await page.eval_on_selector_all(".row", "els => els.length") == 0
        and await page.evaluate("() => document.querySelector('.pill')?.textContent") == "Stop",
        await page.evaluate("() => document.querySelector('.pill')?.textContent"),
    )

    await page.click(".pill")
    await page.wait_for_function(
        "() => (document.querySelector('.pill')?.textContent ?? 'Convert').startsWith('Convert')",
        timeout=30000,
    )
    check(
        "164 Stop really does stop it, and hands the empty window back",
        await page.query_selector(".dropzone") is not None
        and await page.query_selector(".actionbar") is None
        and ERRORS[page] == [],
        ERRORS[page],
    )
    await page.context.close()


async def stale_activity(browser) -> None:
    """`get_activity` is a snapshot, and `?activity=stale` is one that was already out of date.

    The work ended between the shell answering and the store adopting the answer, so its last event
    went to a window that was not listening yet. Adopting that leaves a Stop nothing can clear and
    an Install button disabled for the rest of the session, which is worse than never asking.
    """
    page = await fresh(browser, query="?activity=stale&missing=libreoffice,pandoc")
    check(
        "165 a conversion that had already ended is not adopted",
        await page.query_selector(".actionbar") is None,
        await page.evaluate(INHERITED_STATUS),
    )
    await page.evaluate(DROP, ["one.mov"])
    await page.wait_for_selector(".row")
    check(
        "166 so the window can convert: the primary action is Convert, not a Stop nobody can clear",
        (await page.evaluate("() => document.querySelector('.pill').textContent") or "").startswith(
            "Convert"
        ),
        await page.evaluate("() => document.querySelector('.pill').textContent"),
    )
    await open_settings(page)
    check(
        "167 nor is an install that had already finished: every helper can still be installed",
        await page.evaluate("() => [...document.querySelectorAll('.toolbutton')].length") == 2
        and await page.evaluate(
            "() => [...document.querySelectorAll('.toolbutton')].every((b) => !b.disabled)"
        )
        and await page.eval_on_selector_all(".tool__waiting", "els => els.length") == 0,
        await page.evaluate("() => [...document.querySelectorAll('.toolbutton')].map((b) => b.disabled)"),
    )
    check("168 the stale answer produced no page errors", ERRORS[page] == [], ERRORS[page])
    await page.context.close()


# ============================================================ a destination still being chosen ==

OUTPUT_LOCATION = """
(value) => {
  const section = [...document.querySelectorAll('.section')]
    .find((s) => s.querySelector('.section__title').textContent === 'Output');
  section.open = true;
  const select = section.querySelector('select');
  select.value = value;
  select.dispatchEvent(new Event('change', { bubbles: true }));
}
"""


async def half_made_destination(browser) -> None:
    """"In a folder I choose" starts with no folder, and that is not the user making a mistake.

    `settings_store::check_output` refuses the whole settings object over it, so the app used to
    answer the click on the radio button with an error toast — and then silently drop every later
    edit for as long as the destination stayed unfinished.
    """
    page = await fresh(browser)
    await page.evaluate(DROP, ["one.mov"])
    await page.wait_for_selector(".row")
    await open_settings(page)
    await page.evaluate(OUTPUT_LOCATION, "custom")
    await page.wait_for_timeout(900)
    check(
        "169 choosing a custom folder does not scold the user before they can choose one",
        await page.evaluate(TOAST) is None
        and await page.text_content(".folderpick__path") == "No folder chosen",
        await page.evaluate(TOAST),
    )

    # An edit made while the destination is unfinished must not be answered with the destination's
    # refusal either — the setting is the user's, and it is the *save* that has to wait.
    await page.evaluate(
        "() => { const s = [...document.querySelectorAll('.section')]"
        ".find((x) => x.querySelector('.section__title').textContent === 'Output');"
        " const sel = [...s.querySelectorAll('select')][1];"
        " sel.value = 'overwrite'; sel.dispatchEvent(new Event('change', { bubbles: true })); }"
    )
    await page.wait_for_timeout(700)
    check("170 nor is the next setting the user changes", await page.evaluate(TOAST) is None,
          await page.evaluate(TOAST))

    await page.evaluate(OUTPUT_LOCATION, "subfolder")
    await page.wait_for_timeout(400)
    await page.fill(".control--text", "")
    await page.wait_for_timeout(900)
    check(
        "171 a subfolder name being retyped is unfinished, not wrong",
        await page.evaluate(TOAST) is None,
        await page.evaluate(TOAST),
    )
    await page.fill(".control--text", "Exports")
    await page.wait_for_timeout(900)
    await page.keyboard.press("Escape")
    await page.wait_for_timeout(400)
    check(
        "172 and the finished one takes effect, with nothing having been said in between",
        await page.evaluate(TOAST) is None
        and (await page.evaluate(INHERITED_STATUS) or "").endswith("/Exports"),
        [await page.evaluate(TOAST), await page.evaluate(INHERITED_STATUS)],
    )
    check("173 the half-made destination produced no page errors", ERRORS[page] == [], ERRORS[page])
    await page.context.close()


async def refused_batch_keeps_the_last_run(browser) -> None:
    """A `start_batch` the backend refuses changed nothing, so nothing in the queue may change.

    The rows are cleared and the phase set before the call, because the first `started` can arrive
    while `invoke` is still in flight — which is exactly why a refusal has to be rolled back.
    """
    page = await fresh(browser)
    await page.evaluate(DROP, ["broken-take.mov", "one.mov"])
    await page.wait_for_function("() => document.querySelectorAll('.row').length === 2")
    await convert_and_settle(page)
    tally = await page.evaluate(INHERITED_STATUS)
    check(
        "174 the run that did happen: one failure, one success, and a tally",
        await page.evaluate(STATUSES) == ["failed", "done"] and "1 converted" in (tally or ""),
        [await page.evaluate(STATUSES), tally],
    )

    # A destination `start_batch` will refuse: chosen deliberately, so its refusal is the user's to
    # see — unlike the save, which waits.
    await open_settings(page)
    await page.evaluate(OUTPUT_LOCATION, "custom")
    await page.wait_for_timeout(300)
    await page.keyboard.press("Escape")
    await page.wait_for_timeout(300)
    await page.evaluate(
        "() => [...document.querySelector('.row[data-status=\"failed\"]')"
        ".querySelectorAll('.microlink')].find((b) => b.textContent === 'Retry').click()"
    )
    await page.wait_for_timeout(700)
    check(
        "175 a refused retry leaves the row's verdict, and its message, exactly as they were",
        await page.evaluate(STATUSES) == ["failed", "done"]
        and "exited with status"
        in (await page.text_content('.row[data-status="failed"] .row__error') or ""),
        [await page.evaluate(STATUSES),
         await page.text_content('.row[data-status="failed"] .row__error')],
    )
    check(
        "176 and leaves the last run's tally standing",
        await page.evaluate(INHERITED_STATUS) == tally,
        [tally, await page.evaluate(INHERITED_STATUS)],
    )
    check(
        "177 the reason is said out loud, and the window is idle again",
        "Choose an output folder" in (await page.evaluate(TOAST) or "")
        and (await page.evaluate("() => document.querySelector('.pill').textContent") or "").startswith(
            "Convert"
        )
        and await page.query_selector(".actionbar__progress") is None,
        [await page.evaluate(TOAST),
         await page.evaluate("() => document.querySelector('.pill').textContent")],
    )
    check("178 the refused batch produced no page errors", ERRORS[page] == [], ERRORS[page])
    await page.context.close()


async def prompt_does_not_stack_with_settings(browser) -> None:
    """⌘, stays live under the card — the native menu bar cannot be disabled by a div."""
    page = await fresh(browser, query="?missing=libreoffice")
    await page.evaluate(DROP, ["notes.md", "spec.md"])
    await page.wait_for_function("() => document.querySelectorAll('.row').length === 2")
    await page.click(".pill")
    await page.wait_for_selector(".prompt", timeout=60000)
    await page.wait_for_timeout(300)
    await page.keyboard.press("Control+,")
    await page.wait_for_selector(".tool[data-highlight]")
    await page.wait_for_timeout(700)
    check(
        "179 opening settings answers the question instead of stacking a sheet behind it",
        await page.query_selector(".prompt") is None
        and await page.query_selector(".promptveil") is None
        and await page.get_attribute(".drawer", "data-open") is not None
        and await page.get_attribute(tool("libreoffice"), "data-highlight") is not None
        and await page.eval_on_selector_all(".tool[data-highlight]", "els => els.length") == 1,
        await page.eval_on_selector_all(".tool[data-highlight]", "els => els.length"),
    )
    check(
        "180 and the keyboard lands on that helper's Install button, unpressed",
        await page.evaluate("() => document.activeElement?.getAttribute('aria-label')")
        == "Install LibreOffice"
        and await page.query_selector(".install") is None,
        await page.evaluate("() => document.activeElement?.getAttribute('aria-label')"),
    )

    # Being taken there counts as having been asked: the same batch must not ask again.
    await page.keyboard.press("Escape")
    await page.wait_for_timeout(400)
    await convert_and_settle(page)
    check(
        "181 a helper the sheet was opened on is not asked about a second time",
        await page.query_selector(".prompt") is None,
        await page.text_content(".prompt__title") if await page.query_selector(".prompt") else "",
    )
    check("182 none of that produced page errors", ERRORS[page] == [], ERRORS[page])
    await page.context.close()


async def prompt_about_a_different_helper(browser) -> None:
    """An install of one helper says nothing about a different one, and must not silence it."""
    page = await fresh(browser, query="?missing=libreoffice,magick&installms=1500")
    await open_settings(page)
    # `?missing=` names the *binary* that is absent; the row that installs it is the package's, so
    # the click is on `imagemagick` and not on `magick`.
    await page.click(f"{tool('imagemagick')} .toolbutton")
    await page.wait_for_selector(".install__log")
    await page.keyboard.press("Escape")
    await page.wait_for_timeout(300)
    await page.evaluate(DROP, ["notes.md"])
    await page.wait_for_selector(".row")
    await convert_and_settle(page)
    check(
        "183 a batch blocked by LibreOffice still asks about it while ImageMagick is installing",
        "LibreOffice" in (await page.text_content(".prompt__body") or ""),
        await page.text_content(".prompt__body") if await page.query_selector(".prompt") else None,
    )
    check(
        "184 the precondition: that other install really was still in flight",
        await page.evaluate("() => document.querySelector('.install')?.dataset.status") == "running",
        await page.evaluate("() => document.querySelector('.install')?.dataset.status"),
    )
    check("185 and it produced no page errors", ERRORS[page] == [], ERRORS[page])
    await page.context.close()


async def inherited_install(browser) -> None:
    """The other half of `get_activity`: an install in flight that it cannot name.

    `Activity` is two booleans, so a window that reloaded mid-install knows only *that* a helper is
    being installed. It has to hold every Install button until the first `install://event` says
    which one (`State.foreignInstall`), then adopt that install as if it had started it. Both
    directions are expensive to get wrong: a second install the backend would refuse, or an
    adoption left standing over every helper for the rest of the session.
    """
    # Slow enough that the first event is still to come while the sheet is being read: this is the
    # unnamed state, and it only exists between `get_activity` and that event.
    page = await fresh(
        browser, query="?activity=installing&missing=pandoc,libreoffice&installms=1200"
    )
    await open_settings(page)
    check(
        "186 an install the shell already had running holds every button before it can be named",
        await page.query_selector(".install") is None
        and await page.eval_on_selector_all(".toolbutton", "els => els.length") == 2
        and await page.eval_on_selector_all(".toolbutton", "els => els.every((b) => b.disabled)")
        and await page.eval_on_selector_all(".tool__waiting", "els => els.length") == 2,
        [
            await page.eval_on_selector_all(".toolbutton", "els => els.map((b) => b.disabled)"),
            await page.eval_on_selector_all(".tool__waiting", "els => els.length"),
        ],
    )
    unnamed_errors = ERRORS[page]
    await page.context.close()

    page = await fresh(
        browser, query="?activity=installing&missing=pandoc,libreoffice&installms=300"
    )
    await open_settings(page)
    # The event itself is what is being waited for, not a duration: it is the only thing that can
    # turn the unnamed install into this helper's own log.
    await page.wait_for_selector(f"{tool('pandoc')} .install__log", timeout=30000)
    check(
        "187 its first event names it, and it is adopted as the install this window is showing",
        await page.eval_on_selector_all(".install", "els => els.length") == 1
        and await page.evaluate("() => document.querySelector('.install').dataset.status")
        == "running"
        and await page.evaluate(
            "() => { const b = document.querySelector('.tool[data-tool=\"libreoffice\"] .toolbutton');"
            " return b !== null && b.disabled; }"
        ),
        [
            await page.evaluate("() => document.querySelector('.install').dataset.status"),
            await page.eval_on_selector_all(".install", "els => els.length"),
        ],
    )
    await page.wait_for_selector('.install[data-status="ok"]', timeout=30000)
    await page.wait_for_timeout(600)
    check(
        "188 and when it ends the adoption ends with it: the other helpers are the user's again",
        await page.query_selector(f"{tool('pandoc')} .toolbutton") is None
        and await page.evaluate(
            "() => { const b = document.querySelector('.tool[data-tool=\"libreoffice\"] .toolbutton');"
            " return b !== null && !b.disabled; }"
        )
        and await page.eval_on_selector_all(".tool__waiting", "els => els.length") == 0,
        [
            await page.eval_on_selector_all(".toolbutton", "els => els.map((b) => b.disabled)"),
            await page.eval_on_selector_all(".tool__waiting", "els => els.length"),
        ],
    )
    check(
        "189 neither the unnamed install nor its adoption produced page errors",
        unnamed_errors == [] and ERRORS[page] == [],
        [unnamed_errors, ERRORS[page]],
    )
    await page.context.close()


# ================================================================= a drop the cap cut short ====
#
# `inspect_files` enumerates at most `Inspection.limit` files (5000) and answers with the flag that
# says the walk stopped early. A queue that silently stopped at the cap is indistinguishable from a
# folder that held exactly that many files, so the number is said out loud — and said quietly, in
# the toast slot with the danger hairline taken off it, because nothing has gone wrong. `?maxfiles=`
# lowers the cap so this is reachable without dropping five thousand and one files.

QUIET_NOTICE = """
() => {
  const toast = document.querySelector('.toast');
  if (toast === null) return null;
  const cs = getComputedStyle(toast);
  return {
    text: toast.querySelector('.toast__text')?.textContent ?? null,
    role: toast.getAttribute('role'),
    quiet: toast.classList.contains('toast--quiet'),
    // The error toast wears a danger hairline down its left edge; a statement of fact must not.
    accented: cs.borderLeftColor !== cs.borderTopColor,
  };
}
"""

PILL = "() => document.querySelector('.pill')?.textContent ?? null"


async def capped_drop(browser) -> None:
    """The cap is only honest if it is mentioned, and only kind if it does not interrupt."""
    page = await fresh(browser)
    await page.evaluate(DROP, ["one.mov", "two.mov", "three.m4a"])
    await page.wait_for_function("() => document.querySelectorAll('.row').length === 3")
    await page.wait_for_timeout(400)
    check(
        "190 a drop that fits under the cap is untouched, and nothing is said about it",
        await page.evaluate(QUIET_NOTICE) is None
        and await page.evaluate(PILL) == "Convert 3 files",
        [await page.evaluate(QUIET_NOTICE), await page.evaluate(PILL)],
    )
    await page.context.close()

    page = await fresh(browser, query="?maxfiles=3")
    await page.evaluate(DROP, ["one.mov", "two.mov", "three.m4a", "four.mov", "five.HEIC"])
    # The notice and the short queue are the two observable halves of the same answer.
    await page.wait_for_selector(".toast--quiet")
    await page.wait_for_function("() => document.querySelectorAll('.row').length === 3")
    notice = await page.evaluate(QUIET_NOTICE)
    check(
        "191 a drop the cap cut short says so, and names the number it stopped at",
        notice is not None
        and notice["text"] == "Only the first 3 files were added. Convert these, then drop the rest.",
        notice,
    )
    check(
        "192 and says it quietly: a status, in the toast slot, with the danger hairline taken off",
        notice is not None
        and notice["role"] == "status"
        and notice["quiet"]
        and not notice["accented"],
        notice,
    )

    # Nothing is blocked by it: the files that did land are an ordinary queue, and the primary
    # action is reachable with the notice still standing.
    check(
        "193 the files that did land are convertible at once, notice and all",
        await page.evaluate(PILL) == "Convert 3 files"
        and await page.evaluate("() => document.querySelector('.pill').disabled") is False,
        [await page.evaluate(PILL),
         await page.evaluate("() => document.querySelector('.pill').disabled")],
    )
    await convert_and_settle(page)
    check(
        "194 and they really do convert, with the notice never having got in the way",
        await page.evaluate(STATUSES) == ["done", "done", "done"]
        and (await page.evaluate(QUIET_NOTICE) or {}).get("quiet") is True,
        [await page.evaluate(STATUSES), await page.evaluate(QUIET_NOTICE)],
    )

    # Each inspection answers afresh: a sentence about files that are all in the queue would send
    # the user looking through their folder for nothing. The rows and the notice are written by one
    # store update, so a queue of five is proof the notice has been re-answered too.
    await page.evaluate(DROP, ["six.mov", "seven.mov"])
    await page.wait_for_function("() => document.querySelectorAll('.row').length === 5")
    check(
        "195 an ordinary drop afterwards takes the notice away with it",
        await page.evaluate(QUIET_NOTICE) is None,
        await page.evaluate(QUIET_NOTICE),
    )
    check("196 none of that produced page errors", ERRORS[page] == [], ERRORS[page])
    await page.context.close()



PACKAGE_LIST = """
() => [...document.querySelectorAll('.tool')].map((t) => ({
  id: t.dataset.tool,
  label: t.querySelector('.tool__label').textContent,
  installable: t.dataset.installable ?? null,
  command: t.querySelector('.tool__hint')?.textContent ?? null,
}))
"""

POPPLER_COPY = """
() => { const t = document.querySelector('.tool[data-tool="poppler"]');
 return t === null ? null : { label: t.querySelector('.tool__label').textContent,
 state: t.querySelector('.tool__state').textContent,
 partial: t.querySelector('.tool__partial')?.textContent ?? null,
 button: t.querySelector('.toolbutton')?.getAttribute('aria-label') ?? null }; }
"""


async def one_row_per_package(browser) -> None:
    """One formula, one row, one name — and a half-landed one that says so in words."""
    page = await fresh(browser, query="?missing=")
    await open_settings(page)
    rows = await page.evaluate(PACKAGE_LIST)
    poppler = [r for r in rows if "Poppler" in (r["label"] or "")]
    check(
        "197 Poppler is one row named once, not one row per binary it ships",
        len(poppler) == 1
        and poppler[0]["id"] == "poppler"
        and poppler[0]["label"] == "Poppler"
        and poppler[0]["installable"] is not None
        # And no row anywhere is a binary of it, nor repeats its one command.
        and not any("pdfto" in (r["label"] or "") for r in rows)
        and len([r for r in rows if r["command"] == "brew install poppler"]) <= 1,
        [r["label"] for r in rows],
    )
    await page.context.close()

    # Two of the three binaries absent: the row is offered, and it explains itself.
    page = await fresh(browser, query="?missing=pdftotext,pdftohtml")
    await open_settings(page)
    copy = await page.evaluate(POPPLER_COPY)
    check(
        "198 a half-landed package says how much of it is here, and names only itself",
        copy is not None
        and copy["state"] == "Incomplete"
        and copy["partial"]
        == "1 of Poppler’s 3 programs is here. Installing it again should bring the rest."
        and copy["button"] == "Install Poppler",
        copy,
    )
    check("199 the package list produced no page errors", ERRORS[page] == [], ERRORS[page])
    await page.context.close()



ROW_NAMES = "() => [...document.querySelectorAll('.row__name')].map((n) => n.textContent)"

ROW_LABELS = "() => [...document.querySelectorAll('.row')].map((r) => r.getAttribute('aria-label'))"

SELECTED_ROW = """
() => { const row = document.querySelector('.row[data-selected]');
 return row === null ? null : row.querySelector('.row__name').textContent; }
"""

# Which row the keyboard is standing on, or what it is standing on instead.
FOCUSED_ROW = """
() => { const node = document.activeElement;
 if (node === null || node === document.body) return 'body';
 const row = node.closest('.row');
 return row === node ? row.querySelector('.row__name').textContent : (node.className || node.tagName); }
"""


async def rows_settle(page: Page, count: int, timeout: float = 4000) -> bool:
    """Did the queue become `count` rows?

    Waits on the rows themselves rather than a sleep, and answers False instead of raising: a
    keystroke that is *meant* to be a no-op is one of the things being asserted, and a timeout that
    dumped a stack would take the rest of the section with it.
    """
    try:
        await page.wait_for_function(
            "(n) => document.querySelectorAll('.row').length === n", arg=count, timeout=timeout
        )
        return True
    except PWTimeout:
        return False


async def keyboard_pruning(browser) -> None:
    """Deleting rows with the keyboard, and the ring that has to survive it."""
    page = await fresh(browser)
    await page.evaluate(DROP, ["one.mov", "two.mov", "three.mov", "four.m4a"])
    await page.wait_for_function("() => document.querySelectorAll('.row').length === 4")
    check(
        "200 a row the keyboard can stop on says which file it is",
        await page.evaluate(ROW_LABELS) == ["one.mov", "two.mov", "three.mov", "four.m4a"],
        await page.evaluate(ROW_LABELS),
    )

    await page.click(".row")
    await page.keyboard.press("Backspace")
    gone = await rows_settle(page, 3)
    check(
        "201 removing the selected row moves the selection to the file that took its place",
        gone and await page.evaluate(SELECTED_ROW) == "two.mov",
        [await page.evaluate(SELECTED_ROW), await page.evaluate(ROW_NAMES)],
    )
    check(
        "202 and the focus ring goes with it, rather than falling back to the window",
        await page.evaluate(FOCUSED_ROW) == "two.mov",
        await page.evaluate(FOCUSED_ROW),
    )

    # The point of both: ⌫ held down prunes a queue, instead of deleting one file and going quiet.
    await page.keyboard.press("Backspace")
    walked = await rows_settle(page, 2)
    await page.keyboard.press("Backspace")
    check(
        "203 so ⌫ walks the queue instead of stopping after the first file",
        walked and await rows_settle(page, 1) and await page.evaluate(ROW_NAMES) == ["four.m4a"],
        await page.evaluate(ROW_NAMES),
    )

    # The same question at the end of the list, where there is no row after the one being removed.
    await page.evaluate(DROP, ["five.mov", "six.mov"])
    await page.wait_for_function("() => document.querySelectorAll('.row').length === 3")
    await page.click(".row:last-child")
    await page.keyboard.press("Backspace")
    settled = await rows_settle(page, 2)
    check(
        "204 removing the last row hands the selection back up the list",
        settled
        and await page.evaluate(SELECTED_ROW) == "five.mov"
        and await page.evaluate(FOCUSED_ROW) == "five.mov",
        [await page.evaluate(SELECTED_ROW), await page.evaluate(FOCUSED_ROW)],
    )

    # And the other side of it: a removal the keyboard was not part of must not summon it. `.click()`
    # on the × does not move focus, so Clear all still has it while the row disappears.
    await page.focus(".filelist__header .microlink")
    await page.evaluate("() => document.querySelector('.row .row__remove').click()")
    left = await rows_settle(page, 1)
    check(
        "205 a removal the keyboard was not part of leaves it where it was",
        left and await page.evaluate("() => document.activeElement?.textContent") == "Clear all",
        await page.evaluate("() => document.activeElement?.textContent"),
    )
    check("206 pruning by keyboard produced no page errors", ERRORS[page] == [], ERRORS[page])
    await page.context.close()



PROGRESS_TALK = """
async () => {
  // Everything below happens at microtask checkpoints, where no timer can run: the mock's own
  // progress ticks cannot overtake the reads, so the two halves really are the same instant.
  const drain = async () => { for (let i = 0; i < 8; i += 1) await Promise.resolve(); };
  const runningIds = () => [...document.querySelectorAll('.row')]
    .map((row, i) => (row.dataset.status === 'running' ? `mock-${i + 1}` : null))
    .filter((id) => id !== null);
  const read = () => {
    const el = document.querySelector('.actionbar__text');
    return {
      all: el.textContent,
      spoken: [...el.childNodes]
        .filter((n) => !(n.nodeType === 1 && n.getAttribute('aria-hidden') === 'true'))
        .map((n) => n.textContent)
        .join(''),
      hidden: [...el.querySelectorAll('[aria-hidden="true"]')].map((n) => n.textContent).join(''),
      rows: [...document.querySelectorAll('.row[data-status="running"] .row__status')]
        .map((n) => n.textContent),
    };
  };
  const emit = (eta) => {
    for (const id of runningIds()) {
      window.__ceMockEmit({ type: 'progress', id, fraction: 0.5, speed: 1.4, eta_secs: eta });
    }
  };
  emit(300); await drain(); const far = read();
  emit(30); await drain(); const near = read();
  emit(0); await drain(); const zero = read();
  // The same reading a hair above zero: `formatEta` rounds, so 0.4s used to print "0s left" too.
  emit(0.4); await drain(); const nearly = read();
  return { far, near, zero, nearly,
    live: document.querySelector('.actionbar__note').getAttribute('aria-live') };
}
"""


async def spoken_progress(browser) -> None:
    """The countdown is for the eye; the tally is what gets said."""
    page = await fresh(browser)
    await page.evaluate(DROP, ["one.mov", "two.mov", "three.mov", "four.mov"])
    await page.wait_for_function("() => document.querySelectorAll('.row').length === 4")
    await page.click(".pill")
    await page.wait_for_function("() => document.querySelector('.pill').textContent === 'Stop'")
    await page.wait_for_selector('.row[data-status="running"] .row__status')
    talk = await page.evaluate(PROGRESS_TALK)
    far, near, zero, nearly = talk["far"], talk["near"], talk["zero"], talk["nearly"]

    check(
        "207 the countdown is on the screen but outside the sentence the live region announces",
        talk["live"] == "polite"
        and far["hidden"] == " · 5m 00s left"
        and re.fullmatch(r"\d+ of 4 done", far["spoken"]) is not None
        and far["all"] == far["spoken"] + far["hidden"],
        far,
    )
    check(
        "208 an ETA that keeps moving does not re-announce the tally with it",
        near["spoken"] == far["spoken"]
        and near["hidden"] == " · 30s left"
        and len(near["rows"]) > 0
        and all(re.fullmatch(r"\d+% · 30s left", text) for text in near["rows"]),
        [far["spoken"], near["spoken"], near["hidden"], near["rows"]],
    )
    check(
        "209 and an ETA that would read as zero prints no countdown at all, in the bar or on the row",
        zero["hidden"] == ""
        and nearly["hidden"] == ""
        and len(zero["rows"]) > 0
        and all(re.fullmatch(r"\d+%", text) for text in zero["rows"] + nearly["rows"]),
        [zero["hidden"], zero["rows"], nearly["hidden"], nearly["rows"]],
    )

    await page.click(".pill")
    await page.wait_for_function(
        "() => document.querySelector('.pill').textContent.startsWith('Convert')", timeout=30000
    )
    check("210 none of that produced page errors", ERRORS[page] == [], ERRORS[page])
    await page.context.close()


# ==================================================== the installer log: read, not recited ========
#
# `role="log"` carries an implicit `aria-live="polite"`, so the pane was announcing every line
# `brew` wrote — over the top of `.install__state`, the one sentence that is meant to be heard (96).
# It is a log: something to scroll back through, not a commentary. And it is bounded, because an
# install that writes for ten minutes must not grow the DOM for ten minutes.

INSTALL_FLOOD = """
async () => {
  for (let i = 0; i < 1200; i += 1) {
    window.__ceMockInstallEmit({ type: 'log', package_id: 'pandoc', line: `line ${i}` });
  }
  for (let i = 0; i < 8; i += 1) await Promise.resolve();
  const el = document.querySelector('.install__log');
  const lines = el.textContent.split('\\n');
  return { count: lines.length, first: lines[0], last: lines[lines.length - 1] };
}
"""


async def installer_log_is_quiet(browser) -> None:
    """A log a screen reader can visit, not one it reads aloud — and one that stays a fixed size."""
    page = await fresh(browser, query="?missing=pandoc&installms=1500")
    await open_settings(page)
    await page.click(f"{tool('pandoc')} .toolbutton")
    await page.wait_for_selector(".install__log")
    check(
        "211 the log is something to read back, not a voice over the status line",
        await page.get_attribute(".install__log", "role") == "log"
        and await page.get_attribute(".install__log", "aria-live") == "off",
        [await page.get_attribute(".install__log", "role"),
         await page.get_attribute(".install__log", "aria-live")],
    )
    flood = await page.evaluate(INSTALL_FLOOD)
    check(
        "212 and a thousand lines of output do not pile up in it",
        flood["count"] == 400 and flood["first"] == "line 800" and flood["last"] == "line 1199",
        flood,
    )
    check("213 flooding the log produced no page errors", ERRORS[page] == [], ERRORS[page])
    await page.context.close()


# ============================================ a placeholder nobody can see explains nothing =======
#
# `NumberField` shows its placeholder while the box is empty, which is a real state for the settings
# backed by `Option<u32>` ("Original", "Automatic"). The rest are plain numbers: clear the box and
# the fallback is written straight back into it, so the box is never empty and the placeholder is
# never seen. Five of them were written anyway, and one was carrying the only explanation of what
# `0` means in "GIF loops" — a field that shows `0` and, read plainly, says "do not loop".

NUMBER_FIELDS = """
() => [...document.querySelectorAll('.field')].map((field) => {
  const input = field.querySelector('input[type="number"]');
  if (input === null) return null;
  return {
    label: field.querySelector('.field__label').firstChild?.textContent ?? '',
    hint: field.querySelector('.field__hint')?.textContent ?? null,
    value: input.value,
    placeholder: input.placeholder,
  };
}).filter((field) => field !== null)
"""

# Empty every number box and see which ones stay empty. A field backed by `Option<u32>` does — that
# is the "keep the source value" choice, and the placeholder is the word for it. A field backed by a
# plain number writes its fallback straight back, so a placeholder on it can never be read.
EMPTY_EVERY_NUMBER = """
async () => {
  const setValue = (input, text) => {
    const descriptor = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(input), 'value');
    descriptor.set.call(input, text);
    input.dispatchEvent(new Event('input', { bubbles: true }));
  };
  const out = [];
  for (const field of document.querySelectorAll('.field')) {
    const input = field.querySelector('input[type="number"]');
    if (input === null) continue;
    const label = field.querySelector('.field__label').firstChild?.textContent ?? '';
    setValue(input, '');
    for (let i = 0; i < 8; i += 1) await Promise.resolve();
    out.push({ label, empty: input.value === '', placeholder: input.placeholder });
  }
  return out;
}
"""


async def settings_number_copy(browser) -> None:
    """Every word in the settings sheet is somewhere it can actually be read."""
    page = await fresh(browser)
    await open_settings(page)
    # Every section open: a field inside a closed `<details>` is in the DOM but not on the screen.
    while await page.query_selector(".section:not([open]) > .section__title") is not None:
        await page.click(".section:not([open]) > .section__title")
    fields = await page.evaluate(NUMBER_FIELDS)
    loops = next((f for f in fields if f["label"] == "GIF loops"), None)
    check(
        "214 GIF loops explains what 0 means, beside the label rather than behind the value",
        loops is not None and loops["value"] == "0" and loops["hint"] == "0 loops forever",
        loops,
    )

    emptied = await page.evaluate(EMPTY_EVERY_NUMBER)
    unreadable = [f["label"] for f in emptied if not f["empty"] and f["placeholder"] != ""]
    unsaid = [f["label"] for f in emptied if f["empty"] and f["placeholder"] == ""]
    check(
        "215 a placeholder appears on every field that can be empty, and on no field that cannot",
        len(emptied) >= 10 and unreadable == [] and unsaid == [],
        {"never seen": unreadable, "never said": unsaid, "fields": len(emptied)},
    )
    check("216 clearing every number field produced no page errors", ERRORS[page] == [], ERRORS[page])
    await page.context.close()


# ================================================= the canvas the queue leaves behind =============
#
# The last row removed, or ⇧⌘⌫ on the lot, unmounts the very control the keyboard was standing on.
# The empty canvas is the one thing left, and it is the control that undoes what just happened — so
# it takes the focus rather than letting it fall to `<body>`, where the next Tab starts again at the
# top of the window. A *first* launch is the other empty canvas, and must stay untouched.

FOCUS_IS_CANVAS = "() => document.activeElement === document.querySelector('.dropzone')"


async def emptying_the_queue(browser) -> None:
    """Where the keyboard stands once there is nothing left in the list."""
    page = await fresh(browser)
    check(
        "217 a first launch leaves the canvas alone: nothing takes focus on its own",
        await page.evaluate("() => document.activeElement === document.body")
        and not await page.evaluate(FOCUS_IS_CANVAS),
        await page.evaluate("() => document.activeElement?.className ?? 'body'"),
    )

    await page.evaluate(DROP, ["only.mov"])
    await page.wait_for_function("() => document.querySelectorAll('.row').length === 1")
    await page.click(".row")
    await page.keyboard.press("Backspace")
    await page.wait_for_selector(".dropzone")
    check(
        "218 removing the last row hands the keyboard to the canvas rather than dropping it",
        await page.evaluate(FOCUS_IS_CANVAS),
        await page.evaluate("() => document.activeElement?.className ?? 'body'"),
    )

    # And the same for the whole queue at once. ⇧⌘⌫ is a menu accelerator under Tauri; in a browser
    # the store's own binding answers it.
    await page.evaluate(DROP, ["one.mov", "two.mov", "three.m4a"])
    await page.wait_for_function("() => document.querySelectorAll('.row').length === 3")
    await page.click(".row")
    await page.keyboard.press("Control+Shift+Backspace")
    await page.wait_for_selector(".dropzone")
    check(
        "219 and so does clearing the lot",
        await page.evaluate(FOCUS_IS_CANVAS)
        and await page.evaluate("() => document.querySelectorAll('.row').length") == 0,
        await page.evaluate("() => document.activeElement?.className ?? 'body'"),
    )
    check("220 emptying the queue produced no page errors", ERRORS[page] == [], ERRORS[page])
    await page.context.close()


# ================================================== the bulk picker, and the words beside it ======
#
# The header's per-category picker is the one control whose label a stranger only ever *hears*. It
# used to name the catalog's category ("Convert all Video files to") while the text beside it said
# "All 3 videos" — two vocabularies for one control, and the accessible name was the odd one out.

BULK_LABELS = """
() => [...document.querySelectorAll('.filelist__bulkitem')].map((item) => ({
  text: item.querySelector('.filelist__bulklabel').textContent,
  label: item.querySelector('.target__select').getAttribute('aria-label'),
}))
"""


async def bulk_picker_label(browser) -> None:
    """The name the picker is announced by says what the line beside it says."""
    page = await fresh(browser)
    await page.evaluate(DROP, ["one.mov", "two.mov", "three.mov", "song.m4a"])
    await page.wait_for_function("() => document.querySelectorAll('.row').length === 4")
    await page.wait_for_selector(".filelist__bulkitem")
    labels = await page.evaluate(BULK_LABELS)
    check(
        "221 the bulk picker is announced in the same words as the line beside it",
        labels == [{"text": "All 3 videos", "label": "Convert all 3 videos to"}],
        labels,
    )
    check("222 the bulk picker produced no page errors", ERRORS[page] == [], ERRORS[page])
    await page.context.close()



ROW_PICKERS = ".row .target__select"


def row_picker(index: int) -> str:
    """The nth row's format picker, as one plain CSS selector both sides of the bridge can read."""
    return f".filelist__rows > .row:nth-child({index + 1}) .target__select"

PICKERS = """
() => ({
  rows: document.querySelectorAll('.row').length,
  selects: document.querySelectorAll('.target__select').length,
  options: document.querySelectorAll('.target__select option').length,
  groups: document.querySelectorAll('.target__select optgroup').length,
})
"""

# Everything the closed control puts on the screen, plus the name it is announced by. The width is
# the interesting one: the wrapper is sized by a hidden sizer holding the *selected* label, so a
# select that has lost its longest option must still measure exactly what it measured before.
PICKER_BOX = """
(index) => {
  const select = document.querySelectorAll('.row .target__select')[index];
  const sizer = select.parentElement.querySelector('.target__sizer');
  const r = select.getBoundingClientRect();
  return {
    label: sizer.textContent,
    value: select.value,
    name: select.getAttribute('aria-label'),
    w: Math.round(r.width * 100) / 100,
    h: Math.round(r.height * 100) / 100,
    options: select.options.length,
  };
}
"""

OPTION_VALUES = """
(index) => [...document.querySelectorAll('.row .target__select')[index].options].map((o) => o.value)
"""

OTHER_PICKERS = """
(index) => [...document.querySelectorAll('.row .target__select')]
  .filter((_, i) => i !== index)
  .map((s) => s.options.length)
"""


async def prime_picker(page: Page, selector: str = ROW_PICKERS) -> None:
    """Ask a picker for its list the way a person does, and wait until it has one.

    The list is built when the control is used (223 onwards). Playwright's `select_option` reads the
    `options` collection without focusing or clicking anything, so every check that reads a row's
    option list has to open the picker first — one focus, then a wait on the observable state.
    """
    await page.focus(selector)
    await page.wait_for_function("(s) => document.querySelector(s).options.length > 1", arg=selector)


async def pickers_on_demand(browser) -> None:
    """A row's option list is built when its picker is used, not when the row appears."""
    page = await fresh(browser)
    kinds = ("mov", "m4a", "png", "docx", "srt")
    names = [f"take-{i:03d}.{kinds[i % len(kinds)]}" for i in range(120)]
    await page.evaluate(DROP, names)
    await page.wait_for_function("() => document.querySelectorAll('.row').length === 120")
    await page.wait_for_selector(".filelist__bulkitem")

    counts = await page.evaluate(PICKERS)
    check(
        "223 a queue of 120 rows holds one option per picker, not a whole list per row",
        counts["rows"] == 120
        and counts["selects"] > 120
        and counts["options"] == counts["selects"]
        and counts["groups"] == 0,
        counts,
    )

    closed = await page.evaluate(PICKER_BOX, 3)
    await prime_picker(page, row_picker(3))
    opened = await page.evaluate(PICKER_BOX, 3)
    check(
        "224 opening one builds its list without moving or renaming the control",
        opened["options"] > 10
        and opened["label"] == closed["label"]
        and opened["value"] == closed["value"]
        and opened["name"] == closed["name"]
        and near(opened["w"], closed["w"], 0.01)
        and near(opened["h"], closed["h"], 0.01),
        {"closed": closed, "opened": opened},
    )

    grown = await page.evaluate(PICKERS)
    check(
        "225 and it builds that row's list alone: every other picker still holds its one option",
        set(await page.evaluate(OTHER_PICKERS, 3)) == {1}
        and grown["options"] == grown["selects"] - 1 + opened["options"]
        and grown["groups"] == 2,
        {"before": counts, "after": grown, "opened": opened["options"]},
    )

    # Rows 3 and 8 are both `.docx` (the names cycle through five kinds), so the two lists are the
    # same list — and the only difference between them is which gesture asked for it.
    by_key = await page.evaluate(OPTION_VALUES, 3)
    await page.click(row_picker(8))
    await page.wait_for_function(
        "() => document.querySelectorAll('.row .target__select')[8].options.length > 1"
    )
    by_mouse = await page.evaluate(OPTION_VALUES, 8)
    await page.keyboard.press("Escape")
    check(
        "226 a mouse gets the list a keyboard gets: same options, same order, both groups",
        by_mouse == by_key and len(by_key) > 10,
        {"keyboard": len(by_key), "mouse": len(by_mouse), "first": by_key[:3]},
    )

    # The whole point of building on focus rather than on click: a keystroke arriving at a picker
    # nobody has touched must land on the real list, in the same tick.
    was = await page.evaluate(PICKER_BOX, 7)
    await page.focus(row_picker(7))
    # Type-ahead selects PNG on macOS; headless Chromium cannot drive its native arrow popup.
    await page.keyboard.press("p" if sys.platform == "darwin" else "ArrowDown")
    await page.wait_for_function(
        "(w) => document.querySelectorAll('.row .target__select')[7].value !== w", arg=was["value"]
    )
    now = await page.evaluate(PICKER_BOX, 7)
    check(
        "227 and the keyboard alone still re-targets a row, list unbuilt when the key arrived",
        now["value"] != was["value"] and now["label"] != was["label"] and now["label"] != "",
        {"was": was["value"], "now": now["value"], "label": now["label"]},
    )
    check("228 a 120-row queue of lazy pickers produced no page errors", ERRORS[page] == [], ERRORS[page])
    await page.context.close()



CONTROL_TEXT = (
    ".filelist__bulklabel",
    ".filelist__header .microlink",
    ".row .target__select",
    ".row__remove",
    ".row__output",
    ".row__fix",
    ".row .microlink",
    ".actionbar__line .microlink",
)

#: Words that are not a control, and are not decoration either: they have to be read, not operated.
SECONDARY_TEXT = (".row__meta", ".actionbar__text", ".actionbar__preset")

#: Controls inside the settings sheet, measured with the sheet open and over its own surface.
DRAWER_TEXT = (".section__title", ".drawer__header .iconbutton", ".drawer__footer .microlink")

# The colour of the text, plus every background between it and the top of the document.
BACKDROP = """
(selectors) => selectors.map((sel) => {
  const el = document.querySelector(sel);
  if (el === null) return [sel, null];
  const stack = [];
  for (let n = el; n !== null; n = n.parentElement) stack.push(getComputedStyle(n).backgroundColor);
  return [sel, { color: getComputedStyle(el).color, stack }];
})
"""

# The ladder itself, read off `:root` rather than off any one element that happens to use it. The
# values come back through a throwaway probe rather than as the raw declarations: a token is written
# as a hex triplet in one scheme and as `rgba()` in another, and the browser is the only thing that
# should be in the business of telling those apart.
LADDER = """
() => {
  const root = getComputedStyle(document.documentElement);
  const names = ['--ink', '--ink-1', '--ink-control', '--ink-2', '--ink-quiet', '--ink-3', '--ink-4'];
  const probe = document.createElement('span');
  probe.setAttribute('aria-hidden', 'true');
  document.body.appendChild(probe);
  const rungs = names.map((n) => {
    probe.style.color = `var(${n})`;
    return [n, getComputedStyle(probe).color];
  });
  probe.remove();
  return {
    rungs,
    declared: names.every((n) => root.getPropertyValue(n).trim() !== ''),
    page: getComputedStyle(document.body).backgroundColor,
  };
}
"""


async def ratios(page: Page, selectors: tuple[str, ...]) -> dict[str, float | None]:
    """Every selector's real contrast ratio, or `None` where the element was not on screen."""
    out: dict[str, float | None] = {}
    for sel, found in await page.evaluate(BACKDROP, list(selectors)):
        out[sel] = None if found is None else contrast(found["color"], found["stack"])
    return out


async def settled_queue(browser, scheme: str) -> Page:
    """One window holding every state the contrast checks need at once.

    Three videos (so the header grows its bulk picker) and a document that cannot convert while
    LibreOffice is missing, run to the end: the finished rows carry an output name and Reveal, the
    failed one carries "Install LibreOffice" and Retry, and the bar carries its tally, its caption
    and Open folder. The card the settled batch raises is answered first, because it covers the
    window it is asking about.
    """
    page = await fresh(browser, scheme=scheme, query="?missing=libreoffice")
    await page.evaluate(DROP, ["one.mov", "two.mov", "three.mov", "report.docx"])
    await page.wait_for_function("() => document.querySelectorAll('.row').length === 4")
    await page.click(".pill")
    await page.wait_for_function(
        "() => document.querySelector('.pill').textContent.startsWith('Convert')", timeout=60000
    )
    if await page.query_selector(".prompt") is not None:
        await page.keyboard.press("Escape")
        await page.wait_for_selector(".prompt", state="detached")
    await page.wait_for_selector(".row__output")
    await page.wait_for_selector(".row__fix")
    await unhover(page)
    return page


async def interactive_contrast(browser) -> None:
    """Text a person has to read to operate the app clears AA, in both schemes."""
    measured: dict[str, dict[str, float | None]] = {}
    for scheme in ("light", "dark"):
        page = await settled_queue(browser, scheme)
        found = await ratios(page, CONTROL_TEXT + SECONDARY_TEXT)
        await open_settings(page)
        found.update(await ratios(page, DRAWER_TEXT))
        ladder = await page.evaluate(LADDER)
        measured[scheme] = found
        page_bg = [ladder["page"]]
        n = 229 if scheme == "light" else 233
        paper = f"the default theme ({scheme} OS appearance)"

        controls = {s: r for s, r in found.items() if s in CONTROL_TEXT and r is not None}
        drawer = {s: r for s, r in found.items() if s in DRAWER_TEXT and r is not None}
        check(
            f"{n} every control's own text clears 4.5:1 on {paper}",
            len(controls) >= 7
            and len(drawer) == len(DRAWER_TEXT)
            and all(r >= 4.5 for r in {**controls, **drawer}.values()),
            {**controls, **drawer},
        )

        secondary = {s: r for s, r in found.items() if s in SECONDARY_TEXT and r is not None}
        check(
            f"{n + 1} and the words that are only read, never operated, clear 3:1 on {paper}",
            len(secondary) == len(SECONDARY_TEXT) and all(r >= 3.0 for r in secondary.values()),
            secondary,
        )

        rungs = [(name, contrast(value, page_bg)) for name, value in ladder["rungs"]]
        check(
            f"{n + 2} the ladder is still a ladder on {paper}: seven rungs, each quieter than the last",
            ladder["declared"] and all(a[1] > b[1] for a, b in zip(rungs, rungs[1:])),
            rungs,
        )
        check(f"{n + 3} measuring the contrast on {paper} produced no page errors",
              ERRORS[page] == [], ERRORS[page])
        await page.context.close()

    # The tokens are per-scheme *because* the same alpha does not read the same on paper as on
    # near-black, so neither scheme is allowed to be the one that was actually looked at.
    both = {
        sel: (measured["light"][sel], measured["dark"][sel])
        for sel in CONTROL_TEXT + SECONDARY_TEXT + DRAWER_TEXT
        if measured["light"][sel] is not None
    }
    check(
        "237 and no selector passes in one scheme by failing in the other",
        len(both) >= 12
        and all(
            min(pair) >= (4.5 if sel not in SECONDARY_TEXT else 3.0) for sel, pair in both.items()
        ),
        both,
    )



NOTE = """
() => {
  const el = document.querySelector('.enginenote');
  if (el === null) return null;
  const s = getComputedStyle(el);
  const r = el.getBoundingClientRect();
  const main = document.querySelector('.main').getBoundingClientRect();
  return {
    text: el.textContent,
    role: el.getAttribute('role'),
    position: s.position,
    buttons: el.querySelectorAll('button, a, input, select, [tabindex]').length,
    weight: s.fontWeight,
    size: s.fontSize,
    align: s.textAlign,
    background: s.backgroundColor,
    border: s.borderTopWidth + ' ' + s.borderTopStyle,
    below: Math.round(r.top - document.querySelector('.dropzone__frame').getBoundingClientRect().bottom),
    inWindow: r.bottom <= main.bottom + 1 && r.top >= main.top,
    centred: Math.abs((r.left + r.right) / 2 - (main.left + main.right) / 2) < 2,
  };
}
"""


async def appears(page: Page, selector: str) -> dict | None:
    """`NOTE` for a selector once it is on screen, or `None` if it never arrives.

    A plain `wait_for_selector` is the right wait for an element that is supposed to be there, but
    these assertions have to be run against a build without the fix as well, and the absence has to
    read as a failed check rather than as a 30s timeout traceback. The wait is still on the element
    itself — nothing here sleeps — it simply gives up in five seconds and lets `check` do the talking.
    """
    try:
        await page.wait_for_selector(selector, timeout=5000)
    except PWTimeout:
        return None
    return await page.evaluate(NOTE)


async def engine_missing_is_said_on_the_canvas(browser) -> None:
    """A first-run window with no engine says so, once, where a first-run user is looking."""
    # The mat's geometry, measured on a healthy install, is the thing the note may not disturb.
    healthy = await fresh(browser, query="?missing=")
    await unhover(healthy)
    check(
        "238 a working install says nothing at all on the canvas",
        await healthy.query_selector(".enginenote") is None
        and await healthy.evaluate("() => document.querySelectorAll('.dropzone *').length") > 0,
        await healthy.evaluate(
            "() => document.querySelector('.main').textContent.replace(/\\s+/g, ' ').trim()"
        ),
    )
    was = await box(healthy, ".dropzone__frame")
    await healthy.context.close()

    page = await fresh(browser, query="?missing=ffmpeg,ffprobe")
    await unhover(page)
    note = await appears(page, ".enginenote") or {}
    check(
        "239 a missing engine is said on the empty canvas, in one line, and named",
        "FFmpeg" in note.get("text", "")
        and "convert" in note.get("text", "").lower()
        and note.get("role") == "status",
        note,
    )
    check(
        "240 and said in the canvas's own voice: no box, no button, no alarm",
        note.get("buttons") == 0
        and ink(note.get("background", "")) == 0
        and note.get("border", "").startswith("0px")
        and note.get("weight") in ("400", "normal")
        and float(note.get("size", "99px").replace("px", "")) <= 11.5
        and note.get("align") == "center",
        {k: note.get(k) for k in ("buttons", "background", "border", "weight", "size", "align")},
    )
    now = await box(page, ".dropzone__frame")
    check(
        "241 the mat is exactly where it was: the note is out of flow, under the frame",
        note.get("position") == "absolute"
        and near(now["x"], was["x"], 0.5)
        and near(now["y"], was["y"], 0.5)
        and near(now["w"], was["w"], 0.5)
        and near(now["h"], was["h"], 0.5)
        and note.get("below", -1) >= 0
        and note.get("inWindow") is True
        and note.get("centred") is True,
        {"was": was, "now": now, "note": note},
    )
    # It is a warning, not decoration, so it has to clear the 3:1 the rest of the quiet ink does.
    quiet = await ratios(page, (".enginenote",))
    check(
        "242 and it is legible where it sits, not a watermark",
        (quiet[".enginenote"] or 0) >= 3.0,
        quiet,
    )

    # The queue is not the canvas: a window with files in it has an action bar, rows and their own
    # failures to report a dead engine with, and this line belongs to the state that has none of them.
    await page.evaluate(DROP, ["clip.mov"])
    await page.wait_for_function("() => document.querySelectorAll('.row').length === 1")
    gone = await page.query_selector(".enginenote") is None
    await page.click(".filelist__header .microlink")
    await page.wait_for_selector(".dropzone")
    check(
        "243 the queue never grows one, and emptying the queue brings it back",
        gone and await appears(page, ".enginenote") is not None,
    )

    # Answering it in Settings answers it everywhere: the sheet's banner is the same fact said twice,
    # and a fact dismissed once is dismissed.
    await open_settings(page)
    await page.wait_for_selector(".notice")
    await page.click('.notice .microlink[aria-label="Dismiss warning"]')
    await page.wait_for_selector(".notice", state="detached")
    await page.keyboard.press("Escape")
    await page.wait_for_selector(".drawer:not([data-open])")
    check(
        "244 dismissing it in Settings takes it off the canvas too",
        await page.query_selector(".enginenote") is None,
        await page.evaluate(
            "() => document.querySelector('.main').textContent.replace(/\\s+/g, ' ').trim()"
        ),
    )
    check("245 the canvas warning produced no page errors", ERRORS[page] == [], ERRORS[page])
    await page.context.close()





YOUTUBE = "https://www.youtube.com/watch?v=dQw4w9WgXcQ"
YOUTUBE_SHORT = "https://youtu.be/aqz-KE-bpKQ"
BILIBILI = "https://www.bilibili.com/video/BV1GJ411x7h7"
# A host no substring of "youtube" or "bilibili" will match, which is the point of it below.
BILIBILI_SHORT = "https://b23.tv/av170001"
PLAYLIST = "https://www.youtube.com/playlist?list=PLFgquLnL59akA2PflFpeQG9L01VFg90wS"
CHANNEL = "https://www.youtube.com/@fireship"
NOT_A_VIDEO_PAGE = "https://www.youtube.com/feed/subscriptions"
ELSEWHERE = "https://vimeo.com/76979871"
LOOKALIKE = "https://youtube.com.evil.test/watch?v=abc"

# `convert_core::link::LinkError`, message for message — the sentences a person actually reads. The
# three cap-shaped ones spell "20" out in Rust's own `#[error]` text, which is why they still say 20
# under `?maxlinks=3`: the *copy the UI writes* is what has to follow the backend's number, and the
# copy the backend hands over is already whatever it says.
REFUSED = {
    "playlist": "That is a playlist, not a video. Open the videos you want and paste their "
    "individual links - up to 20 at a time.",
    "channel": "That is a channel, not a video. Open the videos you want and paste their "
    "individual links - up to 20 at a time.",
    "page": "That is not a YouTube video page. Open the video itself and paste the link from the "
    "address bar.",
    "host": "The site `vimeo.com` is not supported. Paste a YouTube, Bilibili, QQ Music, NetEase Music, "
    "SoundCloud or artist.bandcamp.com single-track link. Spotify is not supported.",
    "lookalike": "The site `youtube.com.evil.test` is not supported. Paste a YouTube, Bilibili, QQ Music, NetEase Music, "
    "SoundCloud or artist.bandcamp.com single-track link. Spotify is not supported.",
}

# Everything the card says, in one read: the semantics, every sentence in it, and where the keyboard
# is standing. `boxes` and `veil` are counted because two of either would mean a second modal.
SHEET = """
() => {
  const card = document.querySelector('.links');
  if (card === null) return null;
  const text = (sel) => card.querySelector(sel)?.textContent ?? null;
  const button = card.querySelector('.promptbutton');
  return {
    role: card.getAttribute('role'),
    modal: card.getAttribute('aria-modal'),
    title: document.getElementById(card.getAttribute('aria-labelledby') ?? '')?.textContent ?? null,
    note: document.getElementById(card.getAttribute('aria-describedby') ?? '')?.textContent ?? null,
    count: text('.links__count'),
    error: text('.links__error'),
    refusals: [...card.querySelectorAll('.links__refusal')].map((li) => [
      li.querySelector('.links__refusalline').textContent,
      li.querySelector('.links__refusalnote').textContent,
    ]),
    helper: text('.links__helper'),
    where: text('.links__where'),
    cancel: text('.links__actions .microlink'),
    button: button === null ? null : button.textContent,
    disabled: button === null ? null : button.disabled,
    boxes: document.querySelectorAll('.links').length,
    veils: document.querySelectorAll('.promptveil').length,
    focus: document.activeElement?.className ?? null,
  };
}
"""

# Where a real ⌘V lands: on whatever holds the keyboard, and up from there to the window. The return
# value is the other half of the question — a paste the app does not want must be left for the page.
PASTE = """
(text) => {
  const dt = new DataTransfer();
  dt.setData('text/plain', text);
  const event = new ClipboardEvent('paste', { clipboardData: dt, bubbles: true, cancelable: true });
  (document.activeElement ?? document.body).dispatchEvent(event);
  return event.defaultPrevented;
}
"""


async def open_links(page: Page) -> dict:
    """⌘L, and the card as it stands once the backend has answered.

    The destination line is the wait, not a timeout: `openLinks` re-reads `get_link_support` behind
    the box, and the cap, the hosts, the helper warning and the folder all come from that answer —
    so a check that ran before it landed would be asserting the fallback instead of the backend.
    """
    await page.keyboard.press("Control+l")
    await page.wait_for_selector(".links__where")
    return await page.evaluate(SHEET)


async def judged(page: Page, accepted: int, refused: int = 0, timeout: float = 4000) -> bool:
    """Wait until the box's live judgement is the backend's answer about what is in it *now*.

    The box asks `inspect_links` behind a 200ms debounce, so what is on screen immediately after a
    paste is still the previous answer — and the count line is the observable proof that the newer
    one has arrived. Waiting on it rather than on the debounce is what keeps this section
    deterministic; it also happens to be the thing a stale answer would get wrong.
    """
    try:
        await page.wait_for_function(
            "([a, r]) => { const line = document.querySelector('.links__count');"
            " if (line === null) return false;"
            " const said = Number((line.textContent ?? '').trim().split(' ')[0]);"
            " return said === a && document.querySelectorAll('.links__refusal').length === r; }",
            arg=[accepted, refused],
            timeout=timeout,
        )
        return True
    except PWTimeout:
        return False


async def paste_box(browser) -> None:
    """The box itself: a modal in the install question's idiom, and the keys that belong to it."""
    page = await fresh(browser)
    check(
        "246 the empty canvas names the third way in, beside the click and the drop",
        (await page.text_content(".dropzone__hint") or "").replace("\u2003", " ")
        == "Click to choose · or drop · or paste a link",
        await page.text_content(".dropzone__hint"),
    )

    await page.focus(".dropzone")
    sheet = await open_links(page)
    check(
        "247 ⌘L opens a real dialog, with the box already holding the keyboard",
        sheet is not None
        and sheet["role"] == "dialog"
        and sheet["modal"] == "true"
        and sheet["title"] == "Paste links"
        and sheet["boxes"] == 1
        and sheet["veils"] == 1
        and sheet["focus"] == "links__box",
        sheet,
    )
    check(
        "248 and it says what it takes: the two sites, one per line, and how many at once",
        "YouTube" in (sheet["note"] or "")
        and "Bilibili" in (sheet["note"] or "")
        and "one per line" in (sheet["note"] or "")
        and "20" in (sheet["note"] or "")
        and sheet["cancel"] == "Cancel"
        and sheet["button"] == "Add link"
        and sheet["disabled"] is True,
        [sheet["note"], sheet["button"], sheet["disabled"]],
    )

    await page.keyboard.press("Escape")
    await page.wait_for_selector(".links", state="detached")
    check(
        "249 Esc closes it and hands the keyboard back to whatever opened it",
        await page.evaluate("() => document.activeElement?.className") == "dropzone",
        await page.evaluate("() => document.activeElement?.className"),
    )

    # The veil is the whole reason the card can be a card: a click that went through it would open
    # the file chooser behind — which is exactly what the canvas does with a click anywhere.
    await open_links(page)
    reached = await opens_chooser(page, lambda: page.mouse.click(780, 600))
    check(
        "250 the veil swallows the click that dismisses it: the canvas behind never hears it",
        not reached and await page.query_selector(".links") is None,
        reached,
    )

    await open_links(page)
    # Twice round with an empty box: an Add that cannot be pressed is not a stop on the way, so the
    # ring is the box and Cancel. Then again with a link in it, where Add has joined the cycle.
    quiet = []
    for _ in range(3):
        await page.keyboard.press("Tab")
        quiet.append(await page.evaluate("() => document.activeElement?.className"))
    await page.fill(".links__box", YOUTUBE)
    await judged(page, 1)
    await page.focus(".links__box")
    ring = []
    for _ in range(4):
        await page.keyboard.press("Tab")
        ring.append(await page.evaluate("() => document.activeElement?.className"))
    await page.focus(".links__box")
    await page.keyboard.press("Shift+Tab")
    check(
        "251 Tab cycles inside the card, in both directions, and cannot walk out of it",
        quiet == ["microlink", "links__box", "microlink"]
        and ring == ["microlink", "promptbutton", "links__box", "microlink"]
        and await page.evaluate("() => document.activeElement?.className") == "promptbutton"
        and await page.evaluate(
            "() => document.querySelector('.links').contains(document.activeElement)"
        ),
        [quiet, ring, await page.evaluate("() => document.activeElement?.className")],
    )

    # ⌘L on a box that is already open is the user asking for the box they are looking at. Re-seeding
    # it would silently empty a paste they had begun editing.
    await page.fill(".links__box", YOUTUBE)
    await judged(page, 1)
    await page.keyboard.press("Control+l")
    await page.wait_for_timeout(300)
    check(
        "252 ⌘L on an open box leaves what is in it alone, and opens no second card",
        await page.input_value(".links__box") == YOUTUBE
        and await page.eval_on_selector_all(".links", "els => els.length") == 1,
        await page.input_value(".links__box"),
    )

    # A refusal is about text. Deleting the text has to take it with it: `checkLinks` clears the line
    # on its next answer, but an empty box is never sent to the backend at all, so "You pasted 21
    # links" used to stand beside a box with nothing in it.
    await page.fill(".links__box", "\n".join([f"{YOUTUBE}&i={n}" for n in range(21)]))
    await page.wait_for_selector(".links__error")
    await page.fill(".links__box", "")
    await page.wait_for_selector(".links__error", state="detached")
    empty = await page.evaluate(SHEET)
    check(
        "253 backspacing the paste away takes its refusal with it",
        empty["error"] is None and empty["count"] is None and empty["disabled"] is True,
        empty,
    )
    await page.keyboard.press("Escape")
    await page.wait_for_selector(".links", state="detached")

    # The other half of a modal over a queue: the scrim swallows clicks, so the keyboard must not
    # reach the rows either — and once the box has gone, ⌫ has to work again (the same rule as 158).
    await page.evaluate(DROP, ["one.mov", "two.mov"])
    await page.wait_for_function("() => document.querySelectorAll('.row').length === 2")
    await page.click(".row")
    await open_links(page)
    await page.keyboard.press("Tab")
    await page.keyboard.press("Backspace")
    await page.wait_for_timeout(300)
    still = await page.eval_on_selector_all(".row", "els => els.length")
    await page.keyboard.press("Escape")
    await page.wait_for_selector(".links", state="detached")
    await page.keyboard.press("Backspace")
    check(
        "254 ⌫ cannot reach the queue through the box, and reaches it again once the box has gone",
        still == 2 and await rows_settle(page, 1),
        [still, await page.eval_on_selector_all(".row", "els => els.length")],
    )
    check("255 the box produced no page errors", ERRORS[page] == [], ERRORS[page])
    await page.context.close()


async def paste_anywhere(browser) -> None:
    """⌘V is the feature, not a shortcut for it: the box opens already filled in."""
    page = await fresh(browser)
    claimed = await page.evaluate(PASTE, YOUTUBE)
    await page.wait_for_selector(".links__where")
    filled = await page.input_value(".links__box")
    check(
        "256 ⌘V anywhere with a video link opens the box already filled in, and claims the paste",
        claimed is True and filled == YOUTUBE,
        [claimed, filled],
    )
    check(
        "257 and it is judged without a further keystroke",
        await judged(page, 1)
        and (await page.text_content(".links__count")) == "1 of 20 links"
        and await page.evaluate("() => !document.querySelector('.links .promptbutton').disabled"),
        await page.text_content(".links__count"),
    )

    # Nobody clicked to open this one, so there is no opener to hand the keyboard back to. `<body>`
    # is not an answer: the ring vanishes and the next Tab starts again at the top of the window.
    await page.keyboard.press("Escape")
    await page.wait_for_selector(".links", state="detached")
    check(
        "258 closing a box nobody opened puts the keyboard on the canvas, not on the window",
        await page.evaluate("() => document.activeElement?.className") == "dropzone",
        await page.evaluate("() => document.activeElement?.className"),
    )

    # The host test is the *backend's* list, which is why a short link works: nothing in the frontend
    # spells "b23.tv", and no substring of "youtube"/"bilibili" is in it.
    claimed_short = await page.evaluate(PASTE, BILIBILI_SHORT)
    await page.wait_for_selector(".links__where")
    check(
        "259 a host only `get_link_support` knows about still opens it",
        claimed_short is True
        and await page.input_value(".links__box") == BILIBILI_SHORT
        and await judged(page, 1),
        await page.input_value(".links__box"),
    )
    await page.keyboard.press("Escape")
    await page.wait_for_selector(".links", state="detached")

    ignored = await page.evaluate(PASTE, "notes from the meeting: ship it on Friday")
    await page.wait_for_timeout(400)
    check(
        "260 a clipboard with no link in it opens nothing and is left for the page",
        ignored is False and await page.query_selector(".links") is None,
        ignored,
    )

    # A lookalike host mentions an accepted one, so the box does open — and then the backend says
    # exactly what is wrong with it, which is the only place that judgement is ever made.
    await page.evaluate(PASTE, LOOKALIKE)
    await page.wait_for_selector(".links__where")
    caught = await judged(page, 0, 1)
    sheet = await page.evaluate(SHEET)
    check(
        "261 a lookalike host is opened and then refused in the backend's own words",
        caught and sheet["refusals"] == [[LOOKALIKE, REFUSED["lookalike"]]],
        sheet["refusals"],
    )
    await page.keyboard.press("Escape")
    await page.wait_for_selector(".links", state="detached")

    # And a paste aimed at a control that owns its own keyboard is left to it.
    await page.evaluate(DROP, ["clip.mov"])
    await page.wait_for_selector(".row")
    await page.focus(".row .target__select")
    stolen = await page.evaluate(PASTE, YOUTUBE)
    await page.wait_for_timeout(400)
    check(
        "262 a paste into a control that types is not claimed from it",
        stolen is False and await page.query_selector(".links") is None,
        stolen,
    )
    check("263 ⌘V anywhere produced no page errors", ERRORS[page] == [], ERRORS[page])
    await page.context.close()


async def the_cap_is_the_backends(browser) -> None:
    """Every number in the card is `get_link_support`'s, and `?maxlinks=` is how that is proved."""
    page = await fresh(browser)
    await open_links(page)
    await page.fill(".links__box", f"{YOUTUBE}\n{BILIBILI}")
    await judged(page, 2)
    real = await page.evaluate(SHEET)
    check(
        "264 with the shipped cap the sentence and the count say the same 20",
        real["count"] == "2 of 20 links" and "up to 20 at a time" in (real["note"] or ""),
        [real["note"], real["count"]],
    )

    # The whole paste is refused over the cap, in Rust's own words — twenty-one links is a mistake to
    # correct, not twenty-one rows to render, and the message names what it actually got.
    await page.fill(".links__box", "\n".join([f"{YOUTUBE}&i={n}" for n in range(21)]))
    await page.wait_for_selector(".links__error")
    over = await page.evaluate(SHEET)
    check(
        "265 a paste of 21 is refused whole, saying what it got and what to remove",
        over["error"] == "You pasted 21 links. Flint takes 20 at a time - remove 1 "
        "and paste them as a second batch."
        and over["count"] is None
        and over["refusals"] == []
        and over["disabled"] is True,
        over,
    )
    check("266 the refused paste produced no page errors", ERRORS[page] == [], ERRORS[page])
    await page.context.close()

    # Now the same card against a backend that answers differently. Nothing in the frontend can know
    # this number, so every place it appears has been read through `get_link_support`.
    page = await fresh(browser, query="?maxlinks=3")
    await open_links(page)
    await page.fill(".links__box", f"{YOUTUBE}\n{BILIBILI}")
    await judged(page, 2)
    lowered = await page.evaluate(SHEET)
    check(
        "267 a backend that says 3 moves the sentence and the count with it",
        lowered["count"] == "2 of 3 links" and "up to 3 at a time" in (lowered["note"] or ""),
        [lowered["note"], lowered["count"]],
    )
    await page.fill(".links__box", "\n".join([f"{YOUTUBE}&i={n}" for n in range(4)]))
    await page.wait_for_selector(".links__error")
    check(
        "268 and refuses a paste of 4 in its own words, quoting its own cap",
        await page.text_content(".links__error") == "You pasted 4 links. Flint takes "
        "3 at a time - remove 1 and paste them as a second batch.",
        await page.text_content(".links__error"),
    )

    # The cap is over the whole queue, not over one paste: three links pasted twice is still three
    # rows, and what was left out is said in the quiet slot rather than silently dropped.
    await page.fill(".links__box", f"{YOUTUBE}\n{BILIBILI}")
    await judged(page, 2)
    await page.click(".links .promptbutton")
    await page.wait_for_function("() => document.querySelectorAll('.row').length === 2")
    await open_links(page)
    await page.fill(".links__box", f"{YOUTUBE_SHORT}\n{BILIBILI_SHORT}")
    await judged(page, 2)
    await page.click(".links .promptbutton")
    await page.wait_for_function("() => document.querySelectorAll('.row').length === 3")
    await page.wait_for_selector(".toast--quiet")
    check(
        "269 the cap is over the queue, and the link it would not take says so",
        await page.evaluate(TOAST)
        == "Only 3 links can be queued at once. Convert these, then paste the rest.",
        await page.evaluate(TOAST),
    )

    # A paste the queue has already seen adds nothing — one URL, one row, or two rows would race for
    # one output path. The box cannot see the queue, so it offers "Add 2 links" either way; closing
    # over an unchanged queue in silence is what read as a click that went nowhere.
    await page.evaluate("() => document.querySelector('.toast--quiet .iconbutton').click()")
    await page.wait_for_selector(".toast--quiet", state="detached")
    await open_links(page)
    await page.fill(".links__box", f"{YOUTUBE}\n{BILIBILI}")
    await judged(page, 2)
    await page.click(".links .promptbutton")
    await page.wait_for_selector(".toast--quiet")
    check(
        "270 a link the queue already holds is not queued twice, and is not dropped in silence",
        await page.eval_on_selector_all(".row", "els => els.length") == 3
        and await page.evaluate(TOAST) == "Those 2 links are already in the queue.",
        [await page.eval_on_selector_all(".row", "els => els.length"), await page.evaluate(TOAST)],
    )
    check("271 the queue-wide cap produced no page errors", ERRORS[page] == [], ERRORS[page])
    await page.context.close()


async def a_mixed_paste(browser) -> None:
    """One paste, four verdicts: the good links queue, the rest are refused a line at a time."""
    page = await fresh(browser)
    await open_links(page)
    await page.fill(
        ".links__box",
        "\n".join([YOUTUBE, PLAYLIST, BILIBILI, CHANNEL, NOT_A_VIDEO_PAGE, ELSEWHERE]),
    )
    settled = await judged(page, 2, 4)
    sheet = await page.evaluate(SHEET)
    check(
        "272 a mixed paste counts only what can be converted",
        settled and sheet["count"] == "2 of 20 links" and sheet["error"] is None,
        [sheet["count"], sheet["error"]],
    )
    check(
        "273 and the primary action offers exactly those",
        sheet["button"] == "Add 2 links" and sheet["disabled"] is False,
        [sheet["button"], sheet["disabled"]],
    )
    check(
        "274 each refused line is named, with the backend's own sentence against it",
        sheet["refusals"]
        == [
            [PLAYLIST, REFUSED["playlist"]],
            [CHANNEL, REFUSED["channel"]],
            [NOT_A_VIDEO_PAGE, REFUSED["page"]],
            [ELSEWHERE, REFUSED["host"]],
        ],
        sheet["refusals"],
    )

    await page.click(".links .promptbutton")
    await page.wait_for_function("() => document.querySelectorAll('.row').length === 2")
    check(
        "275 and only those two reach the queue",
        await page.evaluate(ROW_NAMES)
        == ["youtube.com/watch?v=dQw4w9WgXcQ", "bilibili.com/video/BV1GJ411x7h7"],
        await page.evaluate(ROW_NAMES),
    )
    check("276 the mixed paste produced no page errors", ERRORS[page] == [], ERRORS[page])
    await page.context.close()


# The row a link becomes. It is the same row as a file's, with the three facts a URL does not have
# substituted rather than left blank: the gutter carries the site because there is no extension, the
# name is the URL until the backend has resolved a title, and nothing anywhere estimates an output
# path from `info.path`, because a link's is the empty string and `~/Downloads` is the honest answer.

LINK_ROWS = """
() => [...document.querySelectorAll('.row')].map((row) => ({
  kind: row.querySelector('.row__kind').textContent,
  name: row.querySelector('.row__name').textContent,
  tooltip: row.querySelector('.row__name').getAttribute('title'),
  meta: row.querySelector('.row__meta').textContent,
  target: row.querySelector('.target__select')?.value ?? null,
  picker: row.querySelector('.target__select')?.getAttribute('aria-label') ?? null,
  removable: row.querySelector('.row__remove')?.disabled === false,
}))
"""


async def links_in_the_queue(browser) -> None:
    """A queued link is one more row in the list, and it lands where the backend says it lands."""
    page = await fresh(browser)
    await open_links(page)
    await page.fill(".links__box", f"{YOUTUBE}\n{BILIBILI}")
    await judged(page, 2)
    await page.click(".links .promptbutton")
    await page.wait_for_function("() => document.querySelectorAll('.row').length === 2")
    rows = await page.evaluate(LINK_ROWS)
    check(
        "277 a link row's gutter is the site, where a file's is its extension",
        [row["kind"] for row in rows] == ["YT", "BILI"],
        [row["kind"] for row in rows],
    )
    check(
        "278 it shows the URL until a title exists, and keeps the whole address as its tooltip",
        [row["name"] for row in rows]
        == ["youtube.com/watch?v=dQw4w9WgXcQ", "bilibili.com/video/BV1GJ411x7h7"]
        and [row["tooltip"] for row in rows] == [YOUTUBE, BILIBILI]
        and [row["meta"] for row in rows] == ["YouTube", "Bilibili"],
        rows,
    )
    check(
        "279 a link is a video, so it gets the video picker — and the row can still be removed",
        all(row["target"] == "mp4" for row in rows)
        and rows[0]["picker"] == "Convert youtube.com/watch?v=dQw4w9WgXcQ to"
        and all(row["removable"] for row in rows),
        [[row["target"], row["picker"], row["removable"]] for row in rows],
    )
    check(
        "280 and the queue counts it like everything else",
        await page.text_content(".titlebar__count") == "2 files",
        await page.text_content(".titlebar__count"),
    )

    # Where they land is `LinkSupport.destination` — a folder, not a file whose parent is one. The
    # bar used to chop the last component off it and name `/Users/you` under a queue of links.
    await page.wait_for_function(
        "() => (document.querySelector('.actionbar__text')?.textContent ?? '')"
        ".startsWith('Saves to')"
    )
    check(
        "281 a queue of nothing but links says where they land, in the backend's own folder",
        await page.evaluate(INHERITED_STATUS) == "Saves to /Users/you/Downloads",
        await page.evaluate(INHERITED_STATUS),
    )
    check(
        "282 and no link row is ever asked to estimate an output path",
        await page.evaluate("() => window.__ceMockEstimates") == [],
        await page.evaluate("() => window.__ceMockEstimates"),
    )

    # A file joining them is the other half of the same fact: the estimate is asked about the file,
    # and never about the row whose path is "".
    await page.evaluate(DROP, ["clip.mov"])
    await page.wait_for_function("() => document.querySelectorAll('.row').length === 3")
    await page.wait_for_function(
        "() => (document.querySelector('.actionbar__text')?.textContent ?? '')"
        ".startsWith('Saves to /Users/you/Desktop')"
    )
    check(
        "283 with a file in the queue the estimate is asked about the file alone",
        await page.evaluate("() => window.__ceMockEstimates") == ["/Users/you/Desktop/clip.mov"],
        await page.evaluate("() => window.__ceMockEstimates"),
    )
    await page.click(".row:first-child .row__remove")
    check(
        "284 and a link row leaves the queue when its × is pressed",
        await rows_settle(page, 2)
        and await page.evaluate(ROW_NAMES) == ["bilibili.com/video/BV1GJ411x7h7", "clip.mov"],
        await page.evaluate(ROW_NAMES),
    )
    check("285 queued links produced no page errors", ERRORS[page] == [], ERRORS[page])
    await page.context.close()



PHASE_LOG = """
() => {
  // Every value the rows' status lines and progress lines ever hold, in order. A MutationObserver
  // rather than a poll: `Downloading…` can be on screen for one tick of the mock's clock, and a
  // sampler running every frame would be entitled to miss it — which is the one value that proves
  // the indeterminate first sample is not printed as "0%".
  const log = [[], []];
  const sample = () => {
    [...document.querySelectorAll('.row')].forEach((row, i) => {
      const status = row.querySelector('.row__status');
      const list = log[i];
      if (status === null || list === undefined) return;
      const bar = row.querySelector('.row__progress');
      const width = bar === null ? null : Math.round(parseFloat(bar.style.width));
      const last = list[list.length - 1];
      if (last === undefined || last[0] !== status.textContent || last[1] !== width) {
        list.push([status.textContent, width]);
      }
    });
  };
  const observer = new MutationObserver(sample);
  observer.observe(document.querySelector('.filelist__rows'), {
    subtree: true, childList: true, characterData: true, attributes: true,
  });
  window.__cePhaseStop = () => observer.disconnect();
  window.__cePhaseLog = log;
  sample();
}
"""


async def both_halves_of_a_link(browser) -> None:
    """One batch, one link and one file: the link reports two halves and the file reports one."""
    page = await fresh(browser, query="?linkms=20")
    await open_links(page)
    await page.fill(".links__box", YOUTUBE)
    await judged(page, 1)
    await page.click(".links .promptbutton")
    await page.wait_for_function("() => document.querySelectorAll('.row').length === 1")
    await page.evaluate(DROP, ["clip.mov"])
    await page.wait_for_function("() => document.querySelectorAll('.row').length === 2")

    await page.evaluate(PHASE_LOG)
    await convert_and_settle(page)
    log = await page.evaluate("() => { window.__cePhaseStop(); return window.__cePhaseLog; }")
    link, clip = [t for t, _ in log[0]], [t for t, _ in log[1]]
    check(
        "286 a link's first frame says it is fetching, without inventing a percentage for it",
        link[0] == "Downloading…",
        link[:3],
    )
    downloads = [i for i, t in enumerate(link) if t.startswith("Downloading")]
    converts = [i for i, t in enumerate(link) if t.startswith("Converting")]
    check(
        "287 then it names the half it is in, download before convert, and nothing else",
        len(downloads) > 1
        and len(converts) > 1
        and max(downloads) < min(converts)
        and all(
            re.fullmatch(r"(Downloading|Converting)( \d+%| …|…)( · .+)?", t) for t in link
        ),
        [link[0], link[max(downloads)], link[min(converts)], link[-1]],
    )
    check(
        "288 a file in the same batch never says it is downloading: it only ever converts",
        len(clip) > 1
        and all(re.fullmatch(r"\d+%( · .+)?", t) for t in clip)
        and not any("Download" in t or "Converting" in t for t in clip),
        clip[:4],
    )

    widths = [w for _, w in log[0] if w is not None]
    fetching = [w for t, w in log[0] if t.startswith("Downloading") and w is not None]
    converting = [w for t, w in log[0] if t.startswith("Converting") and w is not None]
    check(
        "289 and the line under it only ever grows: the fetch is the first half of one row's work",
        widths == sorted(widths)
        and max(fetching) <= 50
        and min(converting) >= 50,
        [max(fetching), min(converting), widths[:3], widths[-3:]],
    )
    check(
        "290 the resolved title replaces the URL once the backend has announced it",
        await page.evaluate(STATUSES) == ["done", "done"]
        and (await page.evaluate(ROW_NAMES))[0]
        == "How we shipped a Tauri app in three weeks (and what broke)",
        await page.evaluate(ROW_NAMES),
    )
    check("291 converting a link produced no page errors", ERRORS[page] == [], ERRORS[page])
    await page.context.close()


async def without_yt_dlp(browser) -> None:
    """The helper a link needs, absent: said in the box, honest in the row, asked about once."""
    page = await fresh(browser, query="?missing=yt-dlp&linkms=20")
    sheet = await open_links(page)
    check(
        "292 the box says the helper is missing, and offers the one click that fixes it",
        "yt-dlp" in (sheet["helper"] or "")
        and "not installed" in (sheet["helper"] or "")
        and await page.text_content(".links__helper .microlink") == "Install yt-dlp…",
        sheet["helper"],
    )
    await page.click(".links__helper .microlink")
    await page.wait_for_selector(".tool[data-highlight]")
    await page.wait_for_timeout(500)
    check(
        "293 that microlink lands in Settings on yt-dlp, with Install focused and unpressed",
        await page.query_selector(".links") is None
        and await page.get_attribute(tool("yt-dlp"), "data-highlight") is not None
        and await page.eval_on_selector_all(".tool[data-highlight]", "els => els.length") == 1
        and await page.evaluate("() => document.activeElement?.getAttribute('aria-label')")
        == "Install yt-dlp"
        and await page.query_selector(".install") is None,
        await page.evaluate("() => document.activeElement?.getAttribute('aria-label')"),
    )
    await page.keyboard.press("Escape")
    await page.wait_for_selector(".drawer:not([data-open])")

    # Refusing to queue would be a second opinion about a fact the row can state for itself.
    await open_links(page)
    await page.fill(".links__box", YOUTUBE)
    await judged(page, 1)
    check(
        "294 links can still be queued with the helper missing: the box does not refuse them",
        await page.evaluate("() => !document.querySelector('.links .promptbutton').disabled"),
        await page.evaluate(SHEET),
    )
    await page.click(".links .promptbutton")
    await page.wait_for_function("() => document.querySelectorAll('.row').length === 1")
    await page.click(".pill")
    await page.wait_for_selector(".prompt", timeout=60000)
    await page.wait_for_timeout(300)
    check(
        "295 the row fails with the sentence that says how to fix it, and its own way there",
        await page.evaluate(STATUSES) == ["failed"]
        and await page.text_content(".row__error") == "yt-dlp is not installed. Open Settings → "
        "Helpers and install it (one click), then try again."
        and await page.text_content(".row__fix") == "Install yt-dlp",
        [await page.text_content(".row__error"), await page.text_content(".row__fix")],
    )
    body = await page.text_content(".prompt__body")
    check(
        "296 and the settled batch asks about it once, naming the link rather than a format",
        "A pasted link needs yt-dlp" in (body or "")
        and (await page.text_content(".promptbutton") or "").startswith("Install yt-dlp"),
        body,
    )
    await page.keyboard.press("Enter")
    await page.wait_for_selector(".tool[data-highlight]")
    await page.wait_for_timeout(600)
    check(
        "297 Yes lands on yt-dlp's Install button in Settings without pressing it",
        await page.query_selector(".prompt") is None
        and await page.get_attribute(".drawer", "data-open") is not None
        and await page.evaluate("() => document.activeElement?.getAttribute('aria-label')")
        == "Install yt-dlp"
        and await page.query_selector(".install") is None
        and await page.text_content(f"{tool('yt-dlp')} .tool__state") == "Missing",
        await page.text_content(f"{tool('yt-dlp')} .tool__state"),
    )
    check("298 the missing-helper path produced no page errors", ERRORS[page] == [], ERRORS[page])
    await page.context.close()


async def one_thing_over_the_window(browser) -> None:
    """Three things can cover this window, and only ever one of them at a time.

    ⌘, is a native menu accelerator and stays live under the box, so it has to *answer* the box
    rather than open a sheet behind it: two modals over one window left both focus traps fighting for
    Tab, and Esc closing whichever heard it first. The install question is the one that wins, because
    it is the only one the app raised by itself.
    """
    page = await fresh(browser)
    await open_links(page)
    await page.fill(".links__box", YOUTUBE)
    await judged(page, 1)
    await page.keyboard.press("Control+,")
    await page.wait_for_selector(".tools__list")
    await page.wait_for_timeout(400)
    check(
        "299 opening Settings under the box answers the box instead of stacking on it",
        await page.query_selector(".links") is None
        and await page.eval_on_selector_all(".promptveil", "els => els.length") == 0
        and await page.get_attribute(".drawer", "data-open") is not None,
        await page.eval_on_selector_all(".promptveil", "els => els.length"),
    )
    sheet = await open_links(page)
    check(
        "300 and ⌘L from Settings replaces the sheet, keyboard and all",
        sheet["boxes"] == 1
        and sheet["focus"] == "links__box"
        and await page.get_attribute(".drawer", "data-open") is None,
        [sheet["focus"], await page.get_attribute(".drawer", "data-open")],
    )
    await page.keyboard.press("Escape")
    await page.wait_for_selector(".links", state="detached")
    await page.context.close()

    # The install question owns the window until it is answered: a box over it would cover the thing
    # the app went out of its way to say, and a ⌘V it swallowed would leave the paste nowhere at all.
    page = await fresh(browser, query="?missing=libreoffice")
    await page.evaluate(DROP, ["notes.md"])
    await page.wait_for_selector(".row")
    await page.click(".pill")
    await page.wait_for_selector(".prompt", timeout=60000)
    await page.wait_for_timeout(300)
    await page.keyboard.press("Control+l")
    await page.wait_for_timeout(300)
    claimed = await page.evaluate(PASTE, YOUTUBE)
    await page.wait_for_timeout(300)
    check(
        "301 with the install question up, neither ⌘L nor ⌘V opens the box over it",
        await page.query_selector(".links") is None
        and claimed is False
        and await page.query_selector(".prompt") is not None,
        claimed,
    )
    # And answering the question gives the box back.
    await page.click(".prompt .microlink")
    await page.wait_for_selector(".prompt", state="detached")
    check(
        "302 and the box is available again the moment it has been answered",
        (await open_links(page))["boxes"] == 1
        and ERRORS[page] == [],
        ERRORS[page],
    )
    await page.context.close()



TRIM_CARD = """
() => {
  const section = [...document.querySelectorAll('.section')]
    .find((s) => s.querySelector('.section__title').textContent === 'Trim');
  if (section === undefined) return null;
  section.open = true;
  return {
    switch: section.querySelector('.check span').textContent,
    on: section.querySelector('.check input').checked,
    fields: [...section.querySelectorAll('.field')].map((field) => ({
      label: field.querySelector('.field__label').firstChild.textContent,
      hint: field.querySelector('.field__hint')?.textContent ?? null,
      value: field.querySelector('input').value,
      inert: field.querySelector('input').disabled,
    })),
    note: section.querySelector('.section__note')?.textContent ?? null,
  };
}
"""

TRIM_TOGGLE = """
() => {
  const section = [...document.querySelectorAll('.section')]
    .find((s) => s.querySelector('.section__title').textContent === 'Trim');
  section.open = true;
  section.querySelector('.check input').click();
}
"""

# Typed the way a person types: focus, the characters, and — when the third argument says so — the
# blur that drops `SecondsField`'s draft and lets the canonical spelling come back. React listens
# for `input`, so the value is set through the prototype's own setter or the framework never hears
# that anything changed.
TRIM_SET = """
([label, text, blur]) => {
  const section = [...document.querySelectorAll('.section')]
    .find((s) => s.querySelector('.section__title').textContent === 'Trim');
  section.open = true;
  const field = [...section.querySelectorAll('.field')]
    .find((f) => f.querySelector('.field__label').firstChild.textContent === label);
  const input = field.querySelector('input');
  input.focus();
  const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
  setter.call(input, text);
  input.dispatchEvent(new Event('input', { bubbles: true }));
  if (blur) input.blur();
}
"""

# A setting from a different card, to be the witness in the paragraph about the half-typed length:
# it is changed *while* the trim is unfinished, and it has to survive to the next write.
VIDEO_QUALITY = """
(value) => {
  const section = [...document.querySelectorAll('.section')]
    .find((s) => s.querySelector('.section__title').textContent === 'Video');
  section.open = true;
  const select = [...section.querySelectorAll('select')][1];
  select.value = value;
  select.dispatchEvent(new Event('change', { bubbles: true }));
}
"""

# Every settings object the mock has actually written, as the trim and one witness beside it.
TRIM_SAVES = """
() => (window.__ceMockSaves ?? []).map((s) => [
  s.trim.enabled, s.trim.start_secs, s.trim.length_secs, s.video.quality,
])
"""

ROW_METAS = "() => [...document.querySelectorAll('.row__meta')].map((m) => m.textContent)"
BAR_HEIGHT = "() => Math.round(document.querySelector('.actionbar').getBoundingClientRect().height)"


async def trim_written(page: Page, trim: list, timeout: float = 4000) -> bool:
    """Wait until the mock has *written* this trim, rather than until the debounce has probably run.

    The store holds saves for 300ms and drops an unfinished one on the floor, so the write is the
    only observable end of an edit — and it is also the thing a bug here would get wrong, which is
    why every step below waits on it instead of on the clock.
    """
    try:
        await page.wait_for_function(
            "(want) => { const saves = window.__ceMockSaves ?? [];"
            " const last = saves[saves.length - 1];"
            " return last !== undefined && last.trim.enabled === want[0]"
            " && last.trim.start_secs === want[1] && last.trim.length_secs === want[2]; }",
            arg=trim,
            timeout=timeout,
        )
        return True
    except PWTimeout:
        return False


async def the_trim_card(browser) -> None:
    """Two numbers and a switch, and the discipline typing into them comes with."""
    page = await fresh(browser)
    await open_settings(page)
    card = await page.evaluate(TRIM_CARD)
    check(
        "303 the trim is off, and both of its boxes are inert until it is not",
        card is not None
        and card["on"] is False
        and card["switch"] == "Trim every clip to the same length"
        and [f["label"] for f in card["fields"]] == ["Start at", "Keep"]
        and [f["value"] for f in card["fields"]] == ["0", "10"]
        and all(f["inert"] for f in card["fields"])
        and all(f["hint"] == "seconds, or m:ss" for f in card["fields"]),
        card,
    )
    check(
        "304 and the card says what the cut reaches, and what it leaves alone",
        card["note"] == "Applies to every video and audio file in the queue, pasted links included."
        " Anything shorter than that keeps its own length; images, documents and subtitles are"
        " untouched.",
        card["note"],
    )

    await page.evaluate(TRIM_TOGGLE)
    written = await trim_written(page, [True, 0, 10])
    woken = await page.evaluate(TRIM_CARD)
    check(
        "305 turning it on wakes both boxes without touching what they say",
        written
        and woken["on"] is True
        and not any(f["inert"] for f in woken["fields"])
        and [f["value"] for f in woken["fields"]] == ["0", "10"],
        [woken["fields"], await page.evaluate(TRIM_SAVES)],
    )

    # `10` and `1:05` are the same field, because people write both. What comes back on blur is the
    # canonical spelling of what was stored, which `parseSeconds` has to accept in its turn.
    await page.evaluate(TRIM_SET, ["Start at", "1:05", True])
    minutes = await trim_written(page, [True, 65, 10])
    await page.evaluate(TRIM_SET, ["Keep", "20", True])
    seconds = await trim_written(page, [True, 65, 20])
    typed = await page.evaluate(TRIM_CARD)
    check(
        "306 a box takes seconds or m:ss, and hands the time back in its own spelling",
        minutes and seconds and [f["value"] for f in typed["fields"]] == ["1:05", "20"],
        [[f["value"] for f in typed["fields"]], (await page.evaluate(TRIM_SAVES))[-1]],
    )

    # A length being retyped is empty for as long as it takes to type the new one, and `1:05` passes
    # through `1:` on the way in. `classify_trim` calls that unfinished rather than wrong, so the
    # saved trim stays at its last usable value while the field remains empty on screen.
    before = await page.evaluate(TRIM_SAVES)
    await page.evaluate(TRIM_SET, ["Keep", "", False])
    await page.wait_for_timeout(900)
    check(
        "307 a length mid-retype stays silent and preserves the last usable trim",
        await page.evaluate(TOAST) is None
        and (await page.evaluate(TRIM_SAVES))[-1] == before[-1]
        and (await page.evaluate(TRIM_CARD))["fields"][1]["value"] == "",
        [await page.evaluate(TOAST), (await page.evaluate(TRIM_SAVES))[len(before):]],
    )

    # Independent edits persist immediately, even if the user quits before finishing the trim.
    await page.evaluate(VIDEO_QUALITY, "high")
    await page.wait_for_timeout(700)
    held = await page.evaluate(TRIM_SAVES)
    await page.evaluate(TRIM_SET, ["Keep", "5", False])
    landed = await trim_written(page, [True, 65, 5])
    check(
        "308 and the setting changed beside it is kept, not swallowed with it",
        await page.evaluate(TOAST) is None
        and held[-1] == [True, 65, 20, "high"]
        and landed
        and (await page.evaluate(TRIM_SAVES))[-1] == [True, 65, 5, "high"],
        [held[-1], (await page.evaluate(TRIM_SAVES))[-1]],
    )

    # A number the user really typed is a different matter, and its refusal is the backend's to
    # give. The trim that was already usable stays written while the refused one stands on screen.
    await page.evaluate(TRIM_SET, ["Start at", "-5", False])
    await page.wait_for_selector(".toast")
    check(
        "309 a negative start comes back in the backend's words, with the last usable trim kept",
        await page.evaluate(TOAST) == "The trim start cannot be negative."
        and (await page.evaluate(TRIM_SAVES))[-1] == [True, 65, 5, "high"],
        [await page.evaluate(TOAST), (await page.evaluate(TRIM_SAVES))[-1]],
    )

    # Put the start back first: the two fields are read in the order they are on screen, so a start
    # still holding `-5` would answer the next save with the start's sentence again.
    await page.click(".toast .iconbutton")
    await page.wait_for_selector(".toast", state="detached")
    await page.evaluate(TRIM_SET, ["Start at", "0", False])
    await trim_written(page, [True, 0, 5])
    await page.evaluate(TRIM_SET, ["Keep", "100000", False])
    await page.wait_for_selector(".toast")
    check(
        "310 and so does a length longer than a day, naming the field the user typed in",
        await page.evaluate(TOAST) == "The trim length cannot be longer than 24 hours."
        and (await page.evaluate(TRIM_SAVES))[-1] == [True, 0, 5, "high"],
        [await page.evaluate(TOAST), (await page.evaluate(TRIM_SAVES))[-1]],
    )
    check("311 the trim card produced no page errors", ERRORS[page] == [], ERRORS[page])
    await page.context.close()


async def the_trim_where_the_files_land(browser) -> None:
    """What the window says about the cut, in the bar and on the rows that it changes.

    `?clipsecs=90` fixes every clip in this page at a minute and a half, because the whole section
    is about numbers the user can read: the mock otherwise invents a duration per filename, and
    "shorter than the cut" would be a matter of finding the right one.
    """
    page = await fresh(browser, query="?clipsecs=90")
    await page.evaluate(DROP, ["clip.mov", "poster.png", "notes.md"])
    await page.wait_for_function("() => document.querySelectorAll('.row').length === 3")
    await page.wait_for_function(
        "() => (document.querySelector('.actionbar__text')?.textContent ?? '')"
        ".startsWith('Saves to')"
    )
    quiet_bar = await page.evaluate(INHERITED_STATUS)
    quiet_metas = await page.evaluate(ROW_METAS)
    quiet_height = await page.evaluate(BAR_HEIGHT)
    check(
        "312 with the trim off the bar says only where the files land, and no row mentions a cut",
        quiet_bar == "Saves to /Users/you/Desktop/Converted"
        and not any("Trimmed" in meta for meta in quiet_metas),
        [quiet_bar, quiet_metas],
    )

    await open_settings(page)
    await page.evaluate(TRIM_TOGGLE)
    await trim_written(page, [True, 0, 10])
    await page.keyboard.press("Escape")
    await page.wait_for_function(
        "() => (document.querySelector('.actionbar__text')?.textContent ?? '').includes('Trimming')"
    )
    metas = await page.evaluate(ROW_METAS)
    check(
        "313 turning it on states the cut beside where the files land, in one line",
        await page.evaluate(INHERITED_STATUS)
        == "Saves to /Users/you/Desktop/Converted · Trimming to 0:10",
        await page.evaluate(INHERITED_STATUS),
    )
    check(
        "314 the clip says what it will become, in the words its own length is already in",
        metas[0] == f"{quiet_metas[0]} · Trimmed to 0:10",
        metas[0],
    )
    check(
        "315 while the image and the document say nothing new: neither has a timeline to cut",
        metas[1:] == quiet_metas[1:],
        [metas[1:], quiet_metas[1:]],
    )

    # A start is named only when there is one, because "from 0:00" is noise in a line with a folder
    # path in it. Ten seconds from 1:05 of a 1:30 clip is still ten seconds, so the row is unmoved.
    await open_settings(page)
    await page.evaluate(TRIM_SET, ["Start at", "1:05", True])
    await trim_written(page, [True, 65, 10])
    await page.keyboard.press("Escape")
    await page.wait_for_function(
        "() => (document.querySelector('.actionbar__text')?.textContent ?? '').includes('from')"
    )
    check(
        "316 and naming a start says which ten seconds, without changing what the row promises",
        await page.evaluate(INHERITED_STATUS)
        == "Saves to /Users/you/Desktop/Converted · Trimming to 0:10 from 1:05"
        and (await page.evaluate(ROW_METAS))[0] == metas[0],
        [await page.evaluate(INHERITED_STATUS), (await page.evaluate(ROW_METAS))[0]],
    )
    check(
        "317 and the bar does not grow to say any of it",
        await page.evaluate(BAR_HEIGHT) == quiet_height,
        [quiet_height, await page.evaluate(BAR_HEIGHT)],
    )

    # A cut longer than the clip is not a cut: FFmpeg stops when the input ends, so the file keeps
    # its own length and the row says nothing rather than promising ten minutes of a 1:30 clip.
    await open_settings(page)
    await page.evaluate(TRIM_SET, ["Start at", "0", True])
    await page.evaluate(TRIM_SET, ["Keep", "10:00", True])
    await trim_written(page, [True, 0, 600])
    await page.keyboard.press("Escape")
    await page.wait_for_function(
        "() => (document.querySelector('.actionbar__text')?.textContent ?? '')"
        ".includes('Trimming to 10:00')"
    )
    check(
        "318 a clip shorter than the cut keeps its own length, and claims nothing it cannot keep",
        await page.evaluate(ROW_METAS) == quiet_metas,
        await page.evaluate(ROW_METAS),
    )

    await open_settings(page)
    await page.evaluate(TRIM_TOGGLE)
    await trim_written(page, [False, 0, 600])
    await page.keyboard.press("Escape")
    await page.wait_for_function(
        "() => !(document.querySelector('.actionbar__text')?.textContent ?? '')"
        ".includes('Trimming')"
    )
    check(
        "319 and switching it off puts the bar and the rows back exactly as they were",
        await page.evaluate(INHERITED_STATUS) == quiet_bar
        and await page.evaluate(ROW_METAS) == quiet_metas
        and await page.evaluate(BAR_HEIGHT) == quiet_height,
        [await page.evaluate(INHERITED_STATUS), await page.evaluate(BAR_HEIGHT)],
    )

    # A preset is an opinion about quality. Which ten seconds of a clip the user wants is not one,
    # and a click on "Smallest file" that quietly untrimmed the batch would be a setting turning
    # itself off — `commands::apply_preset` carries the current trim across for that reason.
    await open_settings(page)
    await page.evaluate(TRIM_TOGGLE)
    await page.evaluate(TRIM_SET, ["Keep", "10", True])
    await trim_written(page, [True, 0, 10])
    await page.click('.preset input[value="smallest"]')
    await page.wait_for_selector('.preset[data-active] input[value="smallest"]')
    kept = await page.evaluate(TRIM_CARD)
    await page.keyboard.press("Escape")
    await page.wait_for_timeout(400)
    check(
        "320 a preset is an opinion about quality, and it does not carry one about the cut",
        kept["on"] is True
        and [f["value"] for f in kept["fields"]] == ["0", "10"]
        and await page.evaluate(INHERITED_STATUS)
        == "Saves to /Users/you/Desktop/Converted · Trimming to 0:10"
        and await page.text_content(".actionbar__preset") == "Smallest file",
        [kept["fields"], await page.evaluate(INHERITED_STATUS)],
    )
    check("321 the trim in the bar produced no page errors", ERRORS[page] == [], ERRORS[page])
    await page.context.close()


async def a_trim_past_the_end(browser) -> None:
    """The two ways a trim stops a batch, and the length both of them are quoted in.

    `?clipsecs=12.6` is a clip whose length has to be spelled to be said at all — `0:12` by
    truncation, as `probe::seconds_label` spells it, and `0:13` by anything that rounds.
    """
    page = await fresh(browser, query="?clipsecs=12.6")
    await page.evaluate(DROP, ["clip.mov"])
    await page.wait_for_selector(".row")
    check(
        "322 a length is truncated to say it, digit for digit with `probe::seconds_label`",
        (await page.evaluate(ROW_METAS))[0].endswith(" · 0:12"),
        await page.evaluate(ROW_METAS),
    )

    # Trimming on with nothing typed in "Keep" is the state the save is held in silence for, and it
    # is exactly where a batch must *not* go quietly: the planner would add no `-ss`/`-t` at all and
    # convert every file whole, which is not what the window says it is about to do.
    await open_settings(page)
    await page.evaluate(TRIM_TOGGLE)
    await trim_written(page, [True, 0, 10])
    await page.evaluate(TRIM_SET, ["Keep", "", False])
    await page.keyboard.press("Escape")
    await page.wait_for_timeout(400)
    await page.click(".pill")
    await page.wait_for_selector(".toast")
    check(
        "323 trimming with no length yet declines the batch instead of converting whole files",
        await page.evaluate(TOAST) == "Enter how many seconds of each file to keep."
        and await page.evaluate(STATUSES) == ["queued"],
        [await page.evaluate(TOAST), await page.evaluate(STATUSES)],
    )

    await page.click(".toast .iconbutton")
    await page.wait_for_selector(".toast", state="detached")
    await open_settings(page)
    await page.evaluate(TRIM_SET, ["Keep", "10", True])
    await page.evaluate(TRIM_SET, ["Start at", "0:30", True])
    await trim_written(page, [True, 30, 10])
    await page.keyboard.press("Escape")
    await page.wait_for_function(
        "() => (document.querySelector('.actionbar__text')?.textContent ?? '').includes('Trimming')"
    )
    check(
        "324 a trim that starts past the end promises nothing on the row it cannot keep",
        not any("Trimmed" in meta for meta in await page.evaluate(ROW_METAS)),
        await page.evaluate(ROW_METAS),
    )

    # Refused before anything is spawned: `-ss` past the end writes a valid empty file, and an empty
    # file reported as a success is the worst outcome available.
    await convert_and_settle(page)
    check(
        "325 and the batch fails that row by name, quoting both lengths as the app spells them",
        await page.evaluate(STATUSES) == ["failed"]
        and await page.text_content(".row__error")
        == "The trim starts at 0:30 and this file is only 0:12 long. Lower the start time, or turn"
        " trimming off."
        and await page.query_selector(".row__output") is None,
        await page.text_content(".row__error"),
    )
    check("326 the refused trim produced no page errors", ERRORS[page] == [], ERRORS[page])
    await page.context.close()


async def a_trimmed_link(browser) -> None:
    """A pasted link is cut like everything else — and only the half after the fetch gets shorter.

    `?clipsecs=600` makes the video ten minutes long, so a ten second cut is 1/60th of it: the
    fetch is of the whole thing either way, and what the trim shortens is the encode after it.
    """
    page = await fresh(browser, query="?linkms=20&clipsecs=600")
    await open_links(page)
    await page.fill(".links__box", YOUTUBE)
    await judged(page, 1)
    await page.click(".links .promptbutton")
    await page.wait_for_function("() => document.querySelectorAll('.row').length === 1")
    await open_settings(page)
    await page.evaluate(TRIM_TOGGLE)
    await trim_written(page, [True, 0, 10])
    await page.keyboard.press("Escape")
    await page.wait_for_function(
        "() => (document.querySelector('.actionbar__text')?.textContent ?? '').includes('Trimming')"
    )
    check(
        "327 a queue of nothing but links is cut like any other, and says so where they land",
        await page.evaluate(INHERITED_STATUS) == "Saves to /Users/you/Downloads · Trimming to 0:10",
        await page.evaluate(INHERITED_STATUS),
    )
    check(
        "328 while the row claims nothing about a length nobody has measured yet",
        await page.evaluate(ROW_METAS) == ["YouTube"],
        await page.evaluate(ROW_METAS),
    )

    await page.evaluate(PHASE_LOG)
    await convert_and_settle(page)
    log = await page.evaluate("() => { window.__cePhaseStop(); return window.__cePhaseLog; }")
    link = [t for t, _ in log[0]]
    downloads = [i for i, t in enumerate(link) if t.startswith("Downloading")]
    converts = [i for i, t in enumerate(link) if t.startswith("Converting")]
    check(
        "329 the fetch is of the whole video, and the cut is the half that follows it",
        link[0] == "Downloading…"
        and len(downloads) > 1
        and len(converts) > 1
        and max(downloads) < min(converts),
        [link[0], link[max(downloads)], link[min(converts)]],
    )
    check(
        "330 and that half is measured against the cut rather than against the film",
        len(converts) * 2 < len(downloads)
        and await page.evaluate(STATUSES) == ["done"]
        and (await page.evaluate(ROW_NAMES))[0]
        == "How we shipped a Tauri app in three weeks (and what broke)",
        [len(downloads), len(converts), await page.evaluate(ROW_NAMES)],
    )
    check("331 the trimmed link produced no page errors", ERRORS[page] == [], ERRORS[page])
    await page.context.close()



LINKS_GROUP = """
() => {
  const section = [...document.querySelectorAll('.section')]
    .find((s) => s.querySelector('.section__title').textContent === 'Links');
  if (section === undefined) return null;
  section.open = true;
  // A CSS variable resolved to the rgb a hint is actually painted in, so "this is a hint and not an
  // error" can be asserted against the app's own two tokens rather than against a literal colour.
  const resolve = (name) => {
    const probe = document.createElement('span');
    probe.style.color = `var(${name})`;
    document.body.appendChild(probe);
    const colour = getComputedStyle(probe).color;
    probe.remove();
    return colour;
  };
  return {
    fields: [...section.querySelectorAll('.field')].map((field) => {
      const hint = field.querySelector('.field__hint');
      const select = field.querySelector('select');
      const button = field.querySelector('button');
      return {
        label: field.querySelector('.field__label').firstChild.textContent,
        hint: hint === null ? null : hint.textContent,
        ink: hint === null ? null : getComputedStyle(hint).color,
        value: select === null ? null : select.value,
        options: [...(select?.options ?? [])].map((o) => [o.value, o.textContent]),
        inert: (select ?? button).disabled,
        button: button === null ? null : button.textContent,
        path: field.querySelector('.filepick__path')?.textContent ?? null,
      };
    }),
    note: section.querySelector('.section__note')?.textContent ?? null,
    quiet: resolve('--ink-3'),
    danger: resolve('--danger'),
  };
}
"""


def links_select(index: int) -> str:
    """Set one of the group's two selects the way a change event reaches React — see `TRIM_SET`."""
    return f"""
(value) => {{
  const section = [...document.querySelectorAll('.section')]
    .find((s) => s.querySelector('.section__title').textContent === 'Links');
  section.open = true;
  const select = section.querySelectorAll('select')[{index}];
  select.value = value;
  select.dispatchEvent(new Event('change', {{ bubbles: true }}));
}}
"""


LINKS_MODE = links_select(0)
LINKS_BROWSER = links_select(1)

# Every settings object the mock has written, as the sign-in and one witness from another card.
LINK_SAVES = """
() => (window.__ceMockSaves ?? []).map((s) => [
  s.link.cookies, s.link.cookie_browser, s.link.cookie_file, s.video.quality,
])
"""

# Status, message and the one button a blocked row grows — read per row, because the runtime section
# runs a YouTube link that fails beside a Bilibili link that does not.
BLOCKED_ROWS = """
() => [...document.querySelectorAll('.row')].map((r) => [
  r.dataset.status,
  r.querySelector('.row__error')?.textContent ?? null,
  r.querySelector('.row__fix')?.textContent ?? null,
])
"""


async def link_written(page: Page, link: list, timeout: float = 4000) -> bool:
    """Wait until the mock has *written* this sign-in, for the reason `trim_written` waits."""
    try:
        await page.wait_for_function(
            "(want) => { const saves = window.__ceMockSaves ?? [];"
            " const last = saves[saves.length - 1];"
            " return last !== undefined && last.link.cookies === want[0]"
            " && last.link.cookie_browser === want[1] && last.link.cookie_file === want[2]; }",
            arg=link,
            timeout=timeout,
        )
        return True
    except PWTimeout:
        return False


async def the_links_group(browser) -> None:
    """Three ways to answer "who is watching", and the two controls that answer for two of them."""
    page = await fresh(browser)
    await open_settings(page)
    group = await page.evaluate(LINKS_GROUP)
    check(
        "332 no sign-in is borrowed, and each control is inert until its own mode is chosen",
        group is not None
        and [f["label"] for f in group["fields"]]
        == ["Sign-in for pasted links", "Browser", "Cookies file"]
        and group["fields"][0]["value"] == "none"
        and [o for o, _ in group["fields"][0]["options"]] == ["none", "browser", "file"]
        and group["fields"][0]["inert"] is False
        and group["fields"][1]["inert"] is True
        and group["fields"][2]["inert"] is True
        and group["fields"][2]["path"] == "No file chosen"
        and all(f["hint"] is None for f in group["fields"]),
        group,
    )
    # A list rather than a text box is the whole of the safety here: the name lands next to
    # `--cookies-from-browser`, where `chrome:Profile 2` would select somebody else's profile.
    check(
        "333 the browser list is `settings::COOKIE_BROWSERS`, in its order and in its spelling",
        [o for o, _ in group["fields"][1]["options"]]
        == ["", "safari", "chrome", "chromium", "edge", "brave", "firefox", "vivaldi", "opera"]
        and [n for _, n in group["fields"][1]["options"]]
        == [
            "No browser chosen",
            "Safari",
            "Chrome",
            "Chromium",
            "Edge",
            "Brave",
            "Firefox",
            "Vivaldi",
            "Opera",
        ]
        and all(o == o.lower() for o, _ in group["fields"][1]["options"]),
        group["fields"][1]["options"],
    )

    await page.evaluate(LINKS_MODE, "browser")
    await page.wait_for_timeout(500)
    woken = await page.evaluate(LINKS_GROUP)
    check(
        "334 borrowing it from a browser wakes that select alone, and the file row stays inert",
        woken["fields"][1]["inert"] is False
        and woken["fields"][2]["inert"] is True
        and woken["fields"][2]["path"] == "No file chosen",
        [f["inert"] for f in woken["fields"]],
    )

    # The label is capitalised and the value is not, which is the difference that matters: only the
    # lowercase one is on the allowlist, and only the allowlist's own string ever reaches an argv.
    await page.evaluate(LINKS_BROWSER, "chrome")
    written = await link_written(page, ["browser", "chrome", None])
    check(
        "335 and what is sent is the allowlist's lowercase name, not the label beside it",
        written
        and (await page.evaluate(LINK_SAVES))[-1] == ["browser", "chrome", None, "balanced"],
        (await page.evaluate(LINK_SAVES))[-1],
    )

    # `commands::apply_preset` carries the sign-in across for the reason it carries the trim: a
    # click on "Smallest file" that signed the user out would turn the next members-only link back
    # into a failure they had already been to Settings to fix.
    await page.click('.preset input[value="smallest"]')
    await page.wait_for_selector('.preset[data-active] input[value="smallest"]')
    await page.wait_for_timeout(400)
    kept = await page.evaluate(LINKS_GROUP)
    check(
        "336 a preset is an opinion about quality, and it has none about whose sign-in is borrowed",
        kept["fields"][0]["value"] == "browser"
        and kept["fields"][1]["value"] == "chrome"
        and kept["fields"][1]["inert"] is False,
        [f["value"] for f in kept["fields"]],
    )

    # Switching mode does not clear the browser: it stops *reading* it, which is the same rule the
    # destination follows, and the reason a user can look at both answers without losing either.
    await page.evaluate(LINKS_MODE, "file")
    await page.wait_for_timeout(500)
    filed = await page.evaluate(LINKS_GROUP)
    check(
        "337 and reading it from a file wakes the Choose… button and puts the browser back to sleep",
        filed["fields"][1]["inert"] is True
        and filed["fields"][1]["value"] == "chrome"
        and filed["fields"][2]["inert"] is False
        and filed["fields"][2]["button"] == "Choose…",
        [f["inert"] for f in filed["fields"]],
    )
    check("338 the Links group produced no page errors", ERRORS[page] == [], ERRORS[page])
    await page.context.close()


async def a_sign_in_half_chosen(browser) -> None:
    """A mode chosen a keystroke before the thing it needs, which is not a mistake.

    `?cookies=browser` and `?cookies=file` start the session in the two states `settings_store`
    holds back in silence. Other settings still persist; the unfinished source remains visible
    beside the hint that explains which half is missing.
    """
    page = await fresh(browser, query="?cookies=browser")
    await open_settings(page)
    group = await page.evaluate(LINKS_GROUP)
    check(
        "339 a browser source with no browser behind it says which half is missing, beside the field",
        group["fields"][1]["hint"] == "Choose which browser to borrow the sign-in from."
        and group["fields"][0]["hint"] is None
        and group["fields"][2]["hint"] is None,
        [f["hint"] for f in group["fields"]],
    )
    check(
        "340 and says it as a hint: the quiet ink, nothing of the danger one, and no toast at all",
        group["fields"][1]["ink"] == group["quiet"]
        and group["fields"][1]["ink"] != group["danger"]
        and await page.evaluate(TOAST) is None
        and await page.query_selector(".toast") is None,
        [group["fields"][1]["ink"], group["quiet"], group["danger"]],
    )

    # The edit made straight afterwards is what the silence is for — the trim's paragraph again,
    # with a different half-made choice in front of it.
    before = await page.evaluate(LINK_SAVES)
    await page.evaluate(VIDEO_QUALITY, "high")
    await page.wait_for_timeout(900)
    held = await page.evaluate(LINK_SAVES)
    check(
        "341 quality is saved while the unfinished browser source is held",
        before == [] and held == [["none", "", None, "high"]]
        and await page.evaluate(TOAST) is None,
        [held, await page.evaluate(TOAST)],
    )
    await page.evaluate(LINKS_BROWSER, "firefox")
    landed = await link_written(page, ["browser", "firefox", None])
    check(
        "342 naming the browser saves the source without losing the quality edit",
        landed
        and (await page.evaluate(LINK_SAVES))[-1] == ["browser", "firefox", None, "high"],
        (await page.evaluate(LINK_SAVES))[-1],
    )
    errors = list(ERRORS[page])
    await page.context.close()

    page = await fresh(browser, query="?cookies=file")
    await open_settings(page)
    filed = await page.evaluate(LINKS_GROUP)
    await page.evaluate(VIDEO_QUALITY, "high")
    await page.wait_for_timeout(900)
    check(
        "343 a file source with no file yet is the same silence, in the sentence naming both ways out",
        filed["fields"][2]["hint"] == "Choose the cookies.txt file to read the sign-in from, or "
        "take it from a browser instead."
        and filed["fields"][2]["ink"] == filed["quiet"]
        and filed["fields"][2]["path"] == "No file chosen"
        and await page.evaluate(TOAST) is None
        and await page.evaluate(LINK_SAVES) == [["none", "", None, "high"]],
        [filed["fields"][2]["hint"], await page.evaluate(LINK_SAVES)],
    )
    check(
        "344 the unfinished sign-in produced no page errors",
        errors == [] and ERRORS[page] == [],
        [errors, ERRORS[page]],
    )
    await page.context.close()


# The four values the backend really refuses, each with the sentence it refuses them in. A picker
# cannot produce any of them, so `?cookies=` is the only way to look at these at all — and the knob
# passes them through untouched rather than correcting them, because a preview that quietly fixed
# `netscape` would hide the refusal it exists to show. The save that carries one is answered by
# holding `LinkSettings::default()`, which is what "the last usable sign-in" means with nothing
# usable behind it.
REFUSED_SIGN_INS: list[tuple[str, str, str]] = [
    (
        "?cookies=netscape",
        "345 a browser the app will not name is refused, with the eight it will spelled out",
        "Flint cannot take a sign-in from that browser. Choose one of: safari, "
        "chrome, chromium, edge, brave, firefox, vivaldi, opera.",
    ),
    (
        "?cookies=file:cookies.txt",
        "346 a path that is not a full path is refused before it can mean anything",
        "The cookies file must be a full path.",
    ),
    (
        "?cookies=file:/tmp/missing-cookies.txt",
        "347 a cookies.txt that is not there is refused, and named so it can be found again",
        "The cookies file /tmp/missing-cookies.txt is not there. Export cookies.txt from your "
        "browser again and choose it, or take the sign-in from a browser instead.",
    ),
    (
        "?cookies=file:/Users/you/Desktop",
        "348 and a folder is refused for being one, in the sentence that says which",
        "/Users/you/Desktop is a folder, not a cookies.txt file. Choose the exported file itself.",
    ),
]


async def a_sign_in_the_backend_refuses(browser) -> None:
    """The other arm: a value the user really chose, and the refusal that is theirs to read.

    Each one is a fresh window because the state is a knob, and each is provoked by editing an
    unrelated setting — the save is the moment the backend gets to answer, and its answer has to
    arrive in `settings_store`'s own words rather than in a sentence this app wrote a second copy of.
    """
    errors: list = []
    for query, name, sentence in REFUSED_SIGN_INS:
        page = await fresh(browser, query=query)
        await open_settings(page)
        await page.evaluate(VIDEO_QUALITY, "high")
        await page.wait_for_selector(".toast")
        check(
            name,
            await page.evaluate(TOAST) == sentence
            # ...and the sign-in that was stored is the last usable one, which here is none at all.
            and (await page.evaluate(LINK_SAVES))[-1][:3] == ["none", "", None],
            [await page.evaluate(TOAST), (await page.evaluate(LINK_SAVES))[-1]],
        )
        errors += ERRORS[page]
        await page.context.close()
    check("349 the refused sign-ins produced no page errors", errors == [], errors)


async def a_link_with_no_javascript_runtime(browser) -> None:
    """The failure that used to read as a sign-in wall, and the helper it really wants.

    `?linkfail=jsruntime` is the Mac with no runtime on it, which cannot be reached honestly in a
    browser tab. The Bilibili link beside it is the control: YouTube's challenges are YouTube's, so
    it converts, and the question the settled batch asks is about the one row that stopped.
    """
    page = await fresh(browser, query="?linkfail=jsruntime&linkms=20")
    await open_links(page)
    await page.fill(".links__box", f"{YOUTUBE}\n{BILIBILI}")
    await judged(page, 2)
    await page.click(".links .promptbutton")
    await page.wait_for_function("() => document.querySelectorAll('.row').length === 2")
    await page.click(".pill")
    await page.wait_for_selector(".prompt", timeout=60000)
    await page.wait_for_timeout(300)
    rows = await page.evaluate(BLOCKED_ROWS)
    check(
        "350 the row names the runtime YouTube wanted, and never mentions a sign-in",
        rows[0][0] == "failed"
        and rows[0][1] == "YouTube needs a JavaScript runtime to hand this video over, and "
        "Flint could not use one. Open Settings → Helpers and install Deno (one click), "
        "then try again."
        and "sign in" not in rows[0][1].lower(),
        rows[0],
    )
    check(
        "351 and its own way out is the helper the app can install, which is Deno and not yt-dlp",
        rows[0][2] == "Install Deno" and rows[1][0] == "done" and rows[1][2] is None,
        rows,
    )
    body = await page.text_content(".prompt__body")
    check(
        "352 the settled batch asks about Deno once, for the one row a runtime would have saved",
        "A pasted link needs Deno" in (body or "")
        and (await page.text_content(".promptbutton") or "").startswith("Install Deno"),
        body,
    )
    await page.keyboard.press("Enter")
    await page.wait_for_selector(".tool[data-highlight]")
    await page.wait_for_timeout(600)
    check(
        "353 Yes lands on Deno's Install button in Settings without pressing it",
        await page.query_selector(".prompt") is None
        and await page.get_attribute(tool("deno"), "data-highlight") is not None
        and await page.evaluate("() => document.activeElement?.getAttribute('aria-label')")
        == "Install Deno"
        and await page.query_selector(".install") is None
        and await page.text_content(f"{tool('deno')} .tool__state") == "Missing",
        await page.text_content(f"{tool('deno')} .tool__state"),
    )
    # The other runtime, which is a row and not a purchase: the app uses a Node that is already
    # there and installs a JavaScript toolchain for nobody, so there is nothing here to click.
    node = await page.evaluate(
        "() => { const t = document.querySelector('.tool[data-tool=\"node\"]');"
        " return t === null ? null : { label: t.querySelector('.tool__label').textContent,"
        " state: t.querySelector('.tool__state').textContent,"
        " installable: t.dataset.installable ?? null,"
        " button: t.querySelector('.toolbutton')?.textContent ?? null,"
        " plain: t.querySelector('.tool__plain')?.textContent ?? null }; }"
    )
    check(
        "354 Node is a row and not a purchase: no Install button, and it says what is installed instead",
        node is not None
        and node["label"] == "Node.js"
        and node["state"] == "Missing"
        and node["installable"] is None
        and node["button"] is None
        and node["plain"] == "Flint installs Deno instead (brew install deno).",
        node,
    )
    check("355 the missing runtime produced no page errors", ERRORS[page] == [], ERRORS[page])
    await page.context.close()


async def safari_needs_a_permission(browser) -> None:
    """The one failure with nothing to install and nothing to change here: a permission.

    macOS keeps Safari's cookie jar behind Full Disk Access, so `--cookies-from-browser safari`
    comes back `Operation not permitted` however well-formed the request. `?linkfail=safari` is that
    machine, since a browser tab has no permissions to be short of.
    """
    page = await fresh(browser, query="?linkfail=safari&cookies=safari&linkms=20")
    await open_links(page)
    await page.fill(".links__box", YOUTUBE)
    await judged(page, 1)
    await page.click(".links .promptbutton")
    await page.wait_for_function("() => document.querySelectorAll('.row').length === 1")
    await convert_and_settle(page)
    rows = await page.evaluate(BLOCKED_ROWS)
    check(
        "356 the row names the permission, both ways past it, and offers the trip to the pane",
        rows[0][0] == "failed"
        and "Full Disk Access" in (rows[0][1] or "")
        and "Settings → Links" in (rows[0][1] or "")
        and rows[0][2] == "Open Full Disk Access…",
        rows[0],
    )
    # Nothing to install: `missingHelperFor` is not asked, and no question is raised about a helper
    # that would not have helped. The trip is the only affordance, and it is a zero-argument command.
    await page.click(".row__fix")
    await page.wait_for_timeout(500)
    check(
        "357 pressing it says nothing back, and leaves the row exactly as it was to retry",
        await page.query_selector(".prompt") is None
        and await page.evaluate(TOAST) is None
        and await page.evaluate(BLOCKED_ROWS) == rows,
        [await page.evaluate(TOAST), await page.evaluate(BLOCKED_ROWS)],
    )
    await open_settings(page)
    group = await page.evaluate(LINKS_GROUP)
    check(
        "358 and the easier route the message names is a real control, standing on Safari",
        group["fields"][0]["value"] == "browser"
        and group["fields"][1]["value"] == "safari"
        and group["fields"][1]["inert"] is False
        and [o for o, _ in group["fields"][1]["options"]].count("chrome") == 1,
        [f["value"] for f in group["fields"]],
    )
    check("359 the Safari permission produced no page errors", ERRORS[page] == [], ERRORS[page])
    await page.context.close()


# What the three rewritten endings say, and the affordance each one names. The bug behind them was
# not the wording: `install.rs` scanned the disk *before* running the installer, so every fresh
# install was judged against a machine that did not have it yet and came back "the installer
# finished, but we still cannot find it — run `brew install …` in Terminal to see what it did". The
# user who did that was told the formula was already installed, and had learned nothing. The probe
# is now a factory invoked after the installer exits, and no message sends anybody to Terminal to
# audit work this app did: "Re-check" is the app looking again, and the log is already on screen.
INSTALL_ENDINGS: list[tuple[str, str, str, str]] = [
    (
        "?missing=pandoc&installmissing=pandoc&installms=30",
        "pandoc",
        "360 exit 0 with the helper still nowhere asks the app to look again, not the user to go and look",
        "The installer reported success, but Pandoc is not in any of the places Flint "
        "looks. Use Re-check to look again.",
    ),
    (
        "?missing=pdftoppm,pdftotext,pdftohtml&installpartial=poppler&installms=30",
        "poppler",
        "362 a package only half of which landed counts what it found and offers the same two steps",
        "The installer finished, but Flint can only find part of Poppler (1 of its 3 "
        "programs), so some conversions still will not run. Use Re-check, then install Poppler "
        "again if it is still incomplete.",
    ),
    (
        "?missing=pandoc&installfail=pandoc&installms=30",
        "pandoc",
        "363 and a real failure still names Terminal — to install it by hand, never to check up on us",
        "Could not install Pandoc (the installer exited with 1). Last message: Error: No available "
        'formula with the name "pandoc" Show log for everything it wrote, or run `brew install '
        "pandoc` in Terminal to install it by hand.",
    ),
]


async def the_installer_says_what_it_can_do_itself(browser) -> None:
    """The three endings that used to set homework, and the buttons they name instead."""
    errors: list = []
    for query, package, name, message in INSTALL_ENDINGS:
        page = await fresh(browser, query=query)
        await open_settings(page)
        await page.click(f"{tool(package)} .toolbutton")
        await page.wait_for_selector(".install:not([data-status='running'])", timeout=60000)
        await page.wait_for_timeout(300)
        check(name, await page.text_content(".install__state") == message,
              await page.text_content(".install__state"))
        # Every affordance those sentences name has to be on screen beside them, or the message is
        # setting homework again — this time in the app's own vocabulary, which is worse.
        recheck = await page.evaluate(
            "() => { const b = document.querySelector('.tools__header .microlink');"
            " return b === null ? null : [b.textContent, b.disabled]; }"
        )
        log = await page.evaluate(
            "() => { const b = [...document.querySelectorAll('.install__meta .microlink')]"
            ".find((x) => x.textContent.endsWith(' log'));"
            " return b === null || b === undefined ? null"
            " : [b.textContent, document.querySelector('.install__log') !== null]; }"
        )
        errors += ERRORS[page]
        if package == "pandoc" and "installmissing" in query:
            check(
                "361 and Re-check is a live button in the Helper apps header, where the message says",
                recheck == ["Re-check", False] and log == ["Hide log", True],
                [recheck, log],
            )
        await page.context.close()
    check("364 the installer's endings produced no page errors", errors == [], errors)





SIGN_IN_SHEET = """
() => {
  const card = document.querySelector('.signin');
  if (card === null) return null;
  const list = card.querySelector('.signin__steps');
  return {
    role: card.getAttribute('role'),
    modal: card.getAttribute('aria-modal'),
    title: document.getElementById(card.getAttribute('aria-labelledby') ?? '')?.textContent ?? null,
    note: document.getElementById(card.getAttribute('aria-describedby') ?? '')?.textContent ?? null,
    tag: list === null ? null : list.tagName,
    // The sentence and, separately, the one thing the app can do about it from here: a step with two
    // buttons under it would be a step that is really two steps.
    steps: [...card.querySelectorAll('.signin__step')].map((li) => [
      li.childNodes[0].textContent,
      [...li.querySelectorAll('button')].map((b) => b.textContent),
    ]),
    // Drawn by a CSS counter rather than by the list marker, so the number is in the gutter and the
    // sentence keeps its own left edge. Read here because "numbered" is half of "these are steps".
    counter: getComputedStyle(card.querySelector('.signin__step'), '::before').content,
    why: card.querySelector('.signin__why')?.textContent ?? null,
    source: card.querySelector('.signin__source')?.textContent ?? null,
    actions: [...card.querySelectorAll('.signin__actions button')].map((b) => b.textContent),
    cards: document.querySelectorAll('.signin').length,
    veils: document.querySelectorAll('.promptveil').length,
    focus: document.activeElement?.textContent ?? null,
  };
}
"""

# The verdict where it was asked for, with the two tokens it is allowed to be painted in. Colour is
# read because "it works" and "the site said no" must not arrive in the same ink.
#
# The sheet's copy first and Settings → Links' second, because both exist at once: the settings sheet
# is in the DOM whether it is open or not, and the answer a user is looking at is the one on the card
# in front of them.
VERDICT_LINE = (
    "(document.querySelector('.signin .signin__verdict')"
    " ?? document.querySelector('.drawer .signin__verdict'))"
)

VERDICT = f"""
() => {{
  const line = {VERDICT_LINE};
  if (line === null) return null;
  const resolve = (name) => {{
    const probe = document.createElement('span');
    probe.style.color = `var(${{name}})`;
    document.body.appendChild(probe);
    const colour = getComputedStyle(probe).color;
    probe.remove();
    return colour;
  }};
  return {{
    result: line.dataset.result ?? null,
    text: line.textContent,
    role: line.getAttribute('role'),
    ink: getComputedStyle(line).color,
    accent: resolve('--accent'),
    danger: resolve('--danger'),
  }};
}}
"""

# The button that runs it and the way to the walkthrough, as Settings → Links offers them.
SIGN_IN_CHECK_CONTROLS = """
() => [...document.querySelectorAll('.signincheck button')].map((b) => [
  b.className, b.textContent, b.disabled,
])
"""

# The browser picker as a menu rather than as a flat list of values: which names are offered without
# comment, and which are under a heading that says this Mac does not have them.
BROWSER_MENU = """
() => {
  const section = [...document.querySelectorAll('.section')]
    .find((s) => s.querySelector('.section__title').textContent === 'Links');
  if (section === undefined) return null;
  section.open = true;
  const select = section.querySelectorAll('select')[1];
  return {
    here: [...select.children].filter((c) => c.tagName === 'OPTION').map((o) => o.value),
    groups: [...select.children]
      .filter((c) => c.tagName === 'OPTGROUP')
      .map((g) => [g.label, [...g.children].map((o) => o.value)]),
    all: [...select.options].map((o) => [o.value, o.textContent, o.disabled]),
  };
}
"""

# Every status any row has held since this was installed, in order and de-duplicated. The one way to
# ask "was that row run again?" after the fact: a retry the user did not ask for is invisible by the
# time the batch has settled, because it ends in the same `done` the row already had.
WATCH_STATUSES = """
() => {
  const read = () => [...document.querySelectorAll('.row')].map((r) => r.dataset.status).join(',');
  window.__ceSeen = [read()];
  new MutationObserver(() => window.__ceSeen.push(read())).observe(document.body, {
    subtree: true,
    attributes: true,
    attributeFilter: ['data-status'],
  });
}
"""

SEEN_STATUSES = "() => [...new Set(window.__ceSeen ?? [])].map((s) => s.split(','))"

# `link::FetchFailure`, message for message — the three endings the frontend tells apart, and which
# it must never fold together again. A row's whole affordance hangs off the clause in the middle of
# each of these, so they are quoted here in full rather than matched on a substring.
WALL = (
    "The site wants a sign-in before it will hand this video over. Open Settings → Links and let "
    "Flint borrow the sign-in from your browser, or point it at a cookies.txt file you "
    "exported yourself."
)
NOT_ACCEPTED = (
    "The site would not accept the sign-in Flint borrowed. Sign in to the site again "
    "in that browser and retry, or export a fresh cookies.txt - the cookies it was given have most "
    "likely expired, or belong to an account without access to this video."
)
UNREADABLE = (
    "Flint could not read the sign-in from that browser, so the site was asked for "
    "this video with no sign-in at all. Check that the browser is installed and that you are "
    "signed in to the site in it, or export a cookies.txt file and point Settings → Links at that "
    "instead."
)
WORKS = "That sign-in works: Flint read it and the site handed the test video over."
TIMED_OUT = (
    "The sign-in check was still running after 20 seconds and was stopped, so it proved nothing "
    "either way. Check your internet connection and try again."
)
NOTHING_TO_CHECK = (
    "There is no sign-in to check yet. Choose a browser to borrow the sign-in from, or a "
    "cookies.txt file you exported, and then check it."
)

# Eleven windows, one walk. Their page errors are collected rather than asserted a function at a
# time, because a listener left behind by any of them is the same bug wherever it was thrown.
SIGN_IN_ERRORS: list = []


async def gated_links(page: Page, links: list[str]) -> None:
    """Paste these links, queue them and run them — the four gestures every section below starts on."""
    await open_links(page)
    await page.fill(".links__box", "\n".join(links))
    await judged(page, len(links))
    await page.click(".links .promptbutton")
    await page.wait_for_function(
        "(n) => document.querySelectorAll('.row').length === n", arg=len(links)
    )
    await page.click(".pill")


async def walkthrough_from_settings(browser, query: str) -> Page:
    """A window opened straight onto the walkthrough, with no batch in front of it.

    Settings → Links is the always-available route to the same sheet, so it is how the sections that
    are about the sheet's *contents* get to it: a fetch, a failure and a settled batch would be
    three seconds of theatre before an assertion about a list of sentences.
    """
    page = await fresh(browser, query=query)
    await open_settings(page)
    # The group is a collapsed `<details>`; reading the menu opens it, exactly as `LINKS_GROUP` does.
    await page.evaluate(BROWSER_MENU)
    await page.click(".signincheck .microlink")
    await page.wait_for_selector(".signin")
    await page.wait_for_timeout(400)
    return page


async def a_link_stopped_by_a_sign_in_wall(browser) -> None:
    """The red dead end, and the two things standing where it used to end.

    `?linkfail=signin` is a Mac whose YouTube fetches come back needing a sign-in — and, unlike every
    other failure knob in this file, one that *stops* the moment a readable sign-in is configured,
    because a recovery nothing can recover from proves only that the buttons exist. The Bilibili link
    beside the two YouTube ones is the control: it is not gated, so it converts, and everything said
    below is said about the rows that actually stopped.
    """
    page = await fresh(browser, query="?linkfail=signin&linkms=20")
    await gated_links(page, [YOUTUBE, YOUTUBE_SHORT, BILIBILI])
    await page.wait_for_selector(".prompt", timeout=60000)
    await page.wait_for_timeout(300)
    rows = await page.evaluate(BLOCKED_ROWS)
    check(
        "365 a link the site would not hand over says so, and now carries a verb as well",
        [r[0] for r in rows] == ["failed", "failed", "done"]
        and rows[0][1] == WALL
        and rows[0][2] == "Set up a sign-in…"
        and rows[1][2] == "Set up a sign-in…"
        # The link nothing gated keeps the row it always had: no failure, and nothing to press.
        and rows[2][1] is None
        and rows[2][2] is None,
        rows,
    )

    semantics = await page.evaluate(PROMPT_SEMANTICS)
    check(
        "366 the settled batch asks one question, and names the browser this Mac actually has",
        semantics is not None
        and semantics["cards"] == 1
        and semantics["label"] == "2 links need a sign-in. Use your Chrome sign-in and try again?"
        # Two rows, one question — and the count is of links, because a link is what a queue holds
        # before anything has been fetched into a file.
        and semantics["native"] == 0,
        semantics,
    )
    check(
        "367 and it promises only what one click can deliver, with the longer way round beside it",
        (semantics["body"] or "").startswith("Chrome — you used it recently.")
        and "borrows the sign-in it already has" in (semantics["body"] or "")
        and "checks it against a public video" in (semantics["body"] or "")
        and "no password is asked for" in (semantics["body"] or "")
        and await page.evaluate(
            "() => [...document.querySelectorAll('.prompt__actions button')].map((b) => b.textContent)"
        )
        == ["Not now", "Walk me through it…", "Use Chrome"],
        semantics["body"],
    )

    # Declining is not a dead end either: the two rows keep the action they grew, and it opens the
    # same sheet the question's middle button would have.
    await page.keyboard.press("Escape")
    await page.wait_for_selector(".prompt", state="detached")
    await page.click(".row__fix")
    await page.wait_for_selector(".signin")
    await page.wait_for_timeout(400)
    sheet = await page.evaluate(SIGN_IN_SHEET)
    check(
        "368 declining leaves the row's own way out, which opens the walkthrough over the window",
        sheet is not None
        and sheet["role"] == "dialog"
        and sheet["modal"] == "true"
        and sheet["title"] == "Let a link use your sign-in"
        and sheet["cards"] == 1
        and sheet["veils"] == 1
        and sheet["focus"] == "Check sign-in",
        sheet,
    )
    await page.keyboard.press("Escape")
    await page.wait_for_selector(".signin", state="detached")

    # Told once is enough. The rows still carry their action and Settings → Links is still there, so
    # a second card for the same wall would be nagging rather than helping.
    await convert_and_settle(page)
    check(
        "369 and the next batch behind the same wall is not asked about a second time",
        await page.query_selector(".prompt") is None
        and [r[0] for r in await page.evaluate(BLOCKED_ROWS)] == ["failed", "failed", "done"],
        await page.evaluate(BLOCKED_ROWS),
    )
    SIGN_IN_ERRORS.extend(ERRORS[page])
    await page.context.close()


async def the_question_does_the_rest(browser) -> None:
    """"Yes" is the whole repair, which is the one way this question differs from the helper's.

    Installing costs a download and a password, so that question hands over and stops. Borrowing a
    sign-in costs a saved setting and a two-second probe, so this one saves it, proves it, and runs
    the rows the wall stopped — with no further click anywhere in that sentence.
    """
    page = await fresh(browser, query="?linkfail=signin&linkms=20")
    await gated_links(page, [YOUTUBE, YOUTUBE_SHORT, BILIBILI])
    await page.wait_for_selector(".prompt", timeout=60000)
    await page.wait_for_timeout(300)
    await page.evaluate(WATCH_STATUSES)
    await page.click(".prompt .promptbutton")
    await page.wait_for_timeout(800)
    check(
        "370 Yes writes the sign-in the question named, once, in the allowlist's own spelling",
        await link_written(page, ["browser", "chrome", None])
        and await page.evaluate(LINK_SAVES) == [["browser", "chrome", None, "balanced"]],
        await page.evaluate(LINK_SAVES),
    )

    converted = False
    try:
        await page.wait_for_function(
            "() => [...document.querySelectorAll('.row')].every((r) => r.dataset.status === 'done')",
            timeout=60000,
        )
        converted = True
    except PWTimeout:
        pass
    await page.wait_for_timeout(400)
    check(
        "371 and the rows the wall stopped come back converted, with no click in between",
        converted and [r[0] for r in await page.evaluate(BLOCKED_ROWS)] == ["done"] * 3,
        await page.evaluate(BLOCKED_ROWS),
    )
    # `retryRows` is handed the ids the question was raised about, not a predicate over the queue:
    # re-running a link that had already converted would download it a second time to produce a file
    # that was already on disk.
    seen = await page.evaluate(SEEN_STATUSES)
    check(
        "372 while the link that had already converted is never run again",
        all(row[2] == "done" for row in seen),
        seen,
    )
    check(
        "373 and nothing is left standing behind it: no question, no sheet, and nothing to dismiss",
        await page.query_selector(".prompt") is None
        and await page.query_selector(".signin") is None
        and await page.evaluate(TOAST) is None,
        await page.evaluate(TOAST),
    )
    SIGN_IN_ERRORS.extend(ERRORS[page])
    await page.context.close()


async def a_sign_in_the_check_disproves(browser) -> None:
    """The other half of "checks it first": what happens when the check says no.

    `?signin=refused` is the sign-in that reads perfectly and is refused by the site anyway — an
    account that has expired, or one without access to the video. Nothing about that is retryable,
    and a retry would spend the fetch to reprint the failure the row already has.
    """
    page = await fresh(browser, query="?linkfail=signin&signin=refused&linkms=20")
    await gated_links(page, [YOUTUBE, YOUTUBE_SHORT])
    await page.wait_for_selector(".prompt", timeout=60000)
    await page.wait_for_timeout(300)
    before = await page.evaluate(BLOCKED_ROWS)
    await page.click(".prompt .promptbutton")
    await page.wait_for_selector(".signin")
    await page.wait_for_timeout(1200)
    check(
        "374 a check that says no retries nothing, and the rows keep the verdicts they had",
        await page.evaluate(BLOCKED_ROWS) == before
        and [r[0] for r in before] == ["failed", "failed"]
        and (await page.text_content(".pill") or "").startswith("Convert"),
        [before, await page.text_content(".pill")],
    )
    verdict = await page.evaluate(VERDICT)
    check(
        "375 and the walkthrough opens with that verdict standing in it, in the site's own words",
        verdict is not None
        and verdict["result"] == "refused"
        and verdict["text"] == NOT_ACCEPTED
        and verdict["role"] == "status"
        and verdict["ink"] == verdict["danger"],
        verdict,
    )
    SIGN_IN_ERRORS.extend(ERRORS[page])
    await page.context.close()


async def a_sign_in_that_is_already_the_one_failing(browser) -> None:
    """The offer the question is not allowed to make: the browser it is already reading.

    `?linkfail=refused` with Chrome configured is the jar that opened, the cookies that went out and
    the site that said no regardless. "Use your Chrome sign-in and try again?" would be a button that
    repeats what has just failed, so the question becomes the walkthrough instead — where a fresh
    sign-in, another browser and the cookies.txt route are each said out loud.
    """
    page = await fresh(browser, query="?linkfail=refused&cookies=chrome&linkms=20")
    await gated_links(page, [YOUTUBE])
    await page.wait_for_selector(".prompt", timeout=60000)
    await page.wait_for_timeout(300)
    rows = await page.evaluate(BLOCKED_ROWS)
    check(
        "376 a sign-in that was read and refused is a thing to fix, not a thing to set up",
        rows[0][0] == "failed" and rows[0][1] == NOT_ACCEPTED and rows[0][2] == "Fix the sign-in…",
        rows[0],
    )
    semantics = await page.evaluate(PROMPT_SEMANTICS)
    labels = await page.evaluate(
        "() => [...document.querySelectorAll('.prompt__actions button')].map((b) => b.textContent)"
    )
    check(
        "377 and the question does not offer the browser it is already using, but the walk instead",
        semantics["label"]
        == "1 link needs a sign-in. The sign-in Flint already has did not work — see "
        "what to try?"
        and "Use Chrome" not in labels
        and labels == ["Not now", "Show me how…"],
        [semantics["label"], labels],
    )
    SIGN_IN_ERRORS.extend(ERRORS[page])
    await page.context.close()


async def two_macs_where_one_click_would_be_a_lie(browser) -> None:
    """The two machines the measured offer is measured for.

    `?browsers=` is `list_cookie_browsers` on a Mac that is not the one this flow was designed on:
    Safari and nothing else, where the cookie jar is behind Full Disk Access and no click can grant
    it; and no allowlisted browser at all, where the only route left is a file the user exports.
    Both get a question, and neither gets one that promises a fix it cannot perform.
    """
    page = await fresh(browser, query="?linkfail=signin&browsers=safari&linkms=20")
    await gated_links(page, [YOUTUBE, YOUTUBE_SHORT])
    await page.wait_for_selector(".prompt", timeout=60000)
    await page.wait_for_timeout(300)
    semantics = await page.evaluate(PROMPT_SEMANTICS)
    labels = await page.evaluate(
        "() => [...document.querySelectorAll('.prompt__actions button')].map((b) => b.textContent)"
    )
    check(
        "378 Safari alone is offered the permission, not a click that cannot reach its cookies",
        semantics["label"] == "2 links need a sign-in. It is most likely in Safari — it is your "
        "default browser — and macOS keeps those cookies behind Full Disk Access."
        and labels == ["Not now", "Open Full Disk Access…"]
        and "try again" not in semantics["label"]
        and "Privacy & Security" in (semantics["body"] or "")
        # The relaunch is in the promise as well as in the walk: a user who grants the permission
        # and comes straight back is the user this whole pass is about.
        and "quit Flint and open it again" in (semantics["body"] or ""),
        [semantics["label"], labels],
    )
    await page.click(".prompt .promptbutton")
    await page.wait_for_selector(".signin")
    await page.wait_for_timeout(700)
    check(
        "379 accepting it makes Safari the source, asks for the pane, and stands the walk beside it",
        await link_written(page, ["browser", "safari", None])
        and await page.query_selector(".signin") is not None
        # No probe was run: the only thing it could report is the failure the user is on their way
        # to fix, and the rows are left exactly as they were for when they come back.
        and await page.evaluate(VERDICT) is None
        and [r[0] for r in await page.evaluate(BLOCKED_ROWS)] == ["failed", "failed"]
        and await page.evaluate(TOAST) is None,
        [await page.evaluate(LINK_SAVES), await page.evaluate(VERDICT)],
    )
    SIGN_IN_ERRORS.extend(ERRORS[page])
    await page.context.close()

    page = await fresh(browser, query="?linkfail=signin&browsers=&linkms=20")
    await gated_links(page, [YOUTUBE, YOUTUBE_SHORT])
    await page.wait_for_selector(".prompt", timeout=60000)
    await page.wait_for_timeout(300)
    semantics = await page.evaluate(PROMPT_SEMANTICS)
    await page.click(".prompt .promptbutton")
    await page.wait_for_selector(".signin")
    await page.wait_for_timeout(700)
    check(
        "380 a Mac with none of the eight is offered the file it can export, and nothing is written",
        semantics["label"] == "2 links need a sign-in. No browser here can lend one, so the way in "
        "is an exported cookies.txt file."
        and "browser extension" in (semantics["body"] or "")
        and await page.query_selector(".signin") is not None
        # "From a file" with no file yet is a half-made choice, so there is nothing to save until
        # the user has chosen one — see 343.
        and await page.evaluate(LINK_SAVES) == [],
        [semantics["label"], await page.evaluate(LINK_SAVES)],
    )
    SIGN_IN_ERRORS.extend(ERRORS[page])
    await page.context.close()


async def the_walkthrough_itself(browser) -> None:
    """Five sentences, in the order a person does them, and only the ones this Mac can act on.

    The sheet is the part of this that is *only* words, which is why the assertions here are about
    which words are present: a step about a keychain prompt Safari never shows, or about a permission
    for a browser the user does not own, is a step that sends somebody looking for a dialog that does
    not exist.
    """
    page = await walkthrough_from_settings(browser, "?missing=")
    sheet = await page.evaluate(SIGN_IN_SHEET)
    steps = sheet["steps"]
    check(
        "381 the sheet is a numbered walk with at most one button per step, and says what it is on",
        sheet["tag"] == "OL"
        and "counter" in (sheet["counter"] or "")
        and [s[1] for s in steps]
        == [[], ["Use Chrome"], [], ["Open Full Disk Access…"], [], ["Choose cookies.txt…"]]
        and steps[0][0].startswith("Sign in to the site in Chrome first")
        and "never sees your password" in steps[0][0]
        and sheet["source"] == "No sign-in configured yet."
        and sheet["actions"] == ["Done", "Check sign-in"],
        [[s[1] for s in steps], sheet["source"]],
    )
    check(
        "382 including the keychain macOS puts in front of a Chromium browser's cookies",
        "keychain" in steps[2][0]
        and "Allow" in steps[2][0]
        # A refused prompt hands the cookies back encrypted, which the site reads as no sign-in at
        # all — the failure this step exists to keep somebody out of.
        and "encrypted" in steps[2][0],
        steps[2][0],
    )
    SIGN_IN_ERRORS.extend(ERRORS[page])
    await page.context.close()

    page = await walkthrough_from_settings(browser, "?browsers=safari")
    safari = (await page.evaluate(SIGN_IN_SHEET))["steps"]
    check(
        "383 and never in front of Safari's, which are behind a permission and no keychain at all",
        [s[1] for s in safari]
        == [[], ["Use Safari"], ["Open Full Disk Access…"], [], ["Choose cookies.txt…"]]
        and not any("keychain" in text for text, _ in safari)
        and safari[0][0].startswith("Sign in to the site in Safari first"),
        [s[0][:48] for s in safari],
    )
    SIGN_IN_ERRORS.extend(ERRORS[page])
    await page.context.close()

    page = await walkthrough_from_settings(browser, "?browsers=chrome")
    chrome = (await page.evaluate(SIGN_IN_SHEET))["steps"]
    check(
        "384 the Full Disk Access step stands only on a Mac that has the browser it is about",
        [s[1] for s in chrome] == [[], ["Use Chrome"], [], ["Choose cookies.txt…"]]
        and not any("Full Disk Access" in text for text, _ in chrome),
        [s[1] for s in chrome],
    )
    SIGN_IN_ERRORS.extend(ERRORS[page])
    await page.context.close()

    page = await walkthrough_from_settings(browser, "?browsers=")
    none = await page.evaluate(SIGN_IN_SHEET)
    check(
        "385 and with nothing to borrow from, the walk is about the file and offers no browser",
        [s[1] for s in none["steps"]] == [[], ["Choose cookies.txt…"]]
        and none["steps"][0][0].startswith("Flint found none of the browsers")
        and not any(b.startswith("Use ") for _, buttons in none["steps"] for b in buttons),
        [s[1] for s in none["steps"]],
    )
    SIGN_IN_ERRORS.extend(ERRORS[page])
    await page.context.close()


# The five `CookieCheck` verdicts, each in the sentence `test_cookie_source` gives it, and the knob
# that produces it. `?signin=` forces the answer the way `?linkfail=` forces a failure: the mock has
# no yt-dlp and no network, and a check that could only ever say `working` would be a check nobody
# had watched fail.
VERDICTS: list[tuple[str, str, str]] = [
    ("?signin=working&cookies=chrome", "working", WORKS),
    ("?signin=unreadable&cookies=chrome", "unreadable", UNREADABLE),
    ("?signin=refused&cookies=chrome", "refused", NOT_ACCEPTED),
    ("?signin=inconclusive&cookies=chrome", "inconclusive", TIMED_OUT),
    ("?cookies=none", "not_configured", NOTHING_TO_CHECK),
]


async def checking_a_sign_in_without_converting_anything(browser) -> None:
    """The button that answers the question the red row could not: did what I just change help?"""
    said: list = []
    working = None
    for query, result, sentence in VERDICTS:
        page = await fresh(browser, query=query)
        await open_settings(page)
        await page.evaluate(BROWSER_MENU)
        await page.click(".signincheck .button")
        await page.wait_for_function(
            f"() => {VERDICT_LINE}?.dataset.result !== 'pending'",
            timeout=8000,
        )
        verdict = await page.evaluate(VERDICT)
        said.append([verdict["result"], verdict["text"], verdict["role"]])
        if result == "working":
            working = verdict
        SIGN_IN_ERRORS.extend(ERRORS[page])
        await page.context.close()
    check(
        "386 every verdict is said where the check was asked for, in that failure's own sentence",
        said == [[result, sentence, "status"] for _, result, sentence in VERDICTS],
        said,
    )
    check(
        "387 and the one a working sign-in gets says what was proved, in ink the other two are not",
        working is not None
        and working["text"] == WORKS
        and working["ink"] == working["accent"]
        and working["ink"] != working["danger"],
        working,
    )

    # Two seconds of yt-dlp is long enough to wonder whether the press registered, and long enough
    # for a second press to start a second probe behind the first.
    page = await fresh(browser, query="?cookies=chrome")
    await open_settings(page)
    await page.evaluate(BROWSER_MENU)
    # Hold the backend response until the pending state has been observed. Wall-clock
    # sleeps can miss it when the runner is busy compiling native artifacts.
    await page.clock.install()
    await page.clock.pause_at(await page.evaluate("Date.now()"))
    await page.click(".signincheck .button")
    running = await page.evaluate(VERDICT)
    check(
        "388 the wait is said out loud, and the button cannot be pressed again while it runs",
        running is not None
        and running["result"] == "pending"
        and running["text"] == "Checking that sign-in…"
        and (await page.evaluate(SIGN_IN_CHECK_CONTROLS))[0] == ["button", "Checking…", True],
        [running, await page.evaluate(SIGN_IN_CHECK_CONTROLS)],
    )
    SIGN_IN_ERRORS.extend(ERRORS[page])
    await page.context.close()

    # A source with half of itself missing is refused by `settings_store` before anything is run, and
    # that sentence is about the settings rather than about a sign-in — so it is shown where the
    # check was asked for, and never coloured or labelled as a verdict about one.
    page = await fresh(browser, query="?cookies=browser")
    await open_settings(page)
    await page.evaluate(BROWSER_MENU)
    await page.click(".signincheck .button")
    await page.wait_for_selector(".drawer .signin__verdict")
    await page.wait_for_timeout(600)
    half = await page.evaluate(VERDICT)
    check(
        "389 a half-made source gets the settings' own sentence, and is never dressed as a verdict",
        half is not None
        and half["result"] == "pending"
        and half["text"] == "Choose which browser to borrow the sign-in from."
        and half["ink"] != half["danger"]
        and await page.evaluate(TOAST) is None,
        half,
    )
    SIGN_IN_ERRORS.extend(ERRORS[page])
    await page.context.close()


async def settings_links_says_what_is_not_here(browser) -> None:
    """The picker that stopped offering eight browsers to a Mac that has one."""
    page = await fresh(browser, query="?browsers=chrome")
    await open_settings(page)
    menu = await page.evaluate(BROWSER_MENU)
    check(
        "390 a browser this Mac does not have is said to be missing, and is still there to choose",
        menu["here"] == ["", "chrome"]
        and menu["groups"]
        == [
            [
                "Not installed on this Mac",
                ["safari", "chromium", "edge", "brave", "firefox", "vivaldi", "opera"],
            ]
        ]
        # Grouped, never removed and never disabled: a browser installed tomorrow would otherwise
        # be a name the user has to work out the absence of for themselves.
        and len(menu["all"]) == 9
        and not any(disabled for _, _, disabled in menu["all"])
        and all(value == value.lower() for value, _, _ in menu["all"]),
        menu,
    )
    check(
        "391 and the check and the walkthrough stand under the picker, on the one line it gives them",
        await page.evaluate(SIGN_IN_CHECK_CONTROLS)
        == [["button", "Check sign-in", False], ["microlink", "Walk me through it…", False]],
        await page.evaluate(SIGN_IN_CHECK_CONTROLS),
    )
    SIGN_IN_ERRORS.extend(ERRORS[page])
    await page.context.close()


async def the_sheet_answers_the_keyboard_like_the_paste_box(browser) -> None:
    """A third modal over this window, and therefore the paste box's keys exactly (246-255).

    Two cards that look the same and answer the keyboard differently would be worse than either of
    them alone, so this is the same list of questions asked of the same shapes: Esc out with the
    ring handed back, ⌘↩ for the primary action, Tab caught in both directions, a veil the window
    behind never hears a click through.
    """
    page = await fresh(browser, query="?linkfail=signin&browsers=chrome&linkms=20")
    await gated_links(page, [YOUTUBE])
    await page.wait_for_selector(".prompt", timeout=60000)
    await page.keyboard.press("Escape")
    await page.wait_for_selector(".prompt", state="detached")
    await page.click(".row__fix")
    await page.wait_for_selector(".signin")
    await page.wait_for_timeout(300)
    await page.keyboard.press("Escape")
    await page.wait_for_selector(".signin", state="detached")
    check(
        "392 Esc closes it and hands the keyboard back to the control that opened it",
        await page.evaluate("() => document.activeElement?.className") == "microlink row__fix",
        await page.evaluate("() => document.activeElement?.className"),
    )

    await page.click(".row__fix")
    await page.wait_for_selector(".signin")
    await page.wait_for_timeout(300)
    # ⌘↩ rather than ↩, for the paste box's reason: the card is full of buttons, and a bare Return
    # on one of them is that button's own activation.
    await page.evaluate("() => document.querySelector('.signin').focus()")
    await page.keyboard.press("Control+Enter")
    await page.wait_for_selector(".signin .signin__verdict")
    await page.wait_for_timeout(500)
    check(
        "393 ⌘↩ runs the check, without a button having to be found first",
        (await page.evaluate(VERDICT))["text"] == NOTHING_TO_CHECK,
        await page.evaluate(VERDICT),
    )

    await page.evaluate("() => document.querySelector('.signin .promptbutton').focus()")
    ring = []
    for _ in range(5):
        await page.keyboard.press("Tab")
        ring.append(await page.evaluate("() => document.activeElement?.textContent"))
    await page.keyboard.press("Shift+Tab")
    check(
        "394 Tab cycles inside the card, in both directions, and cannot walk out of it",
        ring == ["Use Chrome", "Choose cookies.txt…", "Done", "Check sign-in", "Use Chrome"]
        and await page.evaluate("() => document.activeElement?.textContent") == "Check sign-in"
        and await page.evaluate(
            "() => document.querySelector('.signin').contains(document.activeElement)"
        ),
        ring,
    )
    SIGN_IN_ERRORS.extend(ERRORS[page])
    await page.context.close()

    # The veil is what lets the card be a card: a click that went through it would land on the
    # canvas, which opens the file chooser on a click anywhere.
    page = await fresh(browser, query="?browsers=chrome")
    await open_settings(page)
    await page.evaluate(BROWSER_MENU)
    await page.click(".signincheck .microlink")
    await page.wait_for_selector(".signin")
    await page.wait_for_timeout(400)
    reached = await opens_chooser(page, lambda: page.mouse.click(860, 600))
    check(
        "395 the veil swallows the click that dismisses it: the window behind never hears it",
        not reached and await page.query_selector(".signin") is None,
        reached,
    )
    # Sheets in this app replace each other rather than stack, and the guide is reached *from*
    # Settings — so leaving it has to put the user back where they were standing.
    await page.wait_for_timeout(500)
    check(
        "396 opened from Settings it replaces the sheet, and closing it puts the user back in it",
        await page.query_selector(".drawer[data-open]") is not None
        and await page.query_selector(".signin") is None,
        await page.evaluate("() => document.querySelector('.drawer')?.dataset.open ?? null"),
    )
    SIGN_IN_ERRORS.extend(ERRORS[page])
    await page.context.close()


async def a_failure_no_sign_in_would_have_fixed(browser) -> None:
    """The row this question has to keep its hands off.

    A missing JavaScript runtime is the failure that *read* as a sign-in wall for months (350), and
    the fix for it is an install. A row offering both walks would be asking the user to choose
    between two ways out of a problem nobody has told them the shape of.
    """
    page = await fresh(browser, query="?linkfail=jsruntime&linkms=20")
    await gated_links(page, [YOUTUBE])
    await page.wait_for_selector(".prompt", timeout=60000)
    await page.wait_for_timeout(300)
    check(
        "397 a row a runtime stopped keeps the helper walk, and is never asked about a sign-in",
        await page.evaluate("() => [...document.querySelectorAll('.row__fix')].map((b) => b.textContent)")
        == ["Install Deno"]
        and (await page.text_content(".prompt__title") or "").endswith("couldn’t be converted")
        and "sign-in" not in (await page.text_content(".prompt__title") or "")
        and (await page.text_content(".prompt__body") or "").find("Deno") != -1,
        [await page.text_content(".prompt__title"), await page.evaluate(BLOCKED_ROWS)],
    )
    SIGN_IN_ERRORS.extend(ERRORS[page])
    await page.context.close()


# One gesture, two edits, 60ms apart — a quality changed in the sheet, then the walkthrough opened
# and its browser button pressed. The debounce holds a *snapshot* of the settings it was scheduled
# with, so a save left in the timer fires 300ms later with the sign-in as it was before the button
# was pressed, and silently un-chooses the browser the user just chose. Written as one evaluate
# because the bug lives inside the 300ms and a round trip per click would step over it.
A_SAVE_STILL_IN_THE_TIMER = """
async () => {
  const section = [...document.querySelectorAll('.section')]
    .find((s) => s.querySelector('.section__title').textContent === 'Video');
  section.open = true;
  const quality = [...section.querySelectorAll('select')][1];
  quality.value = 'high';
  quality.dispatchEvent(new Event('change', { bubbles: true }));
  const started = performance.now();
  document.querySelector('.signincheck .microlink').click();
  await new Promise((resolve) => setTimeout(resolve, 60));
  const use = [...document.querySelectorAll('.signin__step button')]
    .find((b) => b.textContent.startsWith('Use '));
  use.click();
  return [Math.round(performance.now() - started), use.textContent];
}
"""


async def a_save_still_in_the_timer(browser) -> None:
    """The bug this pass found: the browser chosen, and unchosen again 300ms later."""
    page = await fresh(browser, query="?browsers=chrome")
    await open_settings(page)
    await page.evaluate(BROWSER_MENU)
    raced = await page.evaluate(A_SAVE_STILL_IN_THE_TIMER)
    await page.wait_for_selector(".signin .signin__verdict[data-result='working']", timeout=8000)
    # Well past the debounce: a save that was going to undo this has had twice as long as it needs.
    await page.wait_for_timeout(900)
    saves = await page.evaluate(LINK_SAVES)
    check(
        "398 a save queued a moment earlier cannot undo the browser the walkthrough just chose",
        raced[0] < 300
        and raced[1] == "Use Chrome"
        # One write, carrying both edits: the immediate one takes the queued one with it rather than
        # racing it, so nothing lands afterwards to put the sign-in back.
        and saves == [["browser", "chrome", None, "high"]]
        and (await page.text_content(".signin__source")) == "Reading the sign-in from Chrome.",
        [raced, saves],
    )
    SIGN_IN_ERRORS.extend(ERRORS[page])
    await page.context.close()
    check("399 the guided sign-in produced no page errors", SIGN_IN_ERRORS == [], SIGN_IN_ERRORS)



FOCUS_HERE = """
() => {
  const node = document.activeElement;
  if (node === null || node === document.body) return ['body', null];
  return [node.className, (node.textContent ?? '').trim()];
}
"""

# One row, sampled from inside the page while it converts: the accessible name it is read by, and
# the percentage printed beside it. A round trip per sample would step over the ticks.
A_NAME_UNDER_THE_RING = """
async () => {
  const row = () => document.querySelectorAll('.row')[0];
  const names = new Set();
  const numbers = new Set();
  for (let i = 0; i < 14; i += 1) {
    const first = row();
    if (first !== undefined && first.dataset.status === 'running') {
      names.add(first.getAttribute('aria-label'));
      numbers.add(first.querySelector('.row__status')?.textContent ?? '');
    }
    await new Promise((resolve) => setTimeout(resolve, 60));
  }
  return [[...names], [...numbers]];
}
"""


async def the_question_hands_the_keyboard_on(browser) -> None:
    """The walkthrough opened *from* the question, and where Done leaves the user."""
    page = await fresh(browser, query="?linkfail=signin&browsers=chrome&linkms=20")
    await gated_links(page, [YOUTUBE, YOUTUBE_SHORT])
    await page.wait_for_selector(".prompt", timeout=60000)
    await page.wait_for_timeout(300)
    await page.evaluate(
        "() => [...document.querySelectorAll('.prompt .microlink')]"
        ".find((b) => b.textContent.startsWith('Walk me')).click()"
    )
    await page.wait_for_selector(".signin")
    await page.wait_for_timeout(300)
    handed = await page.evaluate(FOCUS_HERE)
    await page.keyboard.press("Escape")
    await page.wait_for_selector(".signin", state="detached")
    await page.wait_for_timeout(300)
    landed = await page.evaluate(FOCUS_HERE)
    check(
        "400 the walkthrough taken from the question hands the ring back to what raised it",
        handed[0] == "promptbutton"
        # Not `<body>`, and not the canvas either: a queue is standing, so there is no canvas to
        # fall back to — which is exactly how the lost opener showed up as a lost keyboard.
        and landed[0] == "pill"
        and landed[1].startswith("Convert")
        and await page.query_selector(".dropzone") is None,
        [handed, landed],
    )
    check("401 the hand-over produced no page errors", ERRORS[page] == [], ERRORS[page])
    await page.context.close()


async def two_clocks_that_have_to_agree(browser) -> None:
    """The app's `secondsLabel` and the mock's `durationLabel`, on the same 12.6 seconds.

    They are two independent spellings of `probe::seconds_label` and they stay that way: the mock is
    the backend's stand-in, and importing the app's helper into it would hide exactly the drift this
    asserts against. 322 holds the mock's half; this holds them to each other, in one window.
    """
    page = await fresh(browser, query="?clipsecs=12.6")
    await page.evaluate(DROP, ["clip.mov"])
    await page.wait_for_selector(".row")
    await open_settings(page)
    await page.evaluate(TRIM_TOGGLE)
    await page.evaluate(TRIM_SET, ["Keep", "12.6", True])
    await trim_written(page, [True, 0, 12.6])
    await page.keyboard.press("Escape")
    await page.wait_for_function(
        "() => (document.querySelector('.actionbar__text')?.textContent ?? '').includes('Trimming')"
    )
    metas = await page.evaluate(ROW_METAS)
    bar = await page.text_content(".actionbar__text")
    check(
        "402 the backend's spelling of a length and the app's are the same digits, not nearly",
        metas[0].endswith(" · 0:12") and (bar or "").endswith("Trimming to 0:12"),
        [metas, bar],
    )
    check("403 the two clocks produced no page errors", ERRORS[page] == [], ERRORS[page])
    await page.context.close()


async def a_row_says_how_it_is_getting_on(browser) -> None:
    """The name a row is read by: the file, and the state — never the percentage."""
    page = await fresh(browser)
    await page.evaluate(DROP, ["one.mov", "broken-take.mov", "notes.zip"])
    await page.wait_for_function("() => document.querySelectorAll('.row').length === 3")
    queued = await page.evaluate(ROW_LABELS)

    await page.click(".pill")
    await page.wait_for_function(
        "() => [...document.querySelectorAll('.row')].some((r) => r.dataset.status === 'running')",
        timeout=20000,
    )
    names, numbers = await page.evaluate(A_NAME_UNDER_THE_RING)
    await page.wait_for_function(
        "() => document.querySelector('.pill').textContent.startsWith('Convert')", timeout=60000
    )
    await page.wait_for_timeout(400)
    settled = await page.evaluate(ROW_LABELS)
    check(
        "404 a row is read by its state as well as its name, and a queued one is the name alone",
        # A file nothing can convert says so where it is heard, rather than only where it is seen.
        queued == ["one.mov", "broken-take.mov", "notes.zip, unsupported"]
        and settled == ["one.mov, converted", "broken-take.mov, failed", "notes.zip, unsupported"],
        [queued, settled],
    )
    check(
        "405 and the state it is read by is not rewritten on every progress tick",
        names == ["one.mov, converting"]
        # The percentage really was moving underneath: the name is quiet by choice, not by luck.
        and len(numbers) > 1
        and not any(char.isdigit() for char in names[0]),
        [names, numbers],
    )
    check("406 the spoken row produced no page errors", ERRORS[page] == [], ERRORS[page])
    await page.context.close()


# 407 is the folder under the bar, held against the file it is derived from. `State.destination` is
# an estimated output *path* for a file queue, and the bar's job is to name the folder that path
# sits in — so the tooltip (the whole path) and the sentence (the folder) may never be the same
# string. `parentDir` used to answer an empty parent with the path itself, which is a file wearing
# a folder's label; it now answers `/` or nothing, and the store turns nothing into no sentence.
# 281 holds the other shape of the same slot: a queue of links carries a folder already.


async def the_bar_names_a_folder_not_a_file(browser) -> None:
    """The Saves-to sentence is the estimate's parent, never the estimate."""
    page = await fresh(browser)
    await page.evaluate(DROP, ["clip.mov"])
    await page.wait_for_selector(".row")
    await page.wait_for_function(
        "() => (document.querySelector('.actionbar__text')?.textContent ?? '')"
        ".startsWith('Saves to')"
    )
    sentence = (await page.text_content(".actionbar__text")) or ""
    whole_path = await page.get_attribute(".actionbar__text", "title")
    folder = sentence.replace("Saves to ", "")
    check(
        "407 the bar names the folder the estimate sits in, and never the estimated file",
        whole_path == "/Users/you/Desktop/Converted/clip.mp4"
        and folder == "/Users/you/Desktop/Converted"
        and folder != whole_path
        and (whole_path or "").startswith(folder + "/"),
        [sentence, whole_path],
    )
    check("408 the named folder produced no page errors", ERRORS[page] == [], ERRORS[page])
    await page.context.close()



UNMEASURED_LOG = """
() => {
  // Per row, every distinct (status text, has a pulse, line width) it holds while it runs. A
  // MutationObserver rather than a poll: the whole point is that no sample ever printed a number,
  // and a sampler that can blink is not evidence.
  const log = [[], []];
  const sample = () => {
    [...document.querySelectorAll('.row')].forEach((row, i) => {
      const list = log[i];
      const status = row.querySelector('.row__status');
      if (status === null || list === undefined) return;
      const bar = row.querySelector('.row__progress');
      const state = [
        status.textContent,
        row.querySelector('.row__pulse') !== null,
        bar === null ? null : Math.round(parseFloat(bar.style.width)),
      ];
      const last = list[list.length - 1];
      if (last === undefined || String(last) !== String(state)) list.push(state);
    });
  };
  const observer = new MutationObserver(sample);
  observer.observe(document.querySelector('.filelist__rows'), {
    subtree: true, childList: true, characterData: true, attributes: true,
  });
  window.__ceUnmeasuredStop = () => observer.disconnect();
  window.__ceUnmeasuredLog = log;
  sample();
}
"""


async def a_job_nobody_can_measure(browser) -> None:
    """An unmeasurable job says it is working without claiming to know how far along it is."""
    page = await fresh(browser, query="?nolength=stream.mov")
    await page.evaluate(DROP, ["stream.mov", "clip.mov"])
    await page.wait_for_function("() => document.querySelectorAll('.row').length === 2")
    metas = await page.evaluate(ROW_METAS)
    check(
        "409 a file whose length nobody could read says nothing about how long it is",
        re.search(r" · \d+:\d\d", metas[0]) is None
        and re.search(r" · \d+:\d\d", metas[1]) is not None,
        metas,
    )

    await page.evaluate(UNMEASURED_LOG)
    await convert_and_settle(page)
    log = await page.evaluate(
        "() => { window.__ceUnmeasuredStop(); return window.__ceUnmeasuredLog; }"
    )
    unmeasured, measured = log[0], log[1]
    # `started` carries no sample, and a job that has just started really is at nothing: the first
    # frame is the same "0%" every row opens with. What follows is the whole of the run.
    sampled = unmeasured[1:]
    check(
        "410 the first thing FFmpeg says takes the number away, and no sample ever brings one back",
        len(sampled) > 0
        and all(text == "Converting…" for text, _, _ in sampled)
        and not any(char.isdigit() for text, _, _ in sampled for char in text),
        unmeasured[:4],
    )
    check(
        "411 and it is drawn as a travelling line rather than one pinned to nothing",
        all(pulse and width is None for _, pulse, width in sampled)
        # The row opened the way every row opens, and then went indeterminate and stayed there.
        and unmeasured[0] == ["0%", False, 0],
        unmeasured[:4],
    )
    widths = [width for _, _, width in measured if width is not None]
    check(
        "412 while the measured file beside it counts up in the same batch, as it always has",
        len(widths) > 1
        and widths == sorted(widths)
        and all(re.fullmatch(r"\d+%( · .+)?", text) for text, _, _ in measured)
        and not any(pulse for _, pulse, _ in measured),
        measured[:4],
    )
    check(
        "413 an unmeasurable job still finishes, with its file named and no line left behind",
        await page.evaluate(STATUSES) == ["done", "done"]
        and (await page.text_content(".row:first-child .row__output") or "").startswith("stream.")
        and await page.query_selector(".row__pulse") is None
        and await page.query_selector(".row__progress") is None,
        [await page.evaluate(STATUSES), await page.text_content(".row:first-child .row__output")],
    )
    check("414 the unmeasurable job produced no page errors", ERRORS[page] == [], ERRORS[page])
    await page.context.close()



async def a_batch_that_ended_in_a_panic(browser) -> None:
    """The truthful last word after a crash, arriving late, and then arriving again."""
    page = await fresh(browser, query="?activity=converting")
    await page.evaluate(DROP, ["one.mov", "two.mov"])
    await page.wait_for_function("() => document.querySelectorAll('.row').length === 2")
    check(
        "415 the window is holding the shell's batch, with two rows of its own still queued",
        await page.evaluate(PILL) == "Stop" and await page.evaluate(STATUSES) == ["queued", "queued"],
        [await page.evaluate(PILL), await page.evaluate(STATUSES)],
    )

    # `SlotGuard`'s tally: what the window was told before the thread died, and the rest as skipped.
    closeout = {"type": "batch_finished", "ok": 1, "failed": 0, "skipped": 2}
    await page.evaluate(EMIT, closeout)
    await page.wait_for_timeout(200)
    text = await page.text_content(".actionbar__text")
    check(
        "416 a batch closed out after a panic ends the wait instead of spinning forever",
        (await page.evaluate(PILL) or "").startswith("Convert")
        and await page.query_selector(".actionbar__progress") is None,
        [await page.evaluate(PILL), text],
    )
    check(
        "417 and it reports no tally over a queue it never converted a file of",
        "converted" not in (text or "")
        and "skipped" not in (text or "")
        and await page.evaluate(STATUSES) == ["queued", "queued"]
        and await page.query_selector(".prompt") is None,
        [text, await page.evaluate(STATUSES)],
    )

    await page.evaluate(EMIT, closeout)
    await page.evaluate(EMIT, closeout)
    await page.wait_for_timeout(200)
    check(
        "418 a second copy of that last word changes nothing: no tally, no revived batch",
        await page.text_content(".actionbar__text") == text
        and await page.evaluate(STATUSES) == ["queued", "queued"]
        and (await page.evaluate(PILL) or "").startswith("Convert"),
        [await page.text_content(".actionbar__text"), await page.evaluate(STATUSES)],
    )

    # And the window is the user's again: the slot the shell held is gone, so Convert converts.
    await convert_and_settle(page)
    check(
        "419 the queue converts afterwards, which is the proof the adoption was really retired",
        await page.evaluate(STATUSES) == ["done", "done"],
        await page.evaluate(STATUSES),
    )
    check("420 closing out that batch produced no page errors", ERRORS[page] == [], ERRORS[page])
    await page.context.close()


async def an_install_that_ended_in_a_panic(browser) -> None:
    """The same last word from the installer: `finished { ok: false }`, late and then again."""
    page = await fresh(browser, query="?activity=installing&missing=pandoc,libreoffice&installms=1200")
    await open_settings(page)
    check(
        "421 the window is holding an install the shell started, with every button waiting on it",
        await page.eval_on_selector_all(".toolbutton", "els => els.every((b) => b.disabled)")
        and await page.eval_on_selector_all(".tool__waiting", "els => els.length") == 2,
        await page.eval_on_selector_all(".toolbutton", "els => els.map((b) => b.disabled)"),
    )

    verdict = {
        "type": "finished",
        "package_id": "pandoc",
        "ok": False,
        "message": "The install ended before it finished.",
    }
    await page.evaluate("(event) => window.__ceMockInstallEmit(event)", verdict)
    await page.wait_for_selector('.install[data-status="failed"]', timeout=10000)
    await page.wait_for_timeout(300)
    check(
        "422 an install closed out after a panic names the helper and says what happened",
        await page.eval_on_selector_all(".install", "els => els.length") == 1
        and "The install ended before it finished."
        in (await page.text_content(f"{tool('pandoc')} .install__state") or ""),
        await page.text_content(f"{tool('pandoc')} .install__state"),
    )
    check(
        "423 and the helpers it was holding are the user's again, rather than dead for the session",
        await page.evaluate(
            "() => { const b = document.querySelector('.tool[data-tool=\"libreoffice\"] .toolbutton');"
            " return b !== null && !b.disabled; }"
        )
        and await page.eval_on_selector_all(".tool__waiting", "els => els.length") == 0,
        await page.eval_on_selector_all(".toolbutton", "els => els.map((b) => b.disabled)"),
    )

    state = await page.text_content(f"{tool('pandoc')} .install__state")
    await page.evaluate(
        "(event) => window.__ceMockInstallEmit({ ...event, ok: true, message: 'Pandoc installed.' })",
        verdict,
    )
    await page.evaluate("(event) => window.__ceMockInstallEmit(event)", verdict)
    await page.wait_for_timeout(300)
    check(
        "424 first verdict wins: a later copy cannot turn that failure into a success",
        await page.evaluate("() => document.querySelector('.install').dataset.status") == "failed"
        and await page.text_content(f"{tool('pandoc')} .install__state") == state,
        [
            await page.evaluate("() => document.querySelector('.install').dataset.status"),
            await page.text_content(f"{tool('pandoc')} .install__state"),
        ],
    )
    check("425 closing out that install produced no page errors", ERRORS[page] == [], ERRORS[page])
    await page.context.close()


# The Safari permission, in the three sentences a user actually needs — `FetchFailure::
# SafariNeedsFullDiskAccess`, and the same words `check_safari_cookie_access` answers with. Held here
# in full because the *relaunch* and the rebuild caveat are the halves that were missing, and a
# suite that only looked for "Full Disk Access" would have passed on the sentence that failed a user.
SAFARI_NEEDS_FDA = (
    "Safari keeps its cookies where only an app with Full Disk Access can read them, and "
    "Flint does not have it. Grant it in System Settings → Privacy & Security → Full "
    "Disk Access, then quit Flint and open it again - the permission only reaches an app "
    "that was started after it was given. A copy you built yourself loses the grant every time it "
    "is rebuilt, so switch it off and on again in that list. The easier route is to pick a browser "
    "in Settings → Links that needs no permission at all."
)

# The sign-in sources `test_cookie_source` was actually made to probe, in order — `__ceMockProbes`,
# installed by `src/lib/mock.ts`. Safari's answer is a local permission, so what has to be asserted
# about it is a negative: that twenty seconds of yt-dlp were never spent finding it out.
PROBES = "() => window.__ceMockProbes ?? []"

BROWSER_ERRORS: list = []


async def timed_check(page: Page) -> float:
    """Press Check sign-in in Settings → Links and time the answer, in seconds."""
    await page.click(".signincheck .button")
    started = time.monotonic()
    await page.wait_for_function(
        f"() => {VERDICT_LINE}?.dataset.result !== 'pending' && {VERDICT_LINE} !== null",
        timeout=8000,
    )
    return time.monotonic() - started


async def the_browser_the_sign_in_is_actually_in(browser) -> None:
    """Whose sign-in is it, rather than whose jar is cheapest to open.

    The rule this replaces preferred any browser that was not Safari, because Safari's cookies cost
    a Full Disk Access grant. On the Mac that reported it (`?mac=safari`) that meant offering a
    Chrome whose store is an untouched 65,536 bytes — the empty size — while the user's YouTube
    session sat in the Safari they browse with, 346 KB of it, written minutes earlier. The app was
    confidently borrowing an empty jar and calling the result a sign-in problem.

    `list_cookie_browsers` now ranks the rows by that evidence and the frontend reads rank 1 back, so
    these assertions are about *which* browser is named, *why* the app says it named it, and the two
    machines where the honest answer is to name none.
    """
    page = await fresh(browser, query="?mac=safari&linkfail=signin&linkms=20")
    await gated_links(page, [YOUTUBE, YOUTUBE_SHORT])
    await page.wait_for_selector(".prompt", timeout=60000)
    await page.wait_for_timeout(300)
    semantics = await page.evaluate(PROMPT_SEMANTICS)
    labels = await page.evaluate(
        "() => [...document.querySelectorAll('.prompt__actions button')].map((b) => b.textContent)"
    )
    check(
        "426 the browser offered is the one the sign-in is in, not the one that costs nothing to read",
        semantics["label"] == "2 links need a sign-in. It is most likely in Safari — you used it "
        "recently — and macOS keeps those cookies behind Full Disk Access."
        # The empty Chrome is on this Mac and is named nowhere: not in the question, not on a button.
        and "Chrome" not in (semantics["label"] or "")
        and not any("Chrome" in (label or "") for label in labels)
        and labels == ["Not now", "Open Full Disk Access…"],
        [semantics["label"], labels],
    )
    check(
        "427 and the reason it was picked is said in one clause, about a file's age and nothing in it",
        (semantics["body"] or "").startswith("Safari — you used it recently.")
        # What the app knows is a size and a modification time. Neither the question nor the promise
        # may suggest it has looked at a cookie, an account or a site.
        and not any(
            word in (semantics["body"] or "") for word in ["cookie for", "account", "signed in to"]
        ),
        semantics["body"],
    )
    BROWSER_ERRORS.extend(ERRORS[page])
    await page.context.close()

    page = await walkthrough_from_settings(browser, "?mac=safari")
    sheet = await page.evaluate(SIGN_IN_SHEET)
    steps = sheet["steps"]
    check(
        "428 the walkthrough says the same why, and walks the same browser",
        sheet["why"] == "Safari — you used it recently."
        and [s[1] for s in steps][:2] == [[], ["Use Safari"]]
        and steps[0][0].startswith("Sign in to the site in Safari first"),
        [sheet["why"], [s[1] for s in steps]],
    )
    check(
        "429 and an empty cookie store is never quietly offered instead, on the Mac that has one",
        not any("Use Chrome" in b for _, buttons in steps for b in buttons)
        and not any("Chrome" in text for text, _ in steps),
        [s[0][:40] for s in steps],
    )
    BROWSER_ERRORS.extend(ERRORS[page])
    await page.context.close()

    # The ordinary machine, untouched: Chrome is the default browser here *and* the one written to
    # minutes ago, so the one-click path is still the one-click path — and it says so for a reason.
    page = await walkthrough_from_settings(browser, "?mac=chrome")
    primary = await page.evaluate(SIGN_IN_SHEET)
    check(
        "430 a Chrome-primary Mac still gets the click, with the same clause behind the same name",
        primary["why"] == "Chrome — you used it recently."
        and [s[1] for s in primary["steps"]][:2] == [[], ["Use Chrome"]],
        [primary["why"], [s[1] for s in primary["steps"]]],
    )
    BROWSER_ERRORS.extend(ERRORS[page])
    await page.context.close()

    # `?mac=nostore`: Chrome and Safari are both installed and neither has a cookie store on disk.
    # Nothing can be borrowed from a jar that is not there, and the old rule would have offered
    # Chrome anyway, because the old rule only ever asked whether the `.app` existed.
    page = await fresh(browser, query="?mac=nostore&linkfail=signin&linkms=20")
    await gated_links(page, [YOUTUBE, YOUTUBE_SHORT])
    await page.wait_for_selector(".prompt", timeout=60000)
    await page.wait_for_timeout(300)
    missing = await page.evaluate(PROMPT_SEMANTICS)
    check(
        "431 a Mac whose stores are missing is offered the file route, and no browser at all",
        missing["label"] == "2 links need a sign-in. No browser here can lend one, so the way in "
        "is an exported cookies.txt file."
        and "browser extension" in (missing["body"] or "")
        and await page.evaluate(LINK_SAVES) == [],
        [missing["label"], await page.evaluate(LINK_SAVES)],
    )
    BROWSER_ERRORS.extend(ERRORS[page])
    await page.context.close()

    page = await walkthrough_from_settings(browser, "?mac=nostore")
    empty = await page.evaluate(SIGN_IN_SHEET)
    menu = await page.evaluate(BROWSER_MENU)
    check(
        "432 and its walkthrough offers none either, though both browsers are installed on it",
        empty["why"] is None
        and not any(b.startswith("Use ") for _, buttons in empty["steps"] for b in buttons)
        and empty["steps"][0][0].startswith("Flint found none of the browsers")
        # Installed, and still not offered: this is the difference between "you have no browser" and
        # "your browsers have no sign-in in them", which the picker goes on reporting correctly.
        and menu["groups"]
        == [
            [
                "Not installed on this Mac",
                ["chromium", "edge", "brave", "firefox", "vivaldi", "opera"],
            ]
        ],
        [empty["why"], [s[1] for s in empty["steps"]], menu["groups"]],
    )
    BROWSER_ERRORS.extend(ERRORS[page])
    await page.context.close()


async def safari_answers_from_the_permission(browser) -> None:
    """The question that was being asked over the network, and is a `stat`-sized question at home.

    Checking a Safari sign-in used to mean `test_cookie_source`: run yt-dlp against a public video
    with `--cookies-from-browser safari` and wait up to twenty seconds to be told, in the end, that
    the app does not hold a macOS permission. `check_safari_cookie_access` opens the jar and drops
    it — one syscall, no bytes read, no network — so the permission answers for itself, and the
    probe is kept for the half of the question it is actually good for: whether the site agrees.
    """
    page = await fresh(browser, query="?mac=safari&cookies=safari")
    await open_settings(page)
    await page.evaluate(BROWSER_MENU)
    safari_secs = await timed_check(page)
    verdict = await page.evaluate(VERDICT)
    probes = await page.evaluate(PROBES)
    check(
        "433 Safari's check answers from the permission itself, and runs no probe to find it out",
        verdict is not None
        and verdict["result"] == "unreadable"
        and verdict["text"] == SAFARI_NEEDS_FDA
        and probes == [],
        [verdict, probes],
    )
    check(
        "434 and it says all three things that block a Safari user: grant, relaunch, rebuild",
        "System Settings → Privacy & Security → Full Disk Access" in verdict["text"]
        and "quit Flint and open it again" in verdict["text"]
        and "started after it was given" in verdict["text"]
        and "loses the grant every time it is rebuilt" in verdict["text"],
        verdict["text"],
    )
    BROWSER_ERRORS.extend(ERRORS[page])
    await page.context.close()

    # The control, on the same machine and the same button: a browser whose answer really is a
    # network answer waits for the network. Twenty seconds in Rust, 400ms of pretend probe here.
    page = await fresh(browser, query="?mac=safari&cookies=chrome")
    await open_settings(page)
    await page.evaluate(BROWSER_MENU)
    chrome_secs = await timed_check(page)
    check(
        "435 Safari's answer arrives at once, where a browser that must be probed is still probing",
        safari_secs + 0.2 < chrome_secs and await page.evaluate(PROBES) == ["chrome"],
        [round(safari_secs, 3), round(chrome_secs, 3)],
    )
    BROWSER_ERRORS.extend(ERRORS[page])
    await page.context.close()

    # `?fda` is that same Mac with the permission granted and the app relaunched. `readable` proves
    # the file opens and nothing more, so the check goes on to ask the site — which is the question
    # the probe was always for, and the only one worth twenty seconds.
    page = await fresh(browser, query="?mac=safari&cookies=safari&fda")
    await open_settings(page)
    await page.evaluate(BROWSER_MENU)
    await timed_check(page)
    granted = await page.evaluate(VERDICT)
    check(
        "436 with the permission in place it goes on to the site's half, which is what a probe is for",
        granted["result"] == "working"
        and granted["text"] == WORKS
        and await page.evaluate(PROBES) == ["safari"],
        [granted, await page.evaluate(PROBES)],
    )
    BROWSER_ERRORS.extend(ERRORS[page])
    await page.context.close()

    page = await walkthrough_from_settings(browser, "?mac=safari")
    steps = (await page.evaluate(SIGN_IN_SHEET))["steps"]
    grant = next(text for text, _ in steps if "Full Disk Access" in text)
    caveat = next(text for text, _ in steps if "built this copy" in text)
    check(
        "437 the walk is grant, relaunch, check — in that order, with the button on the first of them",
        grant.index("Grant it") < grant.index("quit Flint and open it again")
        < grant.index("Check sign-in")
        and "the permission only reaches an app that was started after it was given" in grant
        and ["Open Full Disk Access…"] in [buttons for text, buttons in steps if text == grant],
        grant,
    )
    check(
        "438 and the rebuild caveat stands beside it, because a self-built copy loses the grant",
        "loses that grant every time it is rebuilt" in caveat
        and "looking switched on" in caveat
        and "Switch it off and on again" in caveat,
        caveat,
    )
    check(
        "439 no easier browser is named here, because this Mac's other jar is the empty one",
        not any("Or borrow" in text for text, _ in steps),
        [s[0][:40] for s in steps],
    )
    BROWSER_ERRORS.extend(ERRORS[page])
    await page.context.close()

    # The same Mac with a Firefox on it that has really been used. Now the alternative is real, so
    # it is offered — and it is Firefox, never the Chrome sitting at the empty size.
    page = await walkthrough_from_settings(browser, "?mac=safari&browsers=safari,chrome,firefox")
    with_firefox = (await page.evaluate(SIGN_IN_SHEET))["steps"]
    easier = next(text for text, _ in with_firefox if "Or borrow" in text)
    check(
        "440 and it is named the moment one is really there, needing no permission and no export",
        easier == "Or borrow Firefox instead: it is here too, it has a sign-in saved, and it needs "
        "no permission at all."
        and "Chrome" not in easier
        # Still Safari's walk: the alternative is an alternative, not a change of mind about where
        # the sign-in is.
        and [s[1] for s in with_firefox][:2] == [[], ["Use Safari"]],
        easier,
    )
    check(
        "441 borrowing the right browser produced no page errors",
        BROWSER_ERRORS + ERRORS[page] == [],
        BROWSER_ERRORS + ERRORS[page],
    )
    await page.context.close()


def require_dev_server() -> None:
    """Fail with the fix rather than a 30s Playwright timeout dump.

    Every assertion here needs the Vite dev server; forgetting it is the single most likely way to
    run this script wrong, and Playwright's own error for a closed port buries the cause in a stack.
    """
    import urllib.error
    import urllib.request

    try:
        urllib.request.urlopen(URL, timeout=5).read(1)
    except (urllib.error.URLError, OSError) as e:
        sys.exit(
            f"no dev server on {URL} ({e}).\n"
            "Start one first, from the project root:\n"
            "    npm run dev &\n"
            "    python3 scripts/ui_behaviour.py"
        )


async def edited_links_cannot_submit_stale_results(browser) -> None:
    page = await fresh(browser)
    await open_links(page)
    await page.fill(".links__box", "https://youtu.be/Jv8LmQrTx6A")
    assert await judged(page, 1)
    await page.fill(".links__box", "not a video")
    check(
        "442 editing a validated link immediately disables Add",
        await page.locator(".links .promptbutton").is_disabled(),
    )
    await page.keyboard.press("Control+Enter")
    check(
        "443 submitting during validation cannot queue the previous URL",
        await page.locator(".links").count() == 1 and await page.locator(".row").count() == 0,
    )
    check("444 link validation race produces no page errors", ERRORS[page] == [], ERRORS[page])
    await page.context.close()


async def main() -> None:
    require_dev_server()
    async with async_playwright() as p:
        browser = await p.chromium.launch()
        await run(browser)
        await browser.close()
    print(f"\n{passed} passed, {len(failures)} failed")
    for f in failures:
        print("  FAIL", f)
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    asyncio.run(main())
