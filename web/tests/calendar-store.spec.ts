import { test, expect } from '@playwright/test';
import { bootstrap } from './session';

test('CAL-STORE-READ Calendar range snapshots use the authenticated live protocol', async ({ page, context, request }) => {
  await bootstrap(context, request);
  await page.goto('/messages');
  const result = await page.evaluate(() => new Promise<any>((resolve, reject) => {
    const socket = new WebSocket(location.origin.replace(/^http/, 'ws') + '/web/v1/live', 'web.1');
    const timer = setTimeout(() => { socket.close(); reject(new Error('Calendar snapshot unavailable')); }, 8000);
    socket.onopen = () => socket.send(JSON.stringify({ t: 'hello', v: 1 }));
    socket.onmessage = event => {
      const frame = JSON.parse(String(event.data));
      if (frame.t === 'welcome') socket.send(JSON.stringify({ t: 'sub', id: 'calendar-store-read', view: 'calendar.range', args: { start: '2026-10-01T00:00:00Z', end: '2026-11-01T00:00:00Z', time_zone: 'America/New_York' } }));
      if (frame.t === 'error') { clearTimeout(timer); socket.close(); reject(new Error(frame.code)); }
      if (frame.t === 'snap' && frame.sub === 'calendar-store-read') { clearTimeout(timer); socket.close(); resolve(frame.data); }
    };
  }));
  expect(result.events.length).toBeGreaterThan(20);
  expect(result.calendars.length).toBeGreaterThanOrEqual(5);
  expect(result.events.every((entry: any) => entry.event.collection === 'calendar.event' && entry.columns > entry.column && entry.overlap_group)).toBe(true);
  expect(result.events.some((entry: any) => entry.event.data.all_day)).toBe(true);
  expect(result.events.some((entry: any) => entry.columns > 1)).toBe(true);
});
