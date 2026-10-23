import { test, expect } from '@playwright/test';
import { prepareSession, control } from './session';
import { readdir, readFile, mkdir, writeFile } from 'node:fs/promises';
import { gzipSync } from 'node:zlib';
import { execFileSync } from 'node:child_process';
async function metrics(name: string, project: string, value: unknown) { await mkdir('../lab/evidence/web/metrics', { recursive: true }); await writeFile(`../lab/evidence/web/metrics/${name}-${project}.json`, JSON.stringify(value, null, 2) + '\n'); }
function stats(values: number[]) { const sorted = [...values].sort((a, b) => a - b); return { samples: sorted.length, p50_ms: sorted[Math.ceil(sorted.length * .5) - 1] ?? 0, p95_ms: sorted[Math.ceil(sorted.length * .95) - 1] ?? 0, max_ms: sorted.at(-1) ?? 0 }; }
test.beforeEach(async ({ page }) => { await prepareSession(page); });
test('WEB-VIRTUAL-ANCHOR preserves measured anchors through prepend, append and keyboard navigation', async ({ page }, info) => {
    await page.goto('/__fixtures/virtual');
    const list = page.getByRole('listbox');
    await page.getByRole('button', { name: 'Middle', exact: true }).click();
    await expect.poll(() => list.locator('[data-key="50000"]').count()).toBe(1);
    await expect(list).not.toHaveAttribute('aria-activedescendant', /.+/);
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
    await expect.poll(()=>list.evaluate(element=>Boolean(document.getElementById(element.getAttribute('aria-activedescendant')??'')))).toBe(true);
    await page.getByRole('button', { name: 'Bottom mode' }).click();
    await page.getByRole('button', { name: 'Append', exact: true }).click();
    await expect.poll(() => list.evaluate(element => element.scrollHeight - element.scrollTop - element.clientHeight)).toBeLessThan(2);
    const viewport=page.viewportSize()!;
    await page.setViewportSize({width:viewport.width,height:viewport.height-150});
    await expect.poll(()=>list.evaluate(element=>element.scrollHeight-element.scrollTop-element.clientHeight)).toBeLessThan(2);
    await page.setViewportSize(viewport);
    await expect.poll(()=>list.evaluate(element=>element.scrollHeight-element.scrollTop-element.clientHeight)).toBeLessThan(2);
    const frames = await list.evaluate(async (element) => {
        const deltas: number[] = [], work: number[] = [],longTasks:number[]=[];
        const longTaskSupported=PerformanceObserver.supportedEntryTypes.includes('longtask');
        const observer=longTaskSupported?new PerformanceObserver(list=>{for(const entry of list.getEntries())longTasks.push(entry.duration);}):undefined;
        observer?.observe({type:'longtask'});
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
        await new Promise<void>(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>resolve())));
        for(const entry of observer?.takeRecords()??[])longTasks.push(entry.duration);
        observer?.disconnect();
        return { deltas, work, longTasks,longTaskSupported, rendered: element.querySelectorAll('[data-key]').length };
    });
    expect(frames.rendered).toBeLessThan(100);
    if(frames.longTaskSupported)expect(frames.longTasks.filter(value=>value>50)).toEqual([]);
    const timing = stats(frames.deltas), work = stats(frames.work);
    await metrics('WEB-VIRTUAL-ANCHOR', info.project.name, { frame: timing, scroll_assignment: work, dropped_at_120hz: frames.deltas.reduce((sum, value) => sum + Math.max(0, Math.round(value / (1000 / 120)) - 1), 0), dropped_at_60hz: frames.deltas.reduce((sum, value) => sum + Math.max(0, Math.round(value / (1000 / 60)) - 1), 0), rows: 100000, rendered: frames.rendered, anchor_shift_px: after - before.y, refresh_target_hz: 120,refresh_target_met:timing.p95_ms<=1000/120,long_tasks:frames.longTaskSupported?frames.longTasks:null,long_task_observer:frames.longTaskSupported });
    const savedScroll=await list.evaluate(element=>element.scrollTop);
    await page.reload();
    await expect.poll(()=>page.getByRole('listbox').evaluate((element,saved)=>Math.abs(element.scrollTop-saved),savedScroll)).toBeLessThan(2);
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
    await page.request.get('/__mock/signin');
    await expect(page.getByRole('alert')).toContainText('A new device signed in.');
    await page.getByRole('button',{name:'Dismiss',exact:true}).click();
    await expect(page.getByRole('alert')).toHaveCount(0);
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
test('WEB-MOTION interruptible springs, FLIP and reduced-motion crossfades', async ({ page }, info) => {
    await page.addInitScript(()=>{
        const state=window as Window&{__motionRequests:{duration:unknown;easing:unknown}[]};state.__motionRequests=[];
        const animate=Element.prototype.animate;
        Element.prototype.animate=function(...args){const animation=animate.apply(this,args),timing=animation.effect?.getTiming();state.__motionRequests.push({duration:timing?.duration,easing:timing?.easing});return animation;};
    });
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
    await page.evaluate(()=>{(window as Window&{__motionRequests:unknown[]}).__motionRequests.length=0;});
    await page.getByRole('button', { name: 'Animate spring' }).click();
    const requested=await page.evaluate(()=>(window as Window&{__motionRequests:{duration:unknown;easing:unknown}[]}).__motionRequests);
    expect(requested.map(value=>value.duration)).toContain(150);
    await mkdir('../lab/evidence/private/web/motion-observation',{recursive:true});
    await writeFile(`../lab/evidence/private/web/motion-observation/${info.project.name}.json`,JSON.stringify(requested,null,2)+'\n');
});
test('WEB-PERF-SHELL bundle, layout stability, navigation and warm cache budgets', async ({ page }, info) => {
    const release = 'dist-release';
    const build = JSON.parse(await readFile(`${release}/release-build.json`, 'utf8')) as { commit: string };
    expect(build.commit).toBe(execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim());
    const assetFiles = await readdir(`${release}/assets`), files = assetFiles.filter(file => file.endsWith('.js'));
    const sizes = await Promise.all(files.map(async (file) => ({ file, gzip: gzipSync(await readFile(`${release}/assets/` + file)).byteLength })));
    const manifest = JSON.parse(await readFile(`${release}/.vite/manifest.json`, 'utf8')) as Record<string, { file: string; imports?: string[]; dynamicImports?: string[]; isEntry?: boolean; isDynamicEntry?: boolean }>;
    expect(manifest['index.html'].isEntry).toBe(true);
    expect(Object.keys(manifest).filter(key => key.includes('/app/fixtures/'))).toEqual([]);
    expect((await readFile(`${release}/` + manifest['index.html'].file, 'utf8'))).not.toContain('/__fixtures/');
    function closure(root: string, lazy = false, excluded = new Set<string>()) {
        const keys = new Set<string>();
        function include(key: string) { if (keys.has(key) || excluded.has(key)) return; keys.add(key); const chunk = manifest[key]; for (const dependency of [...(chunk.imports ?? []), ...(lazy ? chunk.dynamicImports ?? [] : [])]) include(dependency); }
        include(root); return keys;
    }
    const entryKeys = closure('index.html');
    const chunkFiles = (keys: Set<string>) => new Set([...keys].map(key => manifest[key].file).filter(file => file.endsWith('.js')).map(file => file.slice('assets/'.length)));
    const entryChunks = chunkFiles(entryKeys), gzipBytes = (chunks: Set<string>) => sizes.filter(value => chunks.has(value.file)).reduce((sum, value) => sum + value.gzip, 0);
    const shell = gzipBytes(entryChunks);
    expect(shell).toBeLessThanOrEqual(60 * 1024);
    const routes = Object.entries(manifest).filter(([, chunk]) => chunk.isDynamicEntry).map(([route]) => { const chunks = chunkFiles(closure(route, true, entryKeys)); return { route, gzip_bytes: gzipBytes(chunks), chunks: [...chunks] }; });
    expect(routes.length).toBeGreaterThan(0);
    for (const route of routes) expect(route.gzip_bytes, route.route).toBeLessThanOrEqual(60 * 1024);
    const precacheMatch = (await readFile(`${release}/sw.js`, 'utf8')).match(/const PRECACHE=(\[[^\n]*\]);/u);
    expect(precacheMatch).not.toBeNull();
    const precache = JSON.parse(precacheMatch![1]) as string[];
    const precacheFiles = precache.map(url => url === '/' ? 'index.html' : url.slice(1));
    const compressedBytes = async (paths: string[]) => (await Promise.all(paths.map(async path => gzipSync(await readFile(`${release}/` + path)).byteLength))).reduce((sum, bytes) => sum + bytes, 0);
    const precacheBytes = await compressedBytes(precacheFiles);
    const cacheableBytes = await compressedBytes([...new Set([...precacheFiles, ...assetFiles.map(file => 'assets/' + file)])]);
    expect(cacheableBytes).toBeLessThanOrEqual(600 * 1024);
    await page.addInitScript(() => {
        const metrics = { cls: 0, layoutShifts: [] as unknown[], cachedContentAt: null as number|null, clsSupported:PerformanceObserver.supportedEntryTypes.includes('layout-shift'), longTasks: [] as number[], frames: [] as number[], clickWork:[] as number[], firstFrame:[] as number[], inputAt:0, longTaskSupported:PerformanceObserver.supportedEntryTypes.includes('longtask') };
        new MutationObserver(()=>{if(metrics.cachedContentAt!==null)return;const content=document.querySelector('[aria-label="Live count"]');if(!content||Number(content.textContent)<=0)return;metrics.cachedContentAt=-1;requestAnimationFrame(()=>requestAnimationFrame(()=>{metrics.cachedContentAt=performance.now();}));}).observe(document,{childList:true,subtree:true,characterData:true});
        const events=new WeakMap<Event,number>();
        window.addEventListener('click',event=>{if(event.isTrusted){metrics.inputAt=performance.now();events.set(event,metrics.inputAt);metrics.frames=[];metrics.longTasks=[];last=0;}},{capture:true});
        window.addEventListener('click',event=>{const start=events.get(event);if(start!==undefined)metrics.clickWork.push(performance.now()-start);});
        let last=0;
        const frame=(time:number)=>{if(last)metrics.frames.push(time-last);else if(metrics.inputAt)metrics.firstFrame.push(Math.max(0,time-metrics.inputAt));last=time;requestAnimationFrame(frame);};
        requestAnimationFrame(frame);
        (window as any).__metrics = metrics;
        if (PerformanceObserver.supportedEntryTypes.includes('layout-shift'))
            new PerformanceObserver(list => { for (const entry of list.getEntries())
                if (!(entry as any).hadRecentInput){
                    metrics.cls += (entry as any).value;metrics.layoutShifts.push({at:entry.startTime,value:(entry as any).value,sources:(entry as any).sources.map((source:any)=>({node:source.node?.nodeName,class:source.node?.className,text:source.node?.textContent?.slice(0,100),previous:source.previousRect,current:source.currentRect}))});} }).observe({ type: 'layout-shift', buffered: true });
        if (PerformanceObserver.supportedEntryTypes.includes('longtask'))
            new PerformanceObserver(list => { for (const entry of list.getEntries())
                metrics.longTasks.push(entry.duration); }).observe({ type: 'longtask', buffered: true });
    });
    await page.goto('/messages');
    await page.getByRole('listbox', { name: 'Conversations' }).waitFor();
    await page.waitForTimeout(200);
    const idleFrames=await page.evaluate(async()=>{for(let i=0;i<180;i++)await new Promise<void>(resolve=>requestAnimationFrame(()=>resolve()));return (window as any).__metrics.frames.slice(-179) as number[];});
    await page.locator('.conversation-row').first().click();
    await expect(page.locator('main.transcript')).toBeVisible();
    const navigation = await page.evaluate(async()=>{for(let i=0;i<180;i++)await new Promise<void>(resolve=>requestAnimationFrame(()=>resolve()));return (window as any).__metrics;});
    await metrics('WEB-PERF-NAVIGATION',info.project.name,{cls:navigation.cls,layout_shifts:navigation.layoutShifts,idle_frames:stats(idleFrames),navigation_frames:stats(navigation.frames),click_dispatch:stats(navigation.clickWork),input_to_first_frame:stats(navigation.firstFrame),long_tasks:navigation.longTaskSupported?navigation.longTasks:null,long_task_observer:navigation.longTaskSupported});
    if(navigation.clsSupported)expect(navigation.cls).toBe(0);
    if(navigation.longTaskSupported)expect(navigation.longTasks.filter((value:number)=>value>50)).toEqual([]);
    await page.goto('/__fixtures/live');
    await expect(page.getByRole('status', {name:'Connection'})).toHaveText('connected');
    await expect.poll(() => page.evaluate(async () => { const db = await new Promise<IDBDatabase>((resolve, reject) => { const request = indexedDB.open('ab.views'); request.onsuccess = () => resolve(request.result); request.onerror = () => reject(request.error); }); const count = await new Promise<number>(resolve => { const request = db.transaction('views').objectStore('views').count(); request.onsuccess = () => resolve(request.result); }); db.close(); return count; })).toBeGreaterThan(0);
    const gateway=process.env.AB_WEB_SERVER==='gateway';
    const label=gateway?'Unread count':'Live count';
    const initial = Number(await page.getByLabel(label,{exact:true}).textContent());
    if(gateway)await control(page.request,'/incoming');else await page.request.get('/__mock/delta');
    await expect(page.getByLabel(label,{exact:true})).toHaveText(String(initial+1));
    const latest=gateway?await page.getByLabel('Latest message',{exact:true}).textContent():null;
    await expect.poll(()=>page.evaluate(async(expected)=>{const db=await new Promise<IDBDatabase>(resolve=>{const request=indexedDB.open('ab.views');request.onsuccess=()=>resolve(request.result);});const values=await new Promise<any[]>(resolve=>{const request=db.transaction('views').objectStore('views').getAll();request.onsuccess=()=>resolve(request.result);});db.close();return values.some(value=>expected?value.data.chats?.[0]?.last_message?.id===expected:value.data.chats?.length>0);},latest)).toBe(true);
    await page.routeWebSocket('**/web/v1/live',()=>{});
    const warm: number[] = [], automationWarm:number[]=[],navigationTiming:unknown[]=[];
    for(let i=0;i<3;i++){
        const start=performance.now();
        await page.reload();
        await expect(page.getByLabel(label,{exact:true})).toHaveText(String(initial+1));
        if(latest)await expect(page.getByLabel('Latest message',{exact:true})).toHaveText(latest);
        automationWarm.push(performance.now()-start);
        await expect.poll(()=>page.evaluate(()=>(window as any).__metrics.cachedContentAt??0)).toBeGreaterThan(0);
        const timing=await page.evaluate(()=>({cachedContentAt:(window as any).__metrics.cachedContentAt,navigation:performance.getEntriesByType('navigation')[0]?.toJSON()}));
        warm.push(timing.cachedContentAt);
        navigationTiming.push(timing.navigation);
        await expect(page.getByRole('status',{name:'Connection'})).not.toHaveText('connected');
    }
    const warmStats = stats(warm);
    await metrics('WEB-PERF-SHELL', info.project.name, { shell_gzip_bytes: shell, shell_chunks: [...entryChunks], routes, precache_gzip_bytes: precacheBytes, total_cacheable_gzip_bytes: cacheableBytes, chunks: sizes, cls: navigation.clsSupported?navigation.cls:null, cls_observer:navigation.clsSupported, long_tasks: navigation.longTaskSupported?navigation.longTasks:null, navigation_frames: stats(navigation.frames),idle_frames:stats(idleFrames),click_dispatch:stats(navigation.clickWork),input_to_first_frame:stats(navigation.firstFrame), warm_load: warmStats, warm_endpoint:'navigation start to two animation frames after cached DOM content', automation_round_trip:stats(automationWarm), navigation_timing:navigationTiming, long_task_observer: navigation.longTaskSupported, paint_timing_verified:false,input_to_raf_8ms_met:stats(navigation.firstFrame).p95_ms<=8 });
    expect(warmStats.max_ms).toBeLessThan(1000);
});
