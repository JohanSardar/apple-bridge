import { test } from 'node:test';
import assert from 'node:assert/strict';
import { actionWindowOpen, chatService, conversationService, messageParts, messageService, messageText, nativeParts, partAttachmentID, partText, sender, textService } from './message.ts';
const record = (data: Record<string, unknown>, collection = 'messages.message') => ({ id: 'record-1', collection, source: 'messages', version: 1, deleted: false, data }) as never;
const item = (data: Record<string, unknown>, attachments: unknown[] = []) => ({ ...(record(data) as object), reactions: [], attachments, reply_count: 0 }) as never;
const chat = (service: string) => record({ service }, 'messages.chat');

test('a message keeps its own service inside a chat labeled SMS', () => {
    assert.equal(messageService(record({ service: 'iMessage' }), 'SMS'), 'iMessage');
    assert.equal(messageService(record({ service: 'SMS' }), 'iMessage'), 'SMS');
});

test('a message without a service falls back to the chat label, then iMessage', () => {
    assert.equal(messageService(record({}), 'RCS'), 'RCS');
    assert.equal(messageService(record({}), ''), 'iMessage');
});

test('only SMS and RCS are text services', () => {
    assert.equal(textService('SMS'), true);
    assert.equal(textService('rcs'), true);
    assert.equal(textService('iMessage'), false);
    assert.equal(textService(''), false);
    assert.equal(textService('SMS relay'), false);
});

test('the conversation service follows the latest message, from a thread item or a chat summary', () => {
    assert.equal(conversationService(chat('SMS'), record({ service: 'iMessage' })), 'iMessage');
    assert.equal(conversationService(chat('SMS'), { id: 'm', sent_at: '', is_from_me: true, service: 'iMessage', preview: { kind: 'text' } }), 'iMessage');
    assert.equal(conversationService(chat('SMS'), record({})), 'SMS');
    assert.equal(conversationService(chat('RCS'), null), 'RCS');
    assert.equal(conversationService(undefined, undefined), '');
});

test('edit and undo send follow the deadlines the Helper published', () => {
    const sent = Date.parse('2026-10-26T12:00:00.000Z');
    const message = record({ sent_at: '2026-10-26T12:00:00.000Z', edit_until: '2026-10-26T12:15:00.000Z', unsend_until: '2026-10-26T12:02:00.000Z' });
    assert.equal(actionWindowOpen(message, 'edit', sent), true);
    assert.equal(actionWindowOpen(message, 'unsend', sent + 119999), true);
    assert.equal(actionWindowOpen(message, 'unsend', sent + 120000), false);
    assert.equal(actionWindowOpen(message, 'edit', sent + 899999), true);
    assert.equal(actionWindowOpen(message, 'edit', sent + 900000), false);
    assert.equal(actionWindowOpen(message, 'edit', sent - 1), false);
    assert.equal(actionWindowOpen(record({ sent_at: '2026-10-26T12:00:00.000Z', unsend_until: '2026-10-26T12:02:00.000Z' }), 'edit', sent), false);
    assert.equal(actionWindowOpen(record({ sent_at: '2026-10-26T12:00:00.000Z' }), 'unsend', sent), false);
});

test('chat-level settings read the chat label', () => {
    assert.equal(chatService(chat('SMS')), 'SMS');
    assert.equal(chatService(undefined), '');
});

test('native parts keep valid indexes and fall back to the plain text', () => {
    assert.deepEqual(nativeParts(record({ parts: [{ index: 0, kind: 'text', text: 'a' }, { index: -1, kind: 'text' }, { index: 1.5 }, 'bad', { index: 2, kind: 'attachment', attachment_id: 'att' }] })).map(part => part.index), [0, 2]);
    assert.deepEqual(nativeParts(record({ text: 'hello' })), [{ index: 0, kind: 'text', text: 'hello' }]);
    assert.deepEqual(nativeParts(record({})), []);
    assert.deepEqual(nativeParts(record({ parts: [{ index: -1 }], text: 'ignored' })), []);
});

test('part text and attachment come from the addressed native part', () => {
    const message = record({ text: 'whole', parts: [{ index: 0, kind: 'text' }, { index: 1, kind: 'attachment', attachment_id: 'att-1' }, { index: 2, kind: 'text', text: 'second' }] });
    assert.equal(partText(message, 0), 'whole');
    assert.equal(partText(message, 2), 'second');
    assert.equal(partText(message, 1), undefined);
    assert.equal(partText(message, 9), undefined);
    assert.equal(partAttachmentID(message, 1), 'att-1');
    assert.equal(partAttachmentID(message, 0), undefined);
});

test('message text prefers the joined text, then the parts', () => {
    assert.equal(messageText(record({ text: 'joined', parts: [{ index: 0, text: 'part' }] })), 'joined');
    assert.equal(messageText(record({ parts: [{ index: 0, text: 'one ' }, { index: 1, text: 'two' }] })), 'one two');
    assert.equal(messageText(record({})), '');
});

test('display parts add visible unreferenced attachments and a placeholder', () => {
    const shown = { id: 'att-shown', data: {} }, hidden = { id: 'att-hidden', data: { hidden: true } }, sticker = { id: 'att-sticker', data: { is_sticker: true } };
    assert.deepEqual(messageParts(item({ text: 'hi' }, [shown, hidden, sticker])), [{ index: 0, kind: 'text', text: 'hi' }, { index: 1, kind: 'attachment', attachment_id: 'att-shown' }]);
    assert.deepEqual(messageParts(item({ balloon: { kind: 'link' } })), [{ index: 0, kind: 'balloon' }]);
    assert.deepEqual(messageParts(item({})), [{ index: 0, kind: 'unknown' }]);
});

test('sender reads the sender handle reference', () => {
    assert.equal(sender(record({ sender_handle_id: 'handle-1' })), 'handle-1');
    assert.equal(sender(record({})), '');
});
