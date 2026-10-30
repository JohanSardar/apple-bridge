import { expect, type Page, type APIRequestContext } from '@playwright/test';
import { test } from './list-session';
import { control } from './session';
async function parityFixture(request: APIRequestContext) {
    const response = await request.post(process.env.AB_WEB_CONTROL! + '/parity-fixture', {headers:{'X-Bridge-Lab':process.env.AB_WEB_CONTROL_KEY!}});
    expect(response.ok(), await response.text()).toBe(true); return response.json();
}
const operations = ['messages.pin', 'messages.mute', 'messages.set_read_receipts', 'messages.recover_chat', 'messages.delete_chat_permanently'];
async function observe(page: Page) {
    const commands: any[] = [];
    page.on('websocket', socket => socket.on('framesent', frame => {
        const value = JSON.parse(String(frame.payload));
        if (value.t === 'cmd' && operations.includes(value.op)) commands.push(value);
    }));
    return commands;
}
async function menu(page: Page, id: string) {
    await page.locator('[data-chat="' + id + '"]').first().click({ button: 'right' });
    return page.getByRole('dialog', { name: 'Conversation actions', exact: true });
}
async function settings(page: Page, id: string) {
    await page.goto('/messages/' + encodeURIComponent(id));
    await page.getByRole('button', { name: 'Details', exact: true }).click();
    return page.getByRole('region', { name: 'Conversation settings' });
}
test('WEB-MSG-PARITY-UI trusted pin, mute and receipt settings use the captured target and version', async ({ page, request }) => {
    const ids = await parityFixture(request), commands = await observe(page);
    const details = await settings(page, ids['list-chat-000']);
    await expect(details).toBeVisible();
    const pin = details.getByRole('button', { name: 'Pin Conversation', exact: true });
    await pin.evaluate(button => button.dispatchEvent(new MouseEvent('click', { bubbles: true })));
    expect(commands).toHaveLength(0);
    await pin.click();
    await expect(details.getByRole('button', { name: 'Unpin Conversation' })).toBeEnabled();
    await details.getByRole('switch', { name: 'Hide Alerts', exact: true }).click();
    await expect(details.getByRole('switch')).toHaveAttribute('aria-checked', 'true');
    const pinned = page.locator('[data-pin="' + ids['list-chat-000'] + '"]');
    await expect(pinned.getByLabel('Notifications silenced')).toBeVisible();
    await expect(pinned).toHaveAccessibleDescription('Notifications silenced');
    await pinned.locator('.pin-title').evaluate(element => { element.textContent = '[ab-test] A very long pinned conversation name that must truncate'; });
    expect(await pinned.locator('.pin-title').evaluate(element => element.scrollWidth > element.clientWidth)).toBe(true);
    expect(await pinned.locator('.pin-muted').evaluate(element => { const icon = element.getBoundingClientRect(), caption = element.parentElement!.getBoundingClientRect(); return icon.left >= caption.left && icon.right <= caption.right; })).toBe(true);
    await page.screenshot({ path: '../lab/evidence/private/web/t74-pinned.png' });
    const receipts = details.getByRole('checkbox', { name: /Send Read Receipts/ });
    await expect(receipts).toHaveAttribute('aria-checked', 'mixed');
    await receipts.click();
    await expect(receipts).toHaveAttribute('aria-checked', 'true');
    await details.getByRole('button', { name: 'Use Default Read Receipt Setting' }).click();
    await expect(receipts).toHaveAttribute('aria-checked', 'mixed');
    await details.getByRole('button', { name: 'Unpin Conversation' }).click();
    await expect(pin).toBeEnabled();
    expect(commands.map(value => value.op)).toEqual(['messages.pin', 'messages.mute', 'messages.set_read_receipts', 'messages.set_read_receipts', 'messages.pin']);
    expect(commands.every(value => value.args.id === ids['list-chat-000'] && value.expected_version > 0)).toBe(true);
    expect(new Set(commands.map(value => value.expected_version)).size).toBe(commands.length);
});
test('WEB-MSG-PARITY-UI pin drag changes captured order; Escape leaves order unchanged', async ({ page, request }) => {
    await control(request, '/list-pins');
    const ids = await parityFixture(request), commands = await observe(page);
    await page.goto('/messages');
    const first = page.locator('[data-pin="' + ids['list-chat-000'] + '"]'), last = page.locator('[data-pin="' + ids['list-chat-002'] + '"]');
    await expect(first).toBeVisible(); await expect(first).toHaveAttribute('data-reorderable', 'true');
    const a = (await first.boundingBox())!, b = (await last.boundingBox())!;
    await page.mouse.move(a.x + a.width / 2, a.y + a.height / 2); await page.mouse.down();
    await page.mouse.move(b.x + b.width / 2, b.y + b.height / 2, { steps: 8 });
    await page.keyboard.press('Escape'); await page.mouse.up();
    await first.evaluate(element => Promise.all(element.getAnimations().map(animation => animation.finished.catch(() => {}))));
    expect(commands).toHaveLength(0);
    await expect(page).toHaveURL(/\/messages$/u);
    await page.mouse.move(a.x + a.width / 2, a.y + a.height / 2); await page.mouse.down();
    await page.mouse.move(b.x + b.width / 2, b.y + b.height / 2, { steps: 8 }); await page.mouse.up();
    await expect.poll(() => commands.length).toBe(1);
    expect(commands[0].args).toEqual({ id: ids['list-chat-000'], pinned_index: 2 });
    await expect(page.locator('.pinned-conversation').nth(2)).toHaveAttribute('data-pin', ids['list-chat-000']);
    const actions = await menu(page, ids['list-chat-000']);
    await actions.getByRole('button', { name: 'Unpin Conversation' }).click();
    await expect(first).toHaveCount(0);
});
test('WEB-MSG-PARITY-UI Recently Deleted recovery and confirmed purge preserve unrelated conversations', async ({ page, request }) => {
    const ids = await parityFixture(request), commands = await observe(page);
    const deleted = async () => {
        await page.getByRole('button', { name: 'Filters and Edit', exact: true }).click();
        await page.getByRole('menuitemradio', { name: 'Recently Deleted', exact: true }).click();
    };
    await page.goto('/messages'); await deleted();
    await expect(page.getByRole('heading', { name: 'Recently Deleted', exact: true })).toBeVisible();
    await page.screenshot({ path: '../lab/evidence/private/web/t74-deleted.png' });
    let actions = await menu(page, ids['parity-deleted']);
    await actions.getByRole('button', { name: 'Recover Conversation', exact: true }).click();
    await expect(page.locator('[data-chat="' + ids['parity-deleted'] + '"]')).toHaveCount(0);
    await parityFixture(request);
    actions = await menu(page, ids['parity-deleted']);
    await actions.getByRole('button', { name: 'Delete Permanently…', exact: true }).click();
    const confirmation = page.getByRole('dialog', { name: 'Delete Conversation Permanently?', exact: true });
    await expect(confirmation.getByRole('button', { name: 'Cancel', exact: true })).toBeFocused();
    await parityFixture(request);
    await expect(confirmation.getByRole('button', { name: 'Delete Permanently', exact: true })).toBeDisabled();
    await expect(confirmation).toContainText('The conversation changed.');
    await confirmation.getByRole('button', { name: 'Cancel', exact: true }).click();
    expect(commands).toHaveLength(1);
    await actions.getByRole('button', { name: 'Delete Permanently…', exact: true }).click();
    await confirmation.getByRole('button', { name: 'Delete Permanently', exact: true }).click();
    await expect(page.locator('[data-chat="' + ids['parity-deleted'] + '"]')).toHaveCount(0);
    await page.getByRole('button', { name: 'Filters and Edit', exact: true }).click();
    await page.getByRole('menuitemradio', { name: 'All Messages', exact: true }).click();
    await expect(page.locator('[data-chat="' + ids['list-chat-000'] + '"]')).toBeVisible();
    await expect.poll(async () => { const response = await page.request.get('/web/v1/sources'); const body = await response.json(); return body.records.find((record: any) => record.source === 'messages').data.state; }).toBe('ready');
    expect(commands.map(value => value.op)).toEqual(['messages.recover_chat', 'messages.delete_chat_permanently']);
    expect(commands.every(value => value.args.id === ids['parity-deleted'] && value.expected_version > 0)).toBe(true);
});
test('WEB-MSG-PARITY-UI unproven controls stay hidden and uncertain changes only check the original command', async ({ page, request }) => {
    const ids = await parityFixture(request), commands = await observe(page);
    const details = await settings(page, ids['list-chat-000']);
    await expect(details).toBeVisible();
    await control(request, '/capability-unproven');
    await expect(details).toHaveCount(0);
    expect(commands).toHaveLength(0);
    await control(request, '/capability-proven');
    await expect(details).toBeVisible();
    await control(request, '/message-fault?name=ui-interference');
    await details.getByRole('switch', { name: 'Hide Alerts', exact: true }).click();
    await expect(page.locator('.conversation-details .parity-notices')).toContainText('Not sure');
    await expect(details.getByRole('switch')).toBeDisabled();
    await page.locator('.conversation-details').getByRole('button', { name: 'Check Change' }).click();
    expect(commands).toHaveLength(1);
    await expect(details.getByRole('switch')).toHaveAttribute('aria-checked', 'false');
});
test('WEB-MSG-PARITY-UI trailing swipe offers proven Hide Alerts without opening delete confirmation', async ({ page, request }) => {
    const ids = await parityFixture(request), commands = await observe(page);
    await page.goto('/messages');
    const row = page.locator('.chat-swipe-row').filter({ has: page.locator('[data-chat="' + ids['list-chat-000'] + '"]') });
    await expect(row).toBeVisible();
    const box = (await row.boundingBox())!;
    await page.mouse.move(box.x + box.width * .8, box.y + box.height / 2); await page.mouse.down();
    await page.mouse.move(box.x + box.width * .8 - 110, box.y + box.height / 2, { steps: 8 }); await page.mouse.up();
    const mute = row.getByRole('button', { name: 'Hide Alerts', exact: true });
    await expect(mute).toBeEnabled(); await expect(mute).toBeInViewport({ ratio: 1 });
    await mute.click();
    await expect(row.getByLabel('Notifications silenced')).toBeVisible();
    expect(commands).toHaveLength(1); expect(commands[0].op).toBe('messages.mute');
    expect(commands[0].args).toEqual({ id: ids['list-chat-000'], muted: true });
    await expect(page.getByRole('dialog', { name: 'Delete conversation', exact: true })).toHaveCount(0);
});
