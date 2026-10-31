import { test, expect } from '@playwright/test';
import { mkdir, writeFile } from 'node:fs/promises';
import { timingFloor } from './timing-floor';
import { stats } from './list-performance';

test('WEB-MSG-PERF WEB-MSG-PERF-INPUT measures browser reporting floors with a minimal trusted control', async ({ page }, info) => {
    await page.setContent('<button type="button">Timing control</button><output>0</output>');
    await page.evaluate(() => {
        const probe = { processing: [] as number[], clicks: [] as number[], paints: [] as number[], paintCountBefore: 0 };
        (window as any).__timingControl = probe;
        if (PerformanceObserver.supportedEntryTypes.includes('event')) new PerformanceObserver(list => {
            for (const entry of list.getEntries()) if (entry.name === 'click') probe.clicks.push(entry.duration);
        }).observe({ type: 'event', durationThreshold: 0 } as PerformanceObserverInit);
        if (PerformanceObserver.supportedEntryTypes.includes('paint')) new PerformanceObserver(list => probe.paints.push(...list.getEntries().map(entry => entry.startTime))).observe({ type: 'paint', buffered: true });
        document.querySelector('button')!.addEventListener('click', event => {
            if (!event.isTrusted) return;
            document.querySelector('output')!.textContent = String(probe.processing.length + 1);
            queueMicrotask(() => probe.processing.push(performance.now() - event.timeStamp));
        });
    });
    await timingFloor(page);
    await page.evaluate(() => { const probe = (window as any).__timingControl; probe.paintCountBefore = probe.paints.length; });
    for (let index = 0; index < 24; index++) await page.getByRole('button', { name: 'Timing control', exact: true }).click();
    const floor = await timingFloor(page);
    const probe = await page.evaluate(() => (window as any).__timingControl);
    const measured = { timing_floor: floor, trusted_clicks: probe.processing.length, processing: stats(probe.processing), reported_clicks: probe.clicks.length, reported_presentation: stats(probe.clicks), document_paints_before: probe.paintCountBefore, document_paints_after: probe.paints.length };
    await mkdir('../lab/evidence/web/metrics', { recursive: true });
    await writeFile(`../lab/evidence/web/metrics/WEB-MSG-PERF-FLOOR-${info.project.name}.json`, JSON.stringify(measured, null, 2) + '\n');
    expect(measured.trusted_clicks).toBe(24);
    expect(measured.document_paints_after).toBe(measured.document_paints_before);
    expect(measured.processing.p95_ms).toBeLessThanOrEqual(8);
});
