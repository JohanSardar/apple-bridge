import { test, expect, type Page } from '@playwright/test';
import { bootstrap } from './session';

const digest = 'a'.repeat(64);
const present = { filename: 'Agenda.txt', blob: { state: 'present', digest } };

async function parityFixture(page: Page, data: Record<string, unknown>) {
  const commands: unknown[] = [];
  await page.routeWebSocket('**/web/v1/live', socket => {
    const server = socket.connectToServer();
    socket.onMessage(raw => {
      if (JSON.parse(String(raw)).t === 'cmd') commands.push(raw);
      server.send(raw);
    });
    server.onMessage(raw => {
      const frame = JSON.parse(String(raw));
      const records = [...(frame.data?.records ?? []), ...(frame.data?.events ?? []).map((entry: any) => entry.event)];
      for (const record of records) {
        if (record.collection === 'calendar.event' && record.data.title === 'Project check-in') Object.assign(record.data, data);
      }
      socket.send(JSON.stringify(frame));
    });
  });
  await page.goto('/calendar/list/2026-10-20');
  await page.getByRole('button', { name: 'Project check-in', exact: true }).first().click();
  const inspector = page.getByRole('dialog', { name: 'Event details' });
  await expect(inspector).toContainText('Project check-in');
  return { inspector, commands };
}

for (const coverage of ['complete', 'partial']) {
  test(`WEB-CAL-PARITY ${coverage} store coverage shows local originals without parity writes`, async ({ page, context, request }) => {
    await bootstrap(context, request);
    const { inspector, commands } = await parityFixture(page, {
      travel_time: 0,
      store_coverage: { travel_time: { status: 'complete' }, attachments: { status: coverage } },
      attachments: [present, { filename: 'Cloud.txt', blob: { state: 'unavailable', digest } }, { filename: 'Invalid.txt', blob: { state: 'present', digest: '../invalid' } }],
      invitation_status: 2,
      owner_participation_status: 3,
      attendees: [{ name: 'Sam Example', status: 2 }],
    });
    await expect(inspector.getByText('Travel time', { exact: true })).toBeVisible();
    await expect(inspector.getByText('None', { exact: true })).toBeVisible();
    await expect(inspector.getByText('Sam Example')).toContainText('Accepted');
    const attachment = inspector.getByRole('link', { name: 'Agenda.txt' });
    await expect(attachment).toHaveAttribute('href', new RegExp(`/web/v1/blob/${digest}\\?record=.+&variant=original$`));
    await expect(attachment).toHaveAttribute('download', 'Agenda.txt');
    await expect(inspector.getByRole('link', { name: 'Cloud.txt' })).toHaveCount(0);
    await expect(inspector.getByRole('link', { name: 'Invalid.txt' })).toHaveCount(0);
    await expect(inspector.getByRole('button', { name: /^(Accept|Maybe|Decline|Add attachment|Remove attachment|Travel time)$/ })).toHaveCount(0);
    await expect(inspector.getByRole('textbox', { name: /travel|attachment/i })).toHaveCount(0);
    await expect(inspector.getByText('Declined', { exact: true })).toHaveCount(0);
    await page.keyboard.press('Escape');
    await expect(inspector).toHaveCount(0);
    expect(commands).toEqual([]);
  });
}

for (const status of ['unsupported', 'missing', 'complete']) {
  test(`WEB-CAL-PARITY ${status} coverage never guesses positive travel units or unavailable attachments`, async ({ page, context, request }) => {
    await bootstrap(context, request);
    const { inspector, commands } = await parityFixture(page, {
      travel_time: status === 'complete' ? 1800 : 0,
      store_coverage: status === 'missing' ? undefined : { travel_time: { status }, attachments: { status } },
      attachments: status === 'complete' ? [{ filename: 'Cloud.txt', blob: { state: 'unavailable', digest } }] : [present],
    });
    await expect(inspector.getByText('Travel time', { exact: true })).toHaveCount(0);
    await expect(inspector.getByRole('heading', { name: 'Attachments', exact: true })).toHaveCount(0);
    await expect(inspector.getByRole('link', { name: 'Agenda.txt' })).toHaveCount(0);
    expect(commands).toEqual([]);
  });
}
