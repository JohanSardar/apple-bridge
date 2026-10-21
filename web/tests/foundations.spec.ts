import { test, expect } from '@playwright/test';
import { readdir, readFile, mkdir, writeFile } from 'node:fs/promises';
import { gzipSync } from 'node:zlib';
async function metrics(name: string, project: string, value: unknown) { await mkdir('../lab/evidence/web/metrics', { recursive: true }); await writeFile(`../lab/evidence/web/metrics/${name}-${project}.json`, JSON.stringify(value, null, 2) + '\n'); }
function stats(values: number[]) { const sorted = [...values].sort((a, b) => a - b); return { samples: sorted.length, p50_ms: sorted[Math.ceil(sorted.length * .5) - 1] ?? 0, p95_ms: sorted[Math.ceil(sorted.length * .95) - 1] ?? 0, max_ms: sorted.at(-1) ?? 0 }; }
test.beforeEach(async ({ page }) => { await page.request.get('/__mock/session'); });
test('WEB-VIRTUAL-ANCHOR preserves measured anchors through prepend, append and keyboard navigation', async ({ page }, info) => {
    await page.goto('/__fixtures/virtual');
    const list = page.getByRole('listbox');
    await page.getByRole('button', { name: 'Middle', exact: true }).click();
    await expect.poll(() => list.locator('[data-key="50000"]').count()).toBe(1);
    await page.waitForTimeout(150);
    const before = await list.evaluate(element => { const r = element.getBoundingClientRect(); const rows = [...element.querySelectorAll<HTMLElement>('[data-key]')]; const row = rows.find(row => row.getBoundingClientRect().bottom > r.top)!; return { key: row.dataset.key, y: row.getBoundingClientRect().top - r.top }; });
    await page.getByRole('button', { name: 'Prepend 20' }).click();
    await page.waitForTimeout(150);
    const after = await list.locator(`[data-key="${before.key}"]`).evaluate((element) => element.getBoundingClientRect().top - element.closest('.virtual-viewport')!.getBoundingClientRect().top);
    expect(Math.abs(after - before.y)).toBeLessThan(1);
    await page.getByRole('button', { name: 'Append', exact: true }).click();
    expect(await list.locator(`[data-key="${before.key}"]`).evaluate(element => element.getBoundingClientRect().top - element.closest('.virtual-viewport')!.getBoundingClientRect().top)).toBeCloseTo(before.y, 0);
    await list.focus();
    await page.keyboard.press('End');
    await expect(list.locator('[aria-selected="true"]')).toHaveAttribute('data-key', 'appended-100020');
    await page.getByRole('button', { name: 'Bottom mode' }).click();
    await page.getByRole('button', { name: 'Append', exact: true }).click();
    await expect.poll(() => list.evaluate(element => element.scrollHeight - element.scrollTop - element.clientHeight)).toBeLessThan(2);
    const frames = await list.evaluate(async (element) => {
        const deltas: number[] = [], work: number[] = [];
        let previous = 0;
        for (let i = 0; i < 180; i++) {
            const now = await new Promise<number>(resolve => requestAnimationFrame(resolve));
            if (previous)
                deltas.push(now - previous);
            previous = now;
            const start = performance.now();
            element.scrollTop = Math.max(0, element.scrollHeight - element.clientHeight - 50000 + i * 170);
            work.push(performance.now() - start);
        }
        return { deltas, work, rendered: element.querySelectorAll('[data-key]').length };
    });
    expect(frames.rendered).toBeLessThan(100);
    const timing = stats(frames.deltas), work = stats(frames.work);
    await metrics('WEB-VIRTUAL-ANCHOR', info.project.name, { frame: timing, scroll_assignment: work, dropped_at_120hz: frames.deltas.reduce((sum, value) => sum + Math.max(0, Math.round(value / (1000 / 120)) - 1), 0), dropped_at_60hz: frames.deltas.reduce((sum, value) => sum + Math.max(0, Math.round(value / (1000 / 60)) - 1), 0), rows: 100000, rendered: frames.rendered, anchor_shift_px: after - before.y, refresh_target_hz: 120 });
});
test('WEB-CACHE-POLICY public sessions persist nothing and revocation wipes trusted data', async ({ page }) => {
    await page.goto('/__fixtures/live');
    await expect.poll(() => page.evaluate(async () => (await indexedDB.databases()).map(db => db.name).includes('ab.views'))).toBe(true);
    await page.request.get('/__mock/revoke');
    await expect(page.getByRole('heading', { name: 'Sign in with a passkey' })).toBeVisible();
    await expect.poll(() => page.evaluate(async () => (await indexedDB.databases()).filter(db => db.name?.startsWith('ab.')).length)).toBe(0);
    await expect.poll(() => page.evaluate(async () => (await caches.keys()).filter(key => key.startsWith('ab.')).length)).toBe(0);
    await page.request.get('/__mock/session?kind=public');
    await page.goto('/__fixtures/live');
    await expect(page.getByRole('status', {name:'Connection'})).toHaveText('connected');
    await page.goto('/settings/appearance');
    await page.getByLabel('Theme').selectOption('dark');
    const storage = await page.evaluate(async () => ({ databases: await indexedDB.databases(), caches: await caches.keys(), local: Object.keys(localStorage), session: Object.keys(sessionStorage), workers: await navigator.serviceWorker.getRegistrations().then(rows => rows.length) }));
    expect(storage).toEqual({ databases: [], caches: [], local: [], session: [], workers: 0 });
});
test('WEB-LIVE-MOCK receives deltas and resumes after a dropped socket with cached views', async ({ page }, info) => {
    await page.goto('/__fixtures/live');
    await expect(page.getByRole('status', {name:'Connection'})).toHaveText('connected');
    const values: number[] = [];
    for (let i = 0; i < 10; i++) {
        const before = Number(await page.getByLabel('Live count').textContent());
        const start = performance.now();
        await page.request.get('/__mock/delta');
        await expect(page.getByLabel('Live count')).toHaveText(String(before + 1));
        values.push(performance.now() - start);
    }
    const before = await page.getByLabel('Live count').textContent();
    await page.request.get('/__mock/drop');
    await expect(page.getByRole('status', {name:'Connection'})).toHaveText('connected');
    await expect(page.getByLabel('Live count')).toHaveText(before!);
    await metrics('WEB-LIVE-MOCK', info.project.name, { fixture_round_trip: stats(values), server: 'mock', not_real_journal_latency: true });
});
test('WEB-MOTION interruptible springs, FLIP and reduced-motion crossfades', async ({ page }) => {
    await page.goto('/__fixtures/motion');
    await page.getByRole('button', { name: 'Animate spring' }).click();
    await page.waitForTimeout(60);
    const before = await page.locator('.motion-fixture').evaluate(element => getComputedStyle(element).transform);
    await page.getByRole('button', { name: 'Interrupt spring' }).click();
    await expect(page.getByRole('status')).toHaveText('Settled');
    expect(before).not.toBe('none');
    expect(await page.locator('.motion-fixture').evaluate(element => Math.abs(new DOMMatrix(getComputedStyle(element).transform).m41))).toBeLessThan(.01);
    await page.getByRole('button', { name: 'Shared element' }).click();
    await expect.poll(() => page.locator('.motion-fixture').evaluate(element => getComputedStyle(element).transform)).toBe('none');
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await page.getByRole('button', { name: 'Animate spring' }).click();
    expect(await page.locator('.motion-fixture').evaluate(element => element.getAnimations().map(animation => animation.effect?.getTiming().duration))).toEqual([150]);
});
test('WEB-PERF-SHELL bundle, layout stability, navigation and warm cache budgets', async ({ page }, info) => {
    const files = (await readdir('dist/assets')).filter(file => file.endsWith('.js'));
    const sizes = await Promise.all(files.map(async (file) => ({ file, gzip: gzipSync(await readFile('dist/assets/' + file)).byteLength })));
    const shell = sizes.filter(value => !/^Settings-|^AuthPage-|^Fixtures-/.test(value.file)).reduce((sum, value) => sum + value.gzip, 0);
    expect(shell).toBeLessThanOrEqual(60 * 1024);
    await page.addInitScript(() => {
        const metrics = { cls: 0, longTasks: [] as number[], frames: [] as number[] };
        let last=0;
        const frame=(time:number)=>{if(last)metrics.frames.push(time-last);last=time;requestAnimationFrame(frame);};
        requestAnimationFrame(frame);
        (window as any).__metrics = metrics;
        if (PerformanceObserver.supportedEntryTypes.includes('layout-shift'))
            new PerformanceObserver(list => { for (const entry of list.getEntries())
                if (!(entry as any).hadRecentInput)
                    metrics.cls += (entry as any).value; }).observe({ type: 'layout-shift', buffered: true });
        if (PerformanceObserver.supportedEntryTypes.includes('longtask'))
            new PerformanceObserver(list => { for (const entry of list.getEntries())
                metrics.longTasks.push(entry.duration); }).observe({ type: 'longtask', buffered: true });
    });
    await page.goto('/messages');
    await page.getByText('No Messages', { exact: true }).waitFor();
    await page.waitForTimeout(200);
    await page.evaluate(() => { (window as any).__metrics.longTasks = []; (window as any).__metrics.frames=[]; });
    await page.getByRole('button', { name: 'New Message', exact: true }).click();
    await page.getByText('No Conversation Selected', { exact: true }).waitFor();
    const navigation = await page.evaluate(() => (window as any).__metrics);
    expect(navigation.cls).toBe(0);
    expect(navigation.longTasks.filter((value: number) => value > 50)).toEqual([]);
    expect(navigation.frames.filter((value:number)=>value>50)).toEqual([]);
    await page.goto('/__fixtures/live');
    await expect(page.getByRole('status', {name:'Connection'})).toHaveText('connected');
    await expect.poll(() => page.evaluate(async () => { const db = await new Promise<IDBDatabase>((resolve, reject) => { const request = indexedDB.open('ab.views'); request.onsuccess = () => resolve(request.result); request.onerror = () => reject(request.error); }); const count = await new Promise<number>(resolve => { const request = db.transaction('views').objectStore('views').count(); request.onsuccess = () => resolve(request.result); }); db.close(); return count; })).toBeGreaterThan(0);
    const initial = Number(await page.getByLabel('Live count',{exact:true}).textContent());
    await page.request.get('/__mock/delta');
    await expect(page.getByLabel('Live count',{exact:true})).toHaveText(String(initial+1));
    await expect.poll(()=>page.evaluate(async()=>{const db=await new Promise<IDBDatabase>(resolve=>{const request=indexedDB.open('ab.views');request.onsuccess=()=>resolve(request.result);});const values=await new Promise<any[]>(resolve=>{const request=db.transaction('views').objectStore('views').getAll();request.onsuccess=()=>resolve(request.result);});db.close();return values.some(value=>value.data.chats?.length>0);})).toBe(true);
    await page.routeWebSocket('**/web/v1/live',()=>{});
    const warm: number[] = [];
    for(let i=0;i<3;i++){
        const start=performance.now();
        await page.reload();
        await expect(page.getByLabel('Live count',{exact:true})).toHaveText(String(initial+1));
        warm.push(performance.now()-start);
        await expect(page.getByRole('status',{name:'Connection'})).not.toHaveText('connected');
    }
    const warmStats = stats(warm);
    expect(warmStats.max_ms).toBeLessThan(1000);
    await metrics('WEB-PERF-SHELL', info.project.name, { shell_gzip_bytes: shell, chunks: sizes, cls: navigation.cls, long_tasks: navigation.longTasks, navigation_frames: stats(navigation.frames), warm_load: warmStats, long_task_observer: info.project.use.browserName === 'chromium' });
});
