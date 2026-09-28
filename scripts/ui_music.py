"""Music-link UI regression against the deterministic preview backend."""
import asyncio
from pathlib import Path
from playwright.async_api import async_playwright
from ui_behaviour import fresh

LINKS = [
    "https://y.qq.com/n/ryqq/songDetail/004Ti8rT003TaZ",
    "https://music.163.com/#/song?id=17241424",
    "https://soundcloud.com/the80m/the-following",
    "https://benprunty.bandcamp.com/track/lanius-battle",
]


async def check(browser, width, height):
    page = await fresh(browser, width=width, height=height, query="linkms=5")
    errors = []
    page.on("pageerror", lambda error: errors.append(str(error)))
    await page.keyboard.press("Control+l")
    box = page.get_by_role("textbox", name="Media links, one per line")
    await box.fill("\n".join(LINKS + ["https://open.spotify.com/track/abc"]))
    await page.get_by_role("button", name="Add 4 links").wait_for()
    assert "Spotify is not supported" in await page.locator(".links").inner_text()
    await page.get_by_role("button", name="Add 4 links").click()
    assert await page.locator(".row").count() == 4
    text = await page.locator(".filelist").inner_text() if await page.locator(".filelist").count() else await page.locator("main").inner_text()
    assert "QQ Music" in text and "NetEase Music" in text
    assert "MP3" in text.upper()
    await page.get_by_role("button", name="More conversion options").click()
    await page.get_by_role("menuitem", name="Crop & convert").click()
    dialog = page.get_by_role("dialog", name="Crop & convert")
    await dialog.get_by_label("Start (seconds)", exact=True).fill("10")
    await dialog.get_by_label("Range", exact=True).select_option("end")
    await dialog.get_by_label("End (seconds)", exact=True).fill("50")
    assert "40s kept" in await dialog.inner_text()
    await dialog.get_by_role("button", name="Crop & convert 4 files").click()
    await page.wait_for_function("document.querySelectorAll('.row[data-status=\"done\"]').length === 4")
    assert await page.evaluate("document.body.scrollWidth === document.body.clientWidth")
    assert await page.evaluate("window.__ceMockSaves.length") == 0
    assert not errors, errors
    await page.screenshot(path=str(Path("/tmp") / f"cc-music-{width}.png"))
    await page.context.close()
    print(f"PASS {width}x{height}: four music sources, Spotify refusal, audio defaults, batch crop")


async def main():
    async with async_playwright() as p:
        browser = await p.chromium.launch()
        for size in [(960, 660), (720, 520), (390, 700)]:
            await check(browser, *size)
        await browser.close()


if __name__ == "__main__":
    asyncio.run(main())
