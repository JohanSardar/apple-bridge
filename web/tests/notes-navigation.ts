import { expect, type Page } from '@playwright/test';
export async function showNotesList(page: Page) {
    await expect(page.getByRole('main', { name: 'Notes', exact: true })).toBeVisible();
    const back = page.getByRole('button', { name: '‹ Notes', exact: true });
    if (await back.isVisible()) await back.click();
}
export async function showNotesFolders(page: Page) {
    await showNotesList(page);
    const folders = page.getByRole('button', { name: 'Show folders', exact: true });
    if (await folders.getAttribute('aria-expanded') === 'false') await folders.click();
}
export async function hideNotesFolders(page: Page) {
    const close = page.getByRole('button', { name: 'Close folders', exact: true });
    if (await close.isVisible()) await close.click();
}
