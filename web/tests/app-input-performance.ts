import { expect, type Page, type Locator, type TestInfo } from '@playwright/test';
import { mkdir, writeFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { startTrace, finishTrace, traceWork, resultFrames, stats } from './list-performance';
import { timingFloor } from './timing-floor';

export async function appInputPerformance(page: Page, info: TestInfo, scenario: string, target: Locator, inputSelector: string, resultSelector: string, close: () => Promise<void>) {
    await page.evaluate(({resultSelector, inputSelector}) => {
        const actions: { at: number; captured: number; processed: number; result: boolean }[] = [];
        (window as any).__appInputs = actions;
        const capture = () => {
            const action = actions.at(-1);
            if (!action || action.result || !document.querySelector(resultSelector)) return;
            action.result = true;
            performance.mark('list-input-result-' + (actions.length - 1));
        };
        const observer = new MutationObserver(capture);
        observer.observe(document.body, { subtree: true, childList: true, attributes: true });
        const selected = (event: MouseEvent) => event.isTrusted && event.target instanceof Element && event.target.closest(inputSelector);
        const begin = (event: MouseEvent) => {
            if (!selected(event)) return;
            const index = actions.length;
            actions.push({ at: event.timeStamp, captured: performance.now(), processed: 0, result: false });
            performance.mark('list-input-event-' + index);
            requestAnimationFrame(() => {
                capture(); performance.mark('list-input-frame-start-' + index);
                requestAnimationFrame(() => performance.mark('list-input-frame-end-' + index));
            });
        };
        const end = (event: MouseEvent) => { if (selected(event)) actions.at(-1)!.processed = performance.now(); };
        window.addEventListener('click', begin, true); window.addEventListener('click', end);
        (window as any).__stopAppInputs = () => { observer.disconnect(); window.removeEventListener('click', begin, true); window.removeEventListener('click', end); };
    }, {resultSelector, inputSelector});
    const session = await startTrace(page);
    for (let index = 0; index < 24; index++) {
        await target.click();
        await expect(page.locator(resultSelector)).toBeVisible();
        await page.evaluate(() => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
        await close();
    }
    await page.evaluate(() => (window as any).__stopAppInputs());
    const raw = await finishTrace(session, true), actions = await page.evaluate(() => (window as any).__appInputs);
    await mkdir('../lab/evidence/private/web', { recursive: true });
    if (raw) await writeFile(`../lab/evidence/private/web/${scenario}-paint-${info.project.name}.json`, raw);
    await writeFile(`../lab/evidence/private/web/${scenario}-inputs-${info.project.name}.json`, JSON.stringify(actions));
    const trace = raw ? traceWork(raw, 'action', true) : undefined;
    const dispatch = trace ? traceWork(trace.trace, 'action') : undefined;
    const frames = trace ? resultFrames(trace.trace).frames : undefined;
    const processing = actions.map((action: any, index: number) => Math.max(0, action.captured - action.at) + (dispatch?.dispatch_processing_ms[index] ?? action.processed - action.captured));
    const metrics = { actions: actions.length, results: actions.filter((action: any) => action.result).length, processing: stats(processing), native_result_frame_work: frames ? stats(frames) : null, processing_and_result_frame: frames ? stats(processing.map((value: number, index: number) => value + frames[index])) : null, native_main_thread_paint_observed: Boolean(trace && trace.script_style_layout_ms.paint > 0), timing_floor: await timingFloor(page) };
    await mkdir('../lab/evidence/web/metrics', { recursive: true }); await mkdir('../lab/evidence/private/web', { recursive: true });
    await writeFile(`../lab/evidence/web/metrics/${scenario}-paint-${info.project.name}.json`, JSON.stringify({ ...metrics, digest: createHash('sha256').update(trace?.trace ?? JSON.stringify(metrics)).digest('hex') }, null, 2) + '\n');
    if (trace) await writeFile(`../lab/evidence/private/web/${scenario}-paint-${info.project.name}.json`, trace.trace);
    expect(metrics.actions).toBe(24); expect(metrics.results).toBe(24);
    if (trace) { expect(frames).toHaveLength(24); expect(metrics.native_main_thread_paint_observed).toBe(true); }
    expect(metrics.processing.p95_ms).toBeLessThanOrEqual(8);
    if (metrics.processing_and_result_frame) expect(metrics.processing_and_result_frame.p95_ms).toBeLessThanOrEqual(8);
}
