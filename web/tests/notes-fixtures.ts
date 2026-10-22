import type { Page } from '@playwright/test';
import type { BridgeRecord } from '../src/data/types';
const record = (id: string, collection: string, data: Record<string, unknown>): BridgeRecord => ({ id, collection, source: 'notes', version: 1, data });
export async function notesFixture(page: Page, writable = true, state = 'ready') {
    const operations = ['notes.create', 'notes.update', 'notes.delete', 'notes.create_folder'];
    const source = record('src_notes', 'system.source', { source: 'notes', state, capabilities: writable ? operations : [], coverage: { operations: operations.map(operation => ({ operation, proven: true, reason: 'synthetic_fixture' })) } });
    const folders = [record('fld_test', 'notes.folder', { title: 'Bridge Test', account: 'test' }), record('fld_child', 'notes.folder', { title: 'Nested', account: 'test', parent_id: 'fld_test' }), record('fld_deleted', 'notes.folder', { title: 'Recently Deleted', account: 'test' })];
    let notes = [
        record('nte_plain', 'notes.note', { title: '[ab-test] Plain', folder_id: 'fld_test', plain_text: '[ab-test] Plain\nA quiet draft', editable: 'plain', content_status: 'complete', modified_at: '2026-10-22T15:00:00Z', created_at: '2026-10-20T12:00:00Z', pinned: true }),
        record('nte_rich', 'notes.note', { title: '[ab-test] Rich', folder_id: 'fld_child', plain_text: 'Read only rich text', editable: 'none', modified_at: '2026-10-21T15:00:00Z', blocks: [{ type: 'title', text: '[ab-test] Rich' }, { type: 'heading', text: 'Reading list', level: 1, collapsible: true, children: [{ type: 'paragraph', text: 'Nested reading detail' }] }, { type: 'checklist_item', text: 'Read a book', checked: true }, { type: 'list_item', list_style: 'numbered', text: 'First item' }, { type: 'list_item', list_style: 'numbered', text: 'Second item' }, { type: 'table', rows: [[{ blocks: [{ type: 'paragraph', text: 'Title' }] }, { blocks: [{ type: 'paragraph', text: 'Author' }] }], [{ blocks: [{ type: 'paragraph', text: 'A book' }] }, { blocks: [{ type: 'paragraph', text: 'Example' }] }]] }, { type: 'paragraph', runs: [{ text: 'Safe link', link: 'https://example.com', bold: true, italic: true, underline: true, strikethrough: true, highlight: 'mint', font_size: 12, font: '.SFMono-Regular', color: '#112233ff' }] }, { type: 'paragraph', runs: [{ text: 'Unsafe link', link: 'javascript:alert(1)' }] }, { type: 'attachment', attachment_id: 'missing-file' }, { type: 'unknown', unsupported: 'future_style' }] }),
        record('nte_locked', 'notes.note', { title: '[ab-test] Locked', folder_id: 'fld_test', locked: true, plain_text: 'SECRET LOCKED BODY', editable: 'plain', blocks: [{ type: 'body', text: 'SECRET BLOCK' }] }),
        record('nte_deleted', 'notes.note', { title: '[ab-test] Deleted', folder_id: 'fld_deleted', plain_text: 'Retained note', editable: 'plain' })
    ];
    const commands: { id: string; op: string; args: Record<string, string>; expected_version?: number }[] = [];
    const sessions: string[] = [];
    let conflict = false, sequence = 1;
    const broadcasts = new Set<() => void>();
    await page.route('**/web/v1/sources', route => route.fulfill({ json: { records: [source] } }));
    await page.routeWebSocket('**/web/v1/live', socket => {
        const server = socket.connectToServer(), subscriptions = new Map<string, { view: string; args: Record<string, string> }>();
        const send = (value: unknown) => socket.send(JSON.stringify(value));
        const cursor = () => ({ epoch: 'notes-fixture', sequence, subscription_revision: 1 });
        const snapshot = (id: string, query: { view: string; args: Record<string, string> }) => {
            if (query.view === 'notes.list') {
                const visible = notes.filter(note => query.args.folder ? note.data.folder_id === query.args.folder : note.data.folder_id !== 'fld_deleted');
                const entries = visible.map(note => ({ note, preview: note.data.locked ? '' : String(note.data.plain_text ?? '').split('\n').slice(1).join(' '), pinned: note.data.pinned ?? null, thumbnail: null, read_only: note.data.folder_id === 'fld_deleted' }));
                const pinned = entries.filter(entry => entry.pinned), unpinned = entries.filter(entry => !entry.pinned);
                send({ t: 'snap', sub: id, cursor: cursor(), data: { cursor: cursor(), sort: query.args.sort || 'modified', count: visible.length, accounts: [{ id: 'test', title: 'Test account', count: notes.length, folder_ids: folders.map(folder => folder.id) }], folders: folders.map(folder => ({ folder, count: notes.filter(note => note.data.folder_id === folder.id).length, account_id: 'test', parent_id: folder.data.parent_id ?? null, read_only: folder.id === 'fld_deleted', recently_deleted: folder.id === 'fld_deleted' })), sections: [...(pinned.length ? [{ key: 'pinned', title: 'Pinned', notes: pinned }] : []), { key: 'today', title: 'Today', notes: unpinned }] } });
            } else send({ t: 'snap', sub: id, cursor: cursor(), data: { records: query.args.id === source.id ? [source] : notes.filter(note => note.id === query.args.id) } });
        };
        const refresh = () => subscriptions.forEach((query, id) => snapshot(id, query)); broadcasts.add(refresh);
        socket.onMessage(data => {
            const frame = JSON.parse(String(data));
            if (frame.t === 'sub' && (frame.view === 'notes.list' || frame.view === 'record' && [source.id, ...notes.map(note => note.id)].includes(frame.args.id))) { subscriptions.set(frame.id, frame); snapshot(frame.id, frame); return; }
            if (frame.t === 'unsub' && subscriptions.delete(frame.id)) return;
            if (frame.t !== 'cmd') { server.send(data); return; }
            commands.push(frame); sequence++;
            if (conflict) {
                notes = notes.map(note => note.id === frame.args.id ? { ...note, version: note.version + 1, data: { ...note.data, plain_text: '[ab-test] External change', title: '[ab-test] External change' } } : note);
                send({ t: 'ack', id: frame.id, status: 'failed', error: { code: 'conflict' } }); refresh(); return;
            }
            let observed = frame.args.id;
            if (frame.op === 'notes.update') notes = notes.map(note => note.id === observed ? { ...note, version: note.version + 1, data: { ...note.data, plain_text: frame.args.plain_text, title: frame.args.plain_text.split('\n')[0] } } : note);
            if (frame.op === 'notes.delete') notes = notes.filter(note => note.id !== observed);
            if (frame.op === 'notes.create') { observed = 'nte_created'; notes.push(record(observed, 'notes.note', { title: frame.args.title, plain_text: frame.args.title + '\n' + frame.args.plain_text, folder_id: frame.args.folder_id, editable: 'plain', content_status: 'complete' })); }
            if (frame.op === 'notes.create_folder') { observed = 'fld_created'; folders.push(record(observed, 'notes.folder', { title: frame.args.title, account: 'test' })); }
            send({ t: 'ack', id: frame.id, status: 'observed', command: { observed_record_ids: [observed], commit_seq: sequence, cursor: cursor() } }); refresh();
        });
        server.onMessage(data => { const frame = JSON.parse(String(data)); if (frame.t === 'welcome') sessions.push(frame.session.kind); socket.send(data); });
    });
    return { commands, sessions, conflict: () => { conflict = true; }, external: () => { sequence++; notes = notes.map(note => note.id === 'nte_plain' ? { ...note, version: note.version + 1, data: { ...note.data, title: '[ab-test] External', plain_text: '[ab-test] External\n' + String(note.data.plain_text ?? '').split('\n').slice(1).join('\n') } } : note); broadcasts.forEach(refresh => refresh()); } };
}
