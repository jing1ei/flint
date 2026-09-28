"""Toolbar, dialog focus, touch hints and long-error recovery in the browser preview."""
import argparse
import asyncio
from playwright.async_api import async_playwright
from ui_behaviour import fresh, DROP, ERRORS, opens_chooser


async def check(browser, width, height):
    page = await fresh(browser, width=width, height=height)
    add = page.get_by_role('button', name='Add files', exact=True)
    links = page.get_by_role('button', name='Paste links', exact=True)
    settings = page.get_by_role('button', name='Open settings', exact=True)
    for control in (add, links, settings):
        assert await control.is_visible() and await control.is_enabled()
        bounds = await control.bounding_box()
        assert bounds['x'] >= 0 and bounds['x'] + bounds['width'] <= width
        assert bounds['width'] >= 32 and bounds['height'] >= 32
    assert await opens_chooser(page, lambda: add.click())
    await settings.click()
    await page.get_by_role('button', name='Close settings').wait_for()
    await page.keyboard.press('Escape')
    assert await settings.evaluate('e => e === document.activeElement'), 'Settings must restore its opener'
    await links.click()
    await page.get_by_role('dialog', name='Paste links').wait_for()
    await page.keyboard.press('Escape')
    assert await links.evaluate('e => e === document.activeElement'), 'Links must restore its opener'
    await page.locator('.dropzone').focus()
    await page.keyboard.press('Control+l')
    await page.keyboard.press('Escape')
    assert await page.locator('.dropzone').evaluate('e => e === document.activeElement')
    await page.evaluate(DROP, ['photo.png'])
    await page.wait_for_selector('.row')
    assert await opens_chooser(page, lambda: add.click()), 'adding to a populated queue needs a visible path'
    await links.click()
    await page.get_by_role('dialog', name='Paste links').wait_for()
    await page.get_by_role('textbox').fill('https://youtu.be/dQw4w9WgXcQ')
    await page.get_by_role('button', name='Add link', exact=True).click()
    assert await links.evaluate('e => e === document.activeElement')
    assert await page.locator('.row').count() == 2
    await page.get_by_role('button', name='Clear all', exact=True).click()
    await page.locator('.dropzone').focus()
    await page.keyboard.press('Control+l')
    await page.get_by_role('textbox').fill('https://youtu.be/dQw4w9WgXcQ')
    await page.get_by_role('button', name='Add link', exact=True).click()
    assert await page.evaluate('document.activeElement !== document.body'), 'replaced empty canvas must hand focus to a live control'
    await page.get_by_role('button', name='Convert', exact=True).click()
    await page.wait_for_selector('.row[data-status="done"]')
    await page.evaluate(DROP, ['another.png'])
    await page.wait_for_selector('.row[data-status="queued"]')
    await page.locator('.row[data-status="queued"] .row__remove').click()
    await page.get_by_text('Converted files are ready. Add files to start another batch.', exact=True).wait_for()
    assert await page.get_by_role('button', name='Open folder', exact=True).is_visible()
    # An unbroken backend path must wrap without pushing Dismiss outside the viewport.
    await page.evaluate("""async () => {
      const {useStore} = await import('/src/state/store.ts');
      useStore.setState({error: 'Could not open /' + 'long-folder-'.repeat(90) + '/result.mp4'});
    }""")
    dismiss = page.get_by_role('button', name='Dismiss', exact=True)
    await dismiss.wait_for()
    bounds = await dismiss.bounding_box()
    assert 0 <= bounds['x'] and bounds['x'] + bounds['width'] <= width
    assert 0 <= bounds['y'] and bounds['y'] + bounds['height'] <= height
    assert await page.locator('.toast').evaluate('e => e.scrollWidth <= e.clientWidth + 1')
    await dismiss.click()
    await page.locator('.toast[role=alert]').wait_for(state='hidden')
    assert await page.locator('[role=alert]:visible').count() == 0
    assert await page.locator('.app').evaluate('e => e.scrollLeft === 0'), 'focus must not scroll the entire app sideways'
    for control in (add, links, settings):
        bounds = await control.bounding_box()
        assert bounds['x'] >= 0 and bounds['x'] + bounds['width'] <= width, 'populated toolbar must fit too'
    await page.screenshot(path=f'/tmp/cc-experience-{browser.browser_type.name}-{width}.png')
    assert not ERRORS[page], ERRORS[page]
    await page.context.close()
    print(f'PASS {browser.browser_type.name} {width}x{height}: toolbar, adding, dialogs, focus, long errors')


async def touch_hint(browser):
    context = await browser.new_context(viewport={'width': 390, 'height': 700}, has_touch=True)
    page = await context.new_page()
    await page.goto('http://127.0.0.1:1420/', wait_until='networkidle')
    hint = page.locator('.dropzone__hint')
    assert float(await hint.evaluate('e => getComputedStyle(e).opacity')) > 0
    bounds = await hint.bounding_box()
    assert bounds['x'] >= 0 and bounds['x'] + bounds['width'] <= 390
    await context.close()
    print('PASS touch: empty-state guidance is visible and fits')


async def main(name):
    async with async_playwright() as p:
        browser = await getattr(p, name).launch()
        for size in [(960, 660), (720, 520), (621, 600), (620, 600), (390, 700)]:
            await check(browser, *size)
        await touch_hint(browser)
        await browser.close()


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--browser', choices=['chromium', 'webkit'], default='chromium')
    asyncio.run(main(parser.parse_args().browser))
