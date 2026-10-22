import { test, expect } from '@playwright/test';
import { bootstrap, control } from './session';

test('WEB-CON-PARITY gated membership, read-only group display and capability loss', async ({ page, context, request }, info) => {
    await bootstrap(context, request);
    const compact = info.project.use.viewport!.width < 1024;
    const fixture = await control(request, '/contacts-parity');
    const name = 'Bridge Test [ab-test web-groups]';
    await page.goto('/contacts/' + fixture.contact);
    const membership = page.getByRole('region', { name: 'Lists for this contact', exact: true });
    const lists = page.getByRole('navigation', { name: 'Contact lists', exact: true });
    await expect(page.getByRole('article', { name: 'Parity Bridge Test', exact: true })).toBeVisible();
    await expect(page.getByText(/contact poster/i)).toHaveCount(0);
    await expect(page.getByRole('button', { name: /^(New|Rename|Delete) list$/ })).toHaveCount(0);
    await membership.getByRole('combobox', { name: 'Add to list', exact: true }).selectOption(fixture.group);
    await membership.getByRole('button', { name: 'Add', exact: true }).click();
    const remove = membership.getByRole('button', { name: 'Remove from ' + name, exact: true });
    await expect(remove).toBeEnabled();
    await remove.click();
    await expect(remove).toHaveCount(0);
    await membership.getByRole('combobox', { name: 'Add to list', exact: true }).selectOption(fixture.group);
    await membership.getByRole('button', { name: 'Add', exact: true }).click();
    await expect(remove).toBeEnabled();
    if (compact) {
        await page.getByRole('button', { name: 'Back to contacts', exact: true }).click();
        await page.getByRole('button', { name: 'Back to lists', exact: true }).click();
    }
    await lists.getByRole('button', { name: /web-groups/ }).click();
    await expect(page.locator('.contact-row')).toHaveCount(1);
    await page.getByRole('button', { name: 'Parity Bridge Test', exact: true }).click();
    await membership.getByRole('button', { name: 'Remove from ' + name, exact: true }).click();
    await expect(page.getByRole('heading', { name, exact: true })).toBeVisible();
    await expect(page.locator('.contact-row')).toHaveCount(0);
    if (compact) await page.getByRole('button', { name: 'Back to lists', exact: true }).click();
    await lists.getByRole('button', { name: /^All Contacts/ }).click();
    await page.getByRole('button', { name: 'Parity Bridge Test', exact: true }).click();
    await expect(page.getByRole('article', { name: 'Parity Bridge Test', exact: true })).toBeVisible();
    await control(request, '/contacts-parity?unavailable=1');
    await expect(membership).toHaveCount(0);
    await expect(page.getByRole('button', { name: /^(New|Rename|Delete) list$/ })).toHaveCount(0);
});
