"""Skin workflow and conversion isolation checks against the local browser preview."""
import asyncio
import json
from pathlib import Path

from playwright.async_api import async_playwright
from ui_behaviour import DROP, fresh

ROOT = Path(__file__).resolve().parent.parent
KEY = "cross-converter.skin.v1"

GEOMETRY = """() => Object.fromEntries(['.app','.titlebar','.main','.row','.actionbar'].map(s => {
 const el = document.querySelector(s), r = el.getBoundingClientRect(), c = getComputedStyle(el);
 return [s, {x:r.x,y:r.y,width:r.width,height:r.height,padding:c.padding,gap:c.gap,
   fontSize:c.fontSize,lineHeight:c.lineHeight,display:c.display}];
}))"""


async def check(browser, width, scheme):
    page = await fresh(browser, width=width, height=660, scheme=scheme)
    errors = []
    page.on("pageerror", lambda error: errors.append(str(error)))
    await page.context.grant_permissions(["clipboard-read", "clipboard-write"])
    await page.evaluate(DROP, ["photo.png"])
    await page.wait_for_selector(".row")
    await page.wait_for_timeout(500)
    geometry = await page.evaluate(GEOMETRY)
    await page.keyboard.press("Control+,")
    await page.get_by_role("button", name="Customize skin", exact=True).click()
    box = page.get_by_role("textbox", name="Skin JSON code")
    original = json.loads(await box.input_value())
    await page.get_by_role("button", name="Copy code", exact=True).click()
    assert json.loads(await page.evaluate("navigator.clipboard.readText()")) == original
    await page.get_by_role("button", name="Copy LLM prompt", exact=True).click()
    prompt = await page.evaluate("navigator.clipboard.readText()")
    assert "Do not add CSS" in prompt and json.dumps(original, indent=2) in prompt

    invalid = dict(original, layout={"display": "none"})
    await box.fill(json.dumps(invalid))
    await page.get_by_role("button", name="Preview skin", exact=True).click()
    assert "unknown field" in await page.locator(".skin [role=alert]").inner_text()
    assert await page.evaluate("(key) => localStorage.getItem(key)", KEY) is None

    skin = json.loads(json.dumps(original))
    skin["name"] = "Forest"
    for palette in skin["colors"].values():
        palette["accent"] = "#27856f"
        palette["bg"] = "#e9f1ed" if scheme == "light" else "#111c17"
    skin["fonts"]["sans"] = ["Arial", "sans-serif"]
    skin["fonts"]["serif"] = ["Georgia", "serif"]
    await box.fill("```json\n" + json.dumps(skin, indent=2) + "\n```")
    await page.get_by_role("button", name="Preview skin", exact=True).click()
    assert await page.evaluate("getComputedStyle(document.documentElement).getPropertyValue('--accent')") == "#27856f"
    assert await page.locator(".skin").evaluate("e => getComputedStyle(e).color") == "rgb(23, 22, 26)"
    assert await page.evaluate("(key) => localStorage.getItem(key)", KEY) is None
    assert await page.evaluate(GEOMETRY) == geometry, "queue geometry changed"
    assert await page.evaluate("window.__ceMockSaves.length") == 0
    await page.get_by_role("button", name="Keep skin", exact=True).click()
    assert json.loads(await page.evaluate("(key) => localStorage.getItem(key)", KEY)) == skin
    shots = ROOT / "shots"
    shots.mkdir(exist_ok=True)
    await page.screenshot(path=str(shots / f"skin-{width}-{scheme}.png"))
    assert await page.evaluate("document.body.scrollWidth === document.body.clientWidth")
    await page.get_by_role("button", name="Close settings", exact=True).click()
    await page.get_by_role("button", name="Convert", exact=True).click()
    await page.wait_for_selector('.row[data-status="done"]', timeout=30000)
    assert not errors, errors

    await page.reload(wait_until="networkidle")
    assert await page.evaluate("getComputedStyle(document.documentElement).getPropertyValue('--accent')") == "#27856f"
    await page.keyboard.press("Control+Shift+,")
    await box.wait_for()
    assert json.loads(await box.input_value()) == skin
    await box.fill(json.dumps(original))
    await page.get_by_role("button", name="Preview skin", exact=True).click()
    await page.get_by_role("button", name="Revert", exact=True).click()
    assert await page.evaluate("getComputedStyle(document.documentElement).getPropertyValue('--accent')") == "#27856f"
    await page.get_by_role("button", name="Preview skin", exact=True).click()
    await page.keyboard.press("Escape")
    assert await page.evaluate("getComputedStyle(document.documentElement).getPropertyValue('--accent')") == "#27856f"
    await page.keyboard.press("Control+Shift+,")
    await page.get_by_role("button", name="Reset skin", exact=True).click()
    assert await page.evaluate("(key) => localStorage.getItem(key)", KEY) is None
    await page.reload(wait_until="networkidle")
    assert await page.evaluate("document.documentElement.style.getPropertyValue('--accent')") == ""
    await page.context.close()
    print(f"PASS {width}px {scheme}: copy, LLM prompt, validation, preview, keep, reload, revert, reset, layout, conversion")


async def main():
    async with async_playwright() as p:
        browser = await p.chromium.launch()
        for width, scheme in [(960, "light"), (720, "dark")]:
            await check(browser, width, scheme)
        await browser.close()


if __name__ == "__main__":
    asyncio.run(main())
