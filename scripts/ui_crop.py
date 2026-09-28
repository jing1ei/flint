"""Numeric batch crop UI regression. Uses the preview backend; real outputs have Rust tests."""
import asyncio
import argparse
from pathlib import Path
from playwright.async_api import async_playwright
from ui_behaviour import fresh, DROP, ERRORS


async def check(browser, width, height):
    page = await fresh(browser, width=width, height=height, query="clipsecs=120")
    await page.evaluate(DROP, ["photo.png", "clip.mp4", "notes.txt"])
    await page.wait_for_selector(".row")
    toggle = page.get_by_role("button", name="More conversion options")
    menu_item = page.get_by_role("menuitem", name="Crop & convert")
    await toggle.click()
    # Holding the pointer reveals WebKit's blur-before-click behavior: the item
    # must survive mousedown until mouseup actually activates it.
    box = await menu_item.bounding_box()
    assert box is not None
    await page.mouse.move(box["x"] + box["width"] / 2, box["y"] + box["height"] / 2)
    await page.mouse.down()
    await page.wait_for_timeout(100)
    assert await menu_item.is_visible(), "menu item was removed before its click"
    await page.mouse.up()
    dialog = page.get_by_role("dialog", name="Crop & convert")
    await dialog.wait_for(timeout=3000)
    await dialog.get_by_role("button", name="Cancel", exact=True).click()
    await dialog.wait_for(state="hidden")
    await toggle.click()
    await toggle.click()
    assert await menu_item.count() == 0, "second toggle click must close the menu"
    await toggle.focus()
    await page.keyboard.press("ArrowDown")
    await page.keyboard.press("Enter")
    await dialog.wait_for(timeout=3000)
    await page.keyboard.press("Escape")
    await dialog.wait_for(state="hidden")
    await toggle.click()
    await page.mouse.click(8, 8)
    assert await menu_item.count() == 0, "outside click must close the menu"
    await toggle.click()
    await page.locator(".row select").first.focus()
    assert await menu_item.count() == 0, "outside focus must close the menu"
    await toggle.click()
    await menu_item.click()
    await dialog.wait_for(timeout=3000)
    await page.get_by_label("Start (seconds)", exact=True).fill("10")
    await page.get_by_label("Range", exact=True).select_option("end")
    await page.get_by_label("End (seconds)", exact=True).fill("50")
    assert "40s kept" in await dialog.inner_text()
    await page.get_by_label("Unit", exact=True).select_option("words")
    await page.get_by_label("From (inclusive)", exact=True).fill("2000")
    assert await page.get_by_label("Through (inclusive)", exact=True).input_value() == "10000"
    await page.get_by_label("Width (px)", exact=True).fill("0")
    assert await dialog.get_by_role("button", name="Crop & convert 3 files").is_disabled()
    await page.get_by_label("Width (px)", exact=True).fill("100")
    await page.keyboard.press("Control+Enter")
    assert await dialog.is_visible(), "background Convert shortcut escaped the modal"
    for key in ["Tab", "Shift+Tab"]:
        for _ in range(25):
            await page.keyboard.press(key)
            assert await dialog.evaluate("d => d.contains(document.activeElement)")
    await page.get_by_role("checkbox", name="Documents", exact=True).uncheck()
    assert "1 unchanged in queue" in await dialog.inner_text()
    assert await page.evaluate("document.body.scrollWidth === document.body.clientWidth")
    assert await dialog.get_by_role("button", name="Crop & convert 2 files").evaluate(
        "e => { const r = e.getBoundingClientRect(); return r.top >= 0 && r.bottom <= innerHeight; }"
    ), "batch action must remain visible without scrolling"
    screenshot = Path("/tmp") / f"cc-crop-{browser.browser_type.name}-{width}.png"
    await page.screenshot(path=str(screenshot))
    await dialog.get_by_role("button", name="Crop & convert 2 files").click()
    await dialog.wait_for(state="hidden")
    await page.wait_for_function("document.querySelectorAll('.row[data-status=\"done\"]').length === 2")
    assert await page.locator('.row[data-status="queued"]').count() == 1
    assert await page.evaluate("window.__ceMockSaves.length") == 0
    await page.get_by_role("button", name="More conversion options").click()
    await page.keyboard.press("Escape")
    assert await page.get_by_role("button", name="More conversion options").evaluate("e => e === document.activeElement")
    assert not ERRORS[page], ERRORS[page]
    await page.context.close()
    print(f"PASS {browser.browser_type.name} {width}x{height}: split menu, ranges, validation, modal keyboard, batch filtering, preferences")


async def legacy_dialog_api(browser):
    page = await fresh(browser)
    await page.evaluate("""() => {
        HTMLDialogElement.prototype.showModal = undefined;
        HTMLDialogElement.prototype.close = undefined;
    }""")
    await page.evaluate(DROP, ["photo.png"])
    await page.wait_for_selector(".row")
    assert not await page.locator(".crop-dialog").is_visible()
    await page.get_by_role("button", name="More conversion options").click()
    await page.get_by_role("menuitem", name="Crop & convert").click()
    dialog = page.get_by_role("dialog", name="Crop & convert")
    await dialog.wait_for()
    await dialog.get_by_role("button", name="Cancel", exact=True).click()
    await dialog.wait_for(state="hidden")
    await page.get_by_role("button", name="More conversion options").click()
    await page.get_by_role("menuitem", name="Crop & convert").click()
    await dialog.get_by_role("button", name="Crop & convert 1 files").click()
    await page.wait_for_selector('.row[data-status="done"]')
    assert not ERRORS[page], ERRORS[page]
    await page.context.close()
    print(f"PASS {browser.browser_type.name}: crop fallback opens, cancels, reopens and converts without native dialog APIs")


async def main(browser_name):
    async with async_playwright() as p:
        browser = await getattr(p, browser_name).launch()
        for size in [(960, 660), (720, 520), (390, 700)]:
            await check(browser, *size)
        await legacy_dialog_api(browser)
        await browser.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--browser", choices=["chromium", "webkit"], default="chromium")
    asyncio.run(main(parser.parse_args().browser))
