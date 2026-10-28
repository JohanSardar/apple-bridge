import { chromium } from '@playwright/test';
const [origin, query] = process.argv.slice(2);
const browser = await chromium.launch({ headless: true });
try {
    const context = await browser.newContext();
    await context.addCookies([{ name: 'ab_session_local', value: process.env.AB_GRANTS_SESSION, url: origin }]);
    const page = await context.newPage();
    page.on('pageerror', error => console.error(error.message));
    page.on('response', response => { if (response.status() >= 400) console.error(response.status(), new URL(response.url()).pathname); });
    await page.goto(origin + '/oauth/authorize?' + query);
    await page.getByRole('switch', { name: 'Lab dummy', exact: true }).waitFor();
    const group = page.locator('fieldset').filter({ has: page.getByText('Lab dummy', { exact: true }) });
    if (!(await group.textContent()).includes('labdummy:write')) throw new Error('Missing consent grants');
    await page.goto(origin + '/settings/agents');
    await page.getByRole('button', { name: 'Connect an agent', exact: true }).click();
    await page.getByRole('switch', { name: 'Lab dummy', exact: true }).waitFor();
    await page.getByRole('switch', { name: 'Messages', exact: true }).click();
    await page.getByRole('switch', { name: 'Lab dummy', exact: true }).click();
    await page.getByLabel('Name', { exact: true }).fill('Bridge Test source grants');
    const created = page.waitForResponse(response => response.url().endsWith('/web/v1/agents') && response.request().method() === 'POST');
    await page.getByRole('button', { name: 'Connect an agent', exact: true }).click();
    const response = await created;
    if (response.status() !== 201 || !response.request().postDataJSON().scopes.includes('labdummy:write')) throw new Error('Missing dummy agent grant');
    console.log('source grants rendered');
} finally { await browser.close(); }
