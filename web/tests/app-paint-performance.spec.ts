import { test, expect } from '@playwright/test';
import { bootstrap, control } from './session';
import { appInputPerformance } from './app-input-performance';

test('WEB-CAL-READ repeated inspector inputs measure native main-thread paint', async ({ page, context, request }, info) => {
    await bootstrap(context, request);
    await page.goto('/calendar/list/2026-10-20');
    const target = page.getByRole('button', { name: 'Project check-in', exact: true }).first();
    await expect(target).toBeVisible();
    await appInputPerformance(page, info, 'WEB-CAL-READ', target, '[data-event-id]', '.calendar-inspector[open] h3', async () => {
        await page.getByRole('button', { name: 'Close event details' }).click();
        await expect(page.getByRole('dialog', { name: 'Event details' })).toHaveCount(0);
    });
});

test('WEB-REM-ACTIONS repeated read-only details measure native main-thread paint', async ({ page, context, request }, info) => {
    await context.addCookies([(await control(request, '/session')).cookie]);
    await control(request, '/reminders-reset');
    await page.goto('/reminders');
    await page.getByRole('button', { name: 'Travel', exact: true }).click();
    const target = page.getByRole('button', { name: 'Details for Plan a weekend trip', exact: true });
    await expect(target).toBeVisible();
    await appInputPerformance(page, info, 'WEB-REM-ACTIONS', target, 'button[aria-label="Details for Plan a weekend trip"]', info.project.name.endsWith('-phone') ? '.reminder-details[open] h2' : '.reminder-details-panel h2', async () => {
        await page.getByRole('button', { name: 'Close details', exact: true }).click();
        await expect(page.locator('.reminder-details[open],.reminder-details-panel')).toHaveCount(0);
    });
});
