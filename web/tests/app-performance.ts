import { expect, type Page } from '@playwright/test';
import { createHash } from 'node:crypto';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { timingFloor } from './timing-floor';
export async function observeApp(page: Page, root: string) {
    await page.addInitScript(root => {
        const probe = { trustedInputs: 0, events: [] as { name: string, processing: number, presentation: number }[], longTasks: [] as number[], paints: [] as number[], supported: PerformanceObserver.supportedEntryTypes, warmContentAt: null as number | null };
        (window as any).__appPerformance = probe;
        const content = new MutationObserver(() => {
            if (probe.warmContentAt !== null || !document.querySelector('.calendar-inspector[open] h3')) return;
            probe.warmContentAt = -1;
            requestAnimationFrame(() => requestAnimationFrame(() => { probe.warmContentAt = performance.now(); }));
        });
        content.observe(document, { childList: true, subtree: true, attributes: true });
        window.addEventListener('click', event => {
            if (event.isTrusted && event.target instanceof Element && event.target.closest(root)) probe.trustedInputs++;
        }, true);
        if (probe.supported.includes('event')) new PerformanceObserver(list => {
            for (const entry of list.getEntries()) {
                const event = entry as PerformanceEventTiming;
                if (event.target instanceof Element && event.target.closest(root)) probe.events.push({ name: event.name, processing: event.processingEnd - event.startTime, presentation: event.duration });
            }
        }).observe({ type: 'event', durationThreshold: 16 } as PerformanceObserverInit);
        if (probe.supported.includes('longtask')) new PerformanceObserver(list => probe.longTasks.push(...list.getEntries().map(entry => entry.duration))).observe({ type: 'longtask' });
        if (probe.supported.includes('paint')) new PerformanceObserver(list => probe.paints.push(...list.getEntries().map(entry => entry.startTime))).observe({ type: 'paint', buffered: true });
    }, root);
}
export async function recordApp(page: Page, scenario: string, project: string) {
    await page.evaluate(() => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
    const probe = await page.evaluate(() => (window as any).__appPerformance);
    expect(probe.trustedInputs).toBeGreaterThan(0);
    const processing = probe.events.map((value: any) => value.processing).sort((a: number, b: number) => a - b);
    const counts = {
        digest: createHash('sha256').update(JSON.stringify(probe)).digest('hex'),
        trusted_inputs: probe.trustedInputs,
        event_timing_supported: probe.supported.includes('event'),
        event_timing_samples: processing.length,
        reported_clicks: probe.events.filter((value: any) => value.name === 'click').length,
        minimum_reported_duration_ms: probe.events.length ? Math.min(...probe.events.map((value: any) => value.presentation)) : null,
        timing_floor: await timingFloor(page),
        observed_event_processing_budget_pass: processing.length > 0 && processing[Math.ceil(processing.length * .95) - 1] <= 8,
        event_presentation_over_8ms: probe.events.filter((value: any) => value.presentation > 8).length,
        long_tasks_supported: probe.supported.includes('longtask'),
        long_tasks: probe.longTasks.length,
        document_paint_samples: probe.paints.length,
        app_paint_budget_proven: false,
        app_layout_budget_proven: false,
        own_input_budget_proven: false,
    };
    await mkdir('../lab/evidence/web/metrics', { recursive: true });
    await writeFile(`../lab/evidence/web/metrics/${scenario}-${project}.json`, JSON.stringify(counts, null, 2) + '\n');
    if (counts.long_tasks_supported) expect(counts.long_tasks).toBe(0);
}
export async function recordWarm(scenario: string, project: string, passed: boolean) {
    const path = `../lab/evidence/web/metrics/${scenario}-${project}.json`;
    const counts = JSON.parse(await readFile(path, 'utf8'));
    await writeFile(path, JSON.stringify({ ...counts, warm_content_samples: 1, warm_content_budget_pass: passed, cached_content_without_live_pass: true }, null, 2) + '\n');
}
