"""MIDI discovery and audio-only targets in the browser preview (synthesis has Rust tests)."""
import asyncio
from playwright.async_api import async_playwright
from ui_behaviour import fresh, DROP, ERRORS

async def main():
    async with async_playwright() as p:
        for name in ['chromium','webkit']:
            browser=await getattr(p,name).launch()
            page=await fresh(browser)
            await page.evaluate(DROP,['Piano.mid','Melody.midi'])
            await page.wait_for_selector('.row')
            rows=page.locator('.row')
            assert await rows.count()==2
            assert await page.locator('.row[data-supported="false"]').count()==0
            for select in await rows.locator('select').all():
                assert await select.input_value()=='mp3'
                await select.focus()
                assert await select.locator('option[value="wav"]').count()==1
                assert await select.locator('option[value="midi"]').count()==0
            await page.get_by_role('button',name='Convert 2 files',exact=True).click()
            await page.wait_for_function("document.querySelectorAll('.row[data-status=done]').length===2")
            assert not ERRORS[page],ERRORS[page]
            await browser.close()
            print(f'PASS {name}: MID/MIDI input, MP3 default, audio targets, no MIDI output')

if __name__=='__main__':asyncio.run(main())
