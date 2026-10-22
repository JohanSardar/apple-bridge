import { createHash } from 'node:crypto';
import { test, expect } from '@playwright/test';
import { prepareListSession as bootstrap } from './list-session';
import { notesFixture } from './notes-fixtures';
const body = JSON.stringify({ plain_text: '[ab-test] Plain\n' + 'Full Unicode body é 📝\n'.repeat(6000), html: '<p>Never render raw HTML</p>', blocks: [{ type: 'title', text: '[ab-test] Plain' }, { type: 'paragraph', text: 'Verified external content 📝' }] });
const digest = (value: string) => createHash('sha256').update(value).digest('hex');
const reference = (value = body) => ({ digest: digest(value), size: Buffer.byteLength(value), state: 'present' });
const external = { body_status: 'external', body_blob: reference(), preview: 'Large preview', plain_text: 'PARTIAL INLINE BODY', blocks: [{ type: 'paragraph', text: 'PARTIAL BLOCK' }], editable: 'plain' };
test('WEB-NOTE-READ external content waits for verified bytes and stays read-only', async ({ page, context, request }) => {
    await bootstrap(context, request); const fixture = await notesFixture(page); fixture.patch('nte_plain', external);
    let release!: () => void; const wait = new Promise<void>(resolve => { release = resolve; });
    const requests: string[] = [];
    await page.route('**/web/v1/blob/**', async route => { requests.push(route.request().url()); await wait; await route.fulfill({ body, contentType: 'application/octet-stream' }); });
    try {
        await page.goto('/notes'); await expect(page.getByText('Large preview', { exact: true })).toBeVisible(); expect(requests).toHaveLength(0);
        await page.locator('[data-note-id="nte_plain"]').click();
        await expect(page.getByRole('status').filter({ hasText: 'Loading note content…' })).toBeVisible();
        await expect(page.locator('.notes-editor')).not.toContainText('PARTIAL');
        await expect(page.getByRole('textbox', { name: 'Note text', exact: true })).toHaveCount(0);
        release(); await expect(page.getByText('Verified external content 📝', { exact: true })).toBeVisible();
        await expect(page.locator('.notes-editor h1')).toHaveCount(1);
        await expect(page.locator('.notes-editor')).not.toContainText('Never render raw HTML');
        expect(new URL(requests[0]).searchParams.get('record')).toBe('nte_plain');
        expect(new URL(requests[0]).searchParams.get('variant')).toBe('original');
        expect(fixture.commands).toHaveLength(0);
    } finally { release(); }
});
for (const problem of ['digest', 'size', 'schema', 'missing'] as const) test(`WEB-NOTE-READ external body rejects ${problem} and retries without showing partial text`, async ({ page, context, request }) => {
    await bootstrap(context, request); const fixture = await notesFixture(page);
    const invalid = JSON.stringify({ plain_text: 'Invalid content', html: '', blocks: 'wrong' });
    fixture.patch('nte_plain', { ...external, body_blob: problem === 'schema' ? reference(invalid) : reference() });
    let retry = false;
    await page.route('**/web/v1/blob/**', route => route.fulfill({ status: problem === 'missing' && !retry ? 503 : 200, body: retry ? body : problem === 'digest' ? body.replace('Verified', 'Tampered') : problem === 'size' ? body.slice(0, -1) : problem === 'schema' ? invalid : body }));
    await page.goto('/notes/nte_plain');
    await expect(page.getByRole('alert')).toContainText('Note content unavailable.');
    await expect(page.locator('.notes-editor')).not.toContainText(/PARTIAL|Invalid content|Tampered/);
    if (problem !== 'schema') { retry = true; await page.getByRole('button', { name: 'Retry', exact: true }).click(); await expect(page.getByText('Verified external content 📝', { exact: true })).toBeVisible(); }
});
test('WEB-NOTE-READ external responses cannot replace a different selection and locked bodies never fetch', async ({ page, context, request }) => {
    await bootstrap(context, request); const fixture = await notesFixture(page); fixture.patch('nte_plain', external); fixture.patch('nte_locked', external);
    let release!: () => void; const wait = new Promise<void>(resolve => { release = resolve; }); let requests = 0;
    await page.route('**/web/v1/blob/**', async route => { requests++; await wait; await route.fulfill({ body }).catch(() => {}); });
    try {
        await page.goto('/notes/nte_plain'); await expect(page.getByRole('status').filter({ hasText: 'Loading note content…' })).toBeVisible();
        await page.locator('[data-note-id="nte_rich"]').click(); release();
        await expect(page.getByText('Nested reading detail', { exact: true })).toBeVisible();
        await expect(page.getByText('Verified external content 📝', { exact: true })).toHaveCount(0);
        await page.locator('[data-note-id="nte_locked"]').click();
        await expect(page.getByText('This note is locked.', { exact: false })).toBeVisible();
        expect(requests).toBe(1);
    } finally { release(); }
});
