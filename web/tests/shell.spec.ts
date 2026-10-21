import { test, expect } from '@playwright/test';
import { mkdir } from 'node:fs/promises';
test.beforeEach(async ({ page }) => { await page.request.get('/__mock/session'); });
test('WEB-SHELL-JOURNEY layouts, deep links, appearance, keyboard and history', async ({ page }, info) => {
    const fontRequests: string[] = [];
    page.on('request',request=>{if(request.url().includes('.woff2'))fontRequests.push(request.url());});
    await page.goto('/messages');
    await expect(page.getByText('No Messages', { exact: true })).toBeVisible();
    await expect(page.getByText('Apple Bridge', { exact: true })).toHaveCount(0);
    await expect(page.getByRole('complementary', { name: 'Details' })).toHaveCount(0);
    if (info.project.use.viewport?.width === 390) {
        await expect(page.getByRole('heading', { name: 'Messages', exact: true })).toBeVisible();
        await expect(page.getByRole('button', { name: 'Filters and Edit' })).toBeVisible();
        await expect(page.getByLabel('Search', { exact: true })).toBeHidden();
        if (info.project.use.colorScheme === 'dark')
            expect(await page.locator('.sidebar').evaluate(element => getComputedStyle(element).backgroundColor)).toBe('rgb(0, 0, 0)');
    }
    else
        await expect(page.getByLabel('Search', { exact: true })).toBeVisible();
    await page.getByRole('button', { name: 'Switch app' }).click();
    await expect(page.getByRole('button', { name: 'Reminders Coming soon' })).toBeDisabled();
    await page.getByRole('button', { name: 'Switch app' }).click();
    await page.getByRole('button', { name: 'New Message', exact: true }).click();
    await expect(page).toHaveURL(/messages\/new/);
    await expect(page.getByText('No Conversation Selected', { exact: true })).toBeVisible();
    if (info.project.use.viewport?.width === 1440) {
        await page.getByRole('button', { name: 'Details', exact: true }).click();
        await expect(page.getByRole('complementary', { name: 'Details' })).toBeVisible();
        await page.getByRole('button', { name: 'Close details', exact: true }).click();
        await expect(page.getByRole('complementary', { name: 'Details' })).toHaveCount(0);
    }
    await page.goBack();
    await expect(page).toHaveURL(/messages$/);
    await page.goForward();
    await expect(page).toHaveURL(/messages\/new/);
    await page.goto('/settings/appearance');
    await page.getByLabel('Theme').selectOption(info.project.use.colorScheme === 'dark' ? 'light' : 'dark');
    await page.reload();
    await expect(page.locator('html')).toHaveAttribute('data-theme', info.project.use.colorScheme === 'dark' ? 'light' : 'dark');
    await page.getByLabel('Theme').selectOption('system');
    await page.goto('/messages');
    await mkdir('../lab/evidence/private/web', { recursive: true });
    if(await page.evaluate(()=>/Mac|iPhone|iPad/.test(navigator.platform)))expect(fontRequests).toEqual([]);
    await page.screenshot({ path: `../lab/evidence/private/web/shell-${info.project.name}.png` });
});
test('WEB-INPUT-GUARD denies synthetic events and delayed commands', async ({ page }) => {
    const frames: string[] = [];
    page.on('websocket', socket => socket.on('framesent', e => { if (String(e.payload).includes('"t":"cmd"'))
        frames.push(String(e.payload)); }));
    await page.goto('/__fixtures/input');
    await expect(page.getByRole('status')).toHaveText('Ready');
    await page.getByRole('button', { name: 'Submit fixture command' }).waitFor();
    await page.getByRole('button', { name: 'Submit fixture command' }).dispatchEvent('click');
    await expect(page.getByRole('status')).toHaveText('Trusted input required');
    expect(frames).toHaveLength(0);
    await page.getByRole('button', { name: 'Submit fixture command' }).click();
    await expect(page.getByRole('status')).toContainText('admitted');
    expect(frames).toHaveLength(1);
    await page.getByRole('button', { name: 'Delayed fixture command' }).click();
    await expect(page.getByRole('status')).toHaveText('Trusted input required');
    expect(frames).toHaveLength(1);
});
