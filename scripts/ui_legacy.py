"""Feature-removal regressions, not an emulator for macOS 11 or Windows 10."""
import asyncio
from playwright.async_api import async_playwright
import ui_crop
from ui_behaviour import ERRORS


async def fresh_legacy(browser, width=960, height=660, query=''):
    context = await browser.new_context(viewport={'width': width, 'height': height})
    await context.add_init_script('''
      delete HTMLElement.prototype.inert;
      delete HTMLDialogElement.prototype.showModal;
      delete HTMLDialogElement.prototype.close;
    ''')
    page = await context.new_page()
    ERRORS[page] = []
    page.on('pageerror', lambda error: ERRORS[page].append(str(error)))
    await page.goto('http://127.0.0.1:1420/?' + query, wait_until='networkidle')
    await page.wait_for_selector('.dropzone')
    await page.get_by_role('button', name='Open settings').click()
    assert await page.locator('.titlebar__actions button').first.get_attribute('tabindex') == '-1'
    await page.keyboard.press('Escape')
    await page.wait_for_function("document.querySelector('.drawer').getAttribute('aria-hidden') === 'true'")
    assert await page.get_by_role('button', name='Open settings').evaluate('e => e === document.activeElement')
    assert await page.locator('.drawer button').first.get_attribute('tabindex') == '-1'
    return page


async def main():
    ui_crop.fresh = fresh_legacy
    async with async_playwright() as p:
        for name in ['chromium', 'webkit']:
            browser = await getattr(p, name).launch()
            for size in [(960, 660), (720, 520)]:
                await ui_crop.check(browser, *size)
            await browser.close()
            print(f'PASS {name}: inert and dialog fallbacks loaded before first render')


if __name__ == '__main__':
    asyncio.run(main())
