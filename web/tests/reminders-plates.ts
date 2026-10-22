import type { BrowserContext } from '@playwright/test';
import { readFile } from 'node:fs/promises';
export async function reminderPlate(context: BrowserContext, project: string) {
    const phone = project.endsWith('-phone'), dark = project.includes('-dark-');
    const panels = [...(phone ? [['home', 'Sidebar and home'], ['tags', 'Sidebar · tags facet']] : []), ['list', 'Travel · three levels of subtasks'], ['sections', 'Groceries · ordered sections'], ['completed', 'Personal · completed styling'], ['details', phone ? 'Read-only details sheet' : 'Read-only details inspector']];
    const images = await Promise.all(panels.map(async ([kind, title]) => `<figure><figcaption>${title}</figcaption><img alt="${title}" src="data:image/png;base64,${(await readFile(`../lab/evidence/private/web/reminders-${kind}-${project}.png`)).toString('base64')}"></figure>`));
    const page = await context.newPage();
    try {
        await page.setViewportSize({ width: phone ? 900 : 2880, height: 900 });
        await page.setContent(`<!doctype html><html lang="en"><meta charset="utf-8"><style>
body{margin:0;padding:28px;background:${dark ? '#161618' : '#f2f2f7'};color:${dark ? '#f5f5f7' : '#171717'};font:18px/1.5 system-ui}h1{font-size:28px;margin:0 0 10px}p{margin:0 0 22px;max-width:1400px}.plates{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:24px}figure{margin:0;padding:16px;background:${dark ? '#242426' : '#fff'};border-radius:18px;min-width:0}figcaption{font-weight:600;margin-bottom:12px}img{display:block;width:100%;height:auto;border-radius:8px}
</style><h1>Reminders · ${dark ? 'Dark' : 'Light'} · ${phone ? 'Phone' : 'Desktop'}</h1><p>Captured from the running synthetic demo world. §2: smart tile grid, pinned and grouped lists, emblems and tags. §3: list colors, ordered sections, recursive nesting and completed styling. §4–§6: read-only details, keyboard focus and shared motion primitives. Writes follow in W5-WRITE.</p><div class="plates">${images.join('')}</div>`, { waitUntil: 'load' });
        await page.screenshot({ path: `../lab/evidence/private/web/reminders-plate-${project}.png`, fullPage: true });
    } finally { await page.close(); }
}
