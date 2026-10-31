import type { Page } from '@playwright/test';
import { stats } from './list-performance';

export async function timingFloor(page: Page) {
    return page.evaluate(async () => {
        const started = performance.now();
        let previous = started;
        const ticks: number[] = [];
        while (ticks.length < 16 && performance.now() - started < 100) {
            const next = performance.now();
            if (next > previous) ticks.push(next - previous);
            previous = next;
        }
        const frames: number[] = [];
        for (let index = 0; index < 25; index++) {
            frames.push(await new Promise<number>(resolve => requestAnimationFrame(resolve)));
        }
        return { clock_tick_samples: ticks.length, clock_min_tick_ms: ticks.length ? Math.min(...ticks) : null, frame_intervals_ms: frames.slice(1).map((at, index) => at - frames[index]) };
    }).then(({ frame_intervals_ms, ...clock }) => ({ ...clock, frame_intervals: stats(frame_intervals_ms), event_timing_min_threshold_ms: 16, event_timing_duration_quantum_ms: 8, long_task_threshold_ms: 50 }));
}
