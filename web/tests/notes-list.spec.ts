import { test, expect, type Page } from '@playwright/test';
import { randomUUID } from 'node:crypto';
import { bootstrap } from './session';

async function connectNotes(page: Page) {
    await page.goto('/');
    await page.evaluate(() => {
        const socket = new WebSocket(location.origin.replace('http', 'ws') + '/web/v1/live', 'web.1');
        const fixture = { socket, frames: [] as any[] };
        (window as any).__notesList = fixture;
        socket.onopen = () => socket.send(JSON.stringify({ t: 'hello', v: 1 }));
        socket.onmessage = event => fixture.frames.push(JSON.parse(event.data));
    });
    await expect.poll(() => page.evaluate(() => (window as any).__notesList.frames.some((frame: any) => frame.t === 'welcome'))).toBe(true);
}

async function send(page: Page, frame: object) {
    await page.evaluate(value => (window as any).__notesList.socket.send(JSON.stringify(value)), frame);
}

async function command(page: Page, op: string, args: object, expected_version?: number) {
    const id = randomUUID();
    await send(page, { t: 'cmd', id, op, args, expected_version });
    await expect.poll(() => page.evaluate(commandID => (window as any).__notesList.frames.find((frame: any) => frame.t === 'cmd_status' && frame.command?.command?.command_id === commandID && ['observed', 'rejected', 'failed', 'indeterminate'].includes(frame.command.state)), id)).toBeTruthy();
    const result = await page.evaluate(commandID => (window as any).__notesList.frames.find((frame: any) => frame.t === 'cmd_status' && frame.command?.command?.command_id === commandID && frame.command.state === 'observed')?.command, id);
    expect(result).toBeTruthy();
    return result;
}

async function model(page: Page, sub: string) {
    return page.evaluate(id => {
        const frames = (window as any).__notesList.frames;
        for (let index = frames.length - 1; index >= 0; index--) {
            const frame = frames[index];
            if (frame.t === 'snap' && frame.sub === id) return frame.data;
            if (frame.t === 'delta' && frame.sub === id) {
                const replacement = frame.changes.find((change: any) => change.op === 'replace');
                if (replacement) return replacement.data;
            }
        }
        return null;
    }, sub);
}

test('NOTE-LIST-LIVE observes Notes folder, create, edit and delete through the authenticated read model', async ({ page, context, request }) => {
    await bootstrap(context, request);
    await connectNotes(page);
    await send(page, { t: 'sub', id: 'all', view: 'notes.list', args: { sort: 'title', time_zone: 'UTC' } });
    await expect.poll(() => model(page, 'all')).not.toBeNull();
    const folder = await command(page, 'notes.create_folder', { title: '[ab-test] Bridge Test ' + randomUUID() });
    const folderID = folder.observed_record_ids[0];
    await expect.poll(async () => (await model(page, 'all'))?.folders.some((entry: any) => entry.folder.id === folderID)).toBe(true);
    await send(page, { t: 'sub', id: 'folder', view: 'notes.list', args: { folder: folderID } });
    const created = await command(page, 'notes.create', { folder_id: folderID, title: '[ab-test] Live note', plain_text: 'First preview 🧪' });
    const noteID = created.observed_record_ids[0];
    await expect.poll(async () => (await model(page, 'folder'))?.sections.flatMap((section: any) => section.notes).find((item: any) => item.note.id === noteID)?.preview).toBe('First preview 🧪');
    const item = (await model(page, 'folder')).sections.flatMap((section: any) => section.notes).find((entry: any) => entry.note.id === noteID);
    await command(page, 'notes.update', { id: noteID, plain_text: '[ab-test] Live note\nUpdated preview', mode: 'replace' }, item.note.version);
    await expect.poll(async () => (await model(page, 'folder'))?.sections.flatMap((section: any) => section.notes).find((entry: any) => entry.note.id === noteID)?.preview).toBe('Updated preview');
    const updated = (await model(page, 'folder')).sections.flatMap((section: any) => section.notes).find((entry: any) => entry.note.id === noteID);
    await command(page, 'notes.delete', { id: noteID }, updated.note.version);
    await expect.poll(async () => (await model(page, 'folder'))?.count).toBe(0);
    await page.evaluate(() => (window as any).__notesList.socket.close());
});

test('NOTE-LIST-LIVE returns capacity for an oversized Notes view', async ({ page, context, request }) => {
    await bootstrap(context, request);
    await connectNotes(page);
    const folder = await command(page, 'notes.create_folder', { title: '[ab-test] Bridge Test capacity ' + randomUUID() });
    const folderID = folder.observed_record_ids[0];
    for (let index = 0; index < 11; index++) {
        await command(page, 'notes.create', { folder_id: folderID, title: '[ab-test] Capacity ' + index, plain_text: 'x'.repeat(200000) });
    }
    await send(page, { t: 'sub', id: 'large', view: 'notes.list', args: { folder: folderID } });
    await expect.poll(() => page.evaluate(() => (window as any).__notesList.frames.find((frame: any) => frame.t === 'error' && frame.id === 'large')?.code)).toBe('capacity');
    await page.evaluate(() => (window as any).__notesList.socket.close());
});
