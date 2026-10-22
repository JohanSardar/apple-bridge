import { test, expect } from '@playwright/test';
import { bootstrap } from './session';

test.use({ timezoneId: 'America/Los_Angeles' });
test('WEB-CAL-NAVIGATE localized zones, wheel momentum and compact day selection', async ({ page, context, request }, info) => {
  await bootstrap(context, request);
  await page.goto('/calendar/day/2026-07-01');
  const gutter = page.locator('.calendar-time-header > span');
  await expect(gutter).toHaveText(/PDT|GMT-7/);
  await expect(gutter).toHaveAttribute('title', 'America/Los_Angeles');
  await page.goto('/calendar/day/2026-12-01');
  await expect(gutter).toHaveText(/PST|GMT-8/);
  await page.goto('/calendar/month/2026-10-20');
  const grid = page.getByRole('grid', { name: 'October 2026' });
  await expect(grid).toBeVisible();
  await grid.getByRole('gridcell').nth(15).hover();
  const phone = info.project.name.endsWith('phone');
  await page.mouse.wheel(phone ? 0 : 180, phone ? 180 : 0);
  await expect(page).toHaveURL(/\/calendar\/month\/2026-11-01$/);
  await expect(page.getByRole('grid', { name: 'November 2026' })).toBeVisible();
  await expect.poll(() => page.locator('.calendar-content').evaluate(element => Math.abs(new DOMMatrix(getComputedStyle(element).transform).m41) + Math.abs(new DOMMatrix(getComputedStyle(element).transform).m42))).toBeLessThan(1);
  await page.getByRole('grid', { name: 'November 2026' }).getByRole('gridcell').nth(15).hover();
  await page.mouse.wheel(phone ? 0 : -180, phone ? -180 : 0);
  await expect(page).toHaveURL(/\/calendar\/month\/2026-10-01$/);
  await expect.poll(() => page.locator('.calendar-content').evaluate(element => Math.abs(new DOMMatrix(getComputedStyle(element).transform).m41) + Math.abs(new DOMMatrix(getComputedStyle(element).transform).m42))).toBeLessThan(1);
  await page.getByRole('grid', { name: 'October 2026' }).getByRole('button', { name: 'October 20, 2026', exact: true }).click();
  await expect(page).toHaveURL(phone ? /\/calendar\/month\/2026-10-20$/ : /\/calendar\/day\/2026-10-20$/);
  if (phone) {
    await expect(page.getByRole('region', { name: 'Selected day events' })).toContainText('Project check-in');
    await page.getByRole('button', { name: 'List', exact: true }).click();
    await page.getByRole('button', { name: 'Project check-in', exact: true }).first().click();
    const dialog = page.getByRole('dialog', { name: 'Event details' });
    await expect(dialog).toBeVisible();
    await expect.poll(() => dialog.evaluate(element => Math.round(element.getBoundingClientRect().width))).toBe(await page.evaluate(() => innerWidth));
  } else {
    await expect(page.locator('.calendar-time-day')).toHaveCount(1);
    await expect.poll(() => page.locator('.calendar-content').evaluate(element => Math.abs(new DOMMatrix(getComputedStyle(element).transform).a - 1))).toBeLessThan(.01);
  }
});


test('WEB-CAL-NAVIGATE search stays within capture coverage while paging away', async ({ page, context, request }) => {
  await bootstrap(context, request);
  await page.clock.setFixedTime(new Date('2026-10-22T16:00:00Z'));
  await page.goto('/calendar/year/2030-01-01');
  if (await page.getByRole('button', { name: 'Show calendars' }).getAttribute('aria-expanded') === 'false') await page.getByRole('button', { name: 'Show calendars' }).click();
  await page.getByRole('searchbox', { name: 'Search events' }).fill('Project check-in');
  if (await page.getByRole('button', { name: 'Close calendars' }).isVisible()) await page.getByRole('button', { name: 'Close calendars' }).click();
  await expect(page.getByRole('button', { name: 'Project check-in', exact: true }).first()).toBeVisible();
});
