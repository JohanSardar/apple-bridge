import { test, expect } from '@playwright/test';
import { bootstrap } from './session';
test('WEB-REM-PARITY captured parity stays readable and unproven mutations stay hidden', async ({ page, context, request }) => {
  const commands: string[] = [];
  page.on('websocket', socket => socket.on('framesent', event => {
    const frame = JSON.parse(String(event.payload));
    if (frame.t === 'cmd') commands.push(frame.op);
  }));
  await bootstrap(context, request);
  await page.goto('/reminders');
  await expect(page.getByRole('button', { name: 'Travel', exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Flagged', exact: true }).locator('.reminder-count')).toHaveText('2');
  await expect(page.getByRole('button', { name: 'Flagged errands', exact: true })).toBeDisabled();
  await page.getByRole('button', { name: 'Travel', exact: true }).click();
  await expect(page.locator('.reminder-row')).not.toHaveCount(0);
  await expect(page.getByRole('img', { name: 'Flagged', exact: true })).not.toHaveCount(0);
  await expect(page.locator('.reminder-tag')).not.toHaveCount(0);
  const first = page.locator('.reminder-row').first();
  await first.getByRole('button', { name: /^Details for / }).click();
  await expect(page.getByLabel('Reminder details', { exact: true })).toBeVisible();
  for (const name of [/^(?:Flag|Unflag)(?:\s|$)/i, /^Add Subtask$/i, /^Indent$/i, /^Outdent$/i, /^(?:Add|New) Section$/i, /^Move to Section$/i, /^Add Tag$/i, /^Add (?:Photo|Image)$/i, /^Create Smart List$/i, /^(?:Add|New) Group$/i, /^Move (?:Up|Down)$/i, /^(?:Change|Choose) (?:Icon|Emoji)$/i])
    await expect(page.getByRole('button', { name })).toHaveCount(0);
  for (const name of ['Flag', 'Tags', 'List icon', 'List emoji', 'List group']) {
    await expect(page.getByRole('textbox', { name, exact: true })).toHaveCount(0);
    await expect(page.getByRole('checkbox', { name, exact: true })).toHaveCount(0);
    await expect(page.getByRole('combobox', { name, exact: true })).toHaveCount(0);
  }
  await expect(page.locator('.reminders-app [draggable="true"]')).toHaveCount(0);
  await expect(page.locator('.reminders-app input[type="file"]')).toHaveCount(0);
  await page.keyboard.press('Meta+Shift+f');
  expect(commands).toEqual([]);
});
