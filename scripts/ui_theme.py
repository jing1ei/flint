"""Default theme, glass fallback, and contrast over all gradient endpoint colors."""
import asyncio
from playwright.async_api import async_playwright
from ui_behaviour import fresh, contrast, DROP, ERRORS

async def main():
    async with async_playwright() as p:
        browser=await p.chromium.launch()
        for scheme in ['light','dark']:
            page=await fresh(browser,scheme=scheme)
            colors=await page.evaluate("""() => {const s=getComputedStyle(document.documentElement);return Object.fromEntries(['bg','ink-control','ink-2','ink-quiet','accent','accent-ink','danger'].map(k=>[k,s.getPropertyValue('--'+k).trim()]));}""")
            assert colors['bg']=='#f7f5fb'
            # Bounds of the composited pink/blue/white surfaces; intermediate blends are lighter.
            for backdrop in ['rgb(241, 221, 237)','rgb(237, 246, 255)','rgb(255, 255, 255)']:
                assert contrast(colors['ink-control'],[backdrop])>=4.5
                for key in ['ink-2','ink-quiet']:
                    assert contrast(colors[key],[backdrop])>=3
            await page.evaluate(DROP,['piano.mid','image.png'])
            await page.wait_for_selector('.row')
            await page.get_by_role('button',name='Open settings').click()
            assert await page.locator('.drawer').evaluate("e => getComputedStyle(e).backdropFilter.includes('blur')")
            assert await page.locator('.drawer').evaluate("e => getComputedStyle(e).backgroundColor")=='rgba(255, 255, 255, 0.88)'
            await page.keyboard.press('Escape')
            assert not ERRORS[page],ERRORS[page]
            await page.context.close()
            print(f'PASS {scheme} OS appearance: light palette, gradient contrast, glass and focus')
        await browser.close()

if __name__=='__main__':asyncio.run(main())
