import { batch, createEffect, createSignal, onCleanup } from 'solid-js';
import type { LiveClient } from '../../data/live';
import { withInput } from '../../data/input';
import type { CommandState, Cursor, ThreadItem } from '../../data/types';
import { createCapabilities } from './capabilities';
export type MessageFile = { upload_id: string; name: string };
export type SendMessage = { chat_id: string; text?: string; files?: MessageFile[] };
export type MessageSendState = 'pending' | 'sent' | 'failed' | 'indeterminate';
function messageSendState(command: CommandState): MessageSendState {
    if (command.status === 'indeterminate' || ['observed', 'delivered'].includes(command.status) && !command.observed_record_ids?.length) return 'indeterminate';
    if (['failed', 'rejected', 'delivery_failed', 'expired'].includes(command.status)) return 'failed';
    return ['observed', 'delivered'].includes(command.status) ? 'sent' : 'pending';
}
export function messageCommandLabel(command: CommandState) {
    return { pending: 'Sending…', sent: 'Sent', failed: 'Not Delivered', indeterminate: 'Not sure if delivered' }[messageSendState(command)];
}
export function createMessageCommands(live: LiveClient) {
    const capabilities = createCapabilities(live);
    const echoes = new Map<string, ThreadItem>(), suppressed = new Set<string>(), resolved = new Set<string>();
    const identities = new Map<string, string>(), displayed = new Set<string>(), held = new Map<string, string>();
    const snapshots = new Map<string, { items: readonly ThreadItem[]; cursor?: Cursor }>();
    const timers = new Map<string, ReturnType<typeof setTimeout>>();
    const [revision, setRevision] = createSignal(0), [timeoutCount, setTimeoutCount] = createSignal(0);
    let disposed = false;
    const changed = () => setRevision(value => value + 1);
    const hasEcho = (chatID: string) => [...echoes].some(([id, item]) => item.data.chat_id === chatID && !suppressed.has(id) && !resolved.has(id));
    const release = (chatID: string) => {
        clearTimeout(timers.get(chatID)); timers.delete(chatID);
        for (const [id, chat] of held) if (chat === chatID) { held.delete(id); displayed.add(id); }
    };
    const refresh = () => {
        if (disposed) return;
        const commands = new Map(live.pending().map(command => [command.id, command]));
        for (const [id, item] of echoes) {
            const command = commands.get(id), capture = snapshots.get(String(item.data.chat_id)), cursor = capture?.cursor;
            if (!command?.commit_seq || !command.cursor || !cursor || !command.observed_record_ids?.length) continue;
            if (cursor.epoch !== command.cursor.epoch || cursor.subscription_revision !== command.cursor.subscription_revision || cursor.sequence < command.commit_seq) continue;
            const captured = new Set(capture.items.map(item => item.id));
            if (!command.observed_record_ids.every(recordID => captured.has(recordID))) continue;
            for (const recordID of command.observed_record_ids) identities.set(recordID, id);
            resolved.add(id);
        }
        for (const [chatID, capture] of snapshots) {
            if (!hasEcho(chatID)) { release(chatID); for (const item of capture.items) displayed.add(item.id); continue; }
            for (const item of capture.items) {
                if (displayed.has(item.id) || identities.has(item.id) || !item.data.is_from_me) { displayed.add(item.id); held.delete(item.id); }
                else held.set(item.id, chatID);
            }
            if (![...held.values()].includes(chatID)) { clearTimeout(timers.get(chatID)); timers.delete(chatID); continue; }
            if (timers.has(chatID)) continue;
            timers.set(chatID, setTimeout(() => batch(() => {
                if (disposed) return;
                for (const [id, item] of echoes) if (item.data.chat_id === chatID && !resolved.has(id)) suppressed.add(id);
                release(chatID); setTimeoutCount(value => value + 1); changed();
            }), 500));
        }
        changed();
    };
    createEffect(refresh);
    const observe = (items: readonly ThreadItem[], cursor: Cursor | undefined) => {
        const chats = new Map<string, ThreadItem[]>();
        for (const item of items) {
            const chatID = String(item.data.chat_id ?? '');
            if (!chats.has(chatID)) chats.set(chatID, []);
            chats.get(chatID)!.push(item);
        }
        for (const [chatID, items] of chats) snapshots.set(chatID, { items, cursor });
        refresh();
    };
    const projectCaptured = (items: readonly ThreadItem[], _cursor: Cursor | undefined): readonly ThreadItem[] => {
        revision();
        return items.filter(item => !held.has(item.id)).map(item => {
            const commandID = identities.get(item.id);
            return commandID ? { ...item, data: { ...item.data, command_id: commandID, command_sent_at: echoes.get(commandID)?.data.sent_at } } : item;
        });
    };
    const pending = (): readonly ThreadItem[] => {
        revision();
        const commands = new Map(live.pending().map(command => [command.id, command]));
        return [...echoes].filter(([id]) => !suppressed.has(id) && !resolved.has(id)).map(([id, item]) => {
            const command = commands.get(id);
            return { ...item, data: { ...item.data, send_state: command ? messageSendState(command) : 'pending' } };
        });
    };
    const send = (event: Event, args: SendMessage) => withInput(event, () => {
        if (!capabilities.allowed('messages.send')) throw new Error('Sending is unavailable');
        const id = live.command('messages.send', args), sentAt = new Date().toISOString();
        echoes.set(id, { id: 'command:' + id, collection: 'messages.message', source: 'messages', version: 0,
            data: { command_id: id, command_sent_at: sentAt, chat_id: args.chat_id, text: args.text ?? '', parts: args.text ? [{ index: 0, kind: 'text', text: args.text }] : [], is_from_me: true, sent_at: sentAt, send_state: 'pending' },
            attachments: [], reactions: [], reply_count: 0 });
        refresh();
        return id;
    });
    const statuses = () => live.pending().filter(command => echoes.has(command.id));
    const dispose = () => { disposed = true; for (const timer of timers.values()) clearTimeout(timer); timers.clear(); snapshots.clear(); echoes.clear(); identities.clear(); held.clear(); displayed.clear(); capabilities.dispose(); };
    onCleanup(dispose);
    return { send, pending, observe, projectCaptured, statuses, timeoutCount, capabilities, dispose };
}
