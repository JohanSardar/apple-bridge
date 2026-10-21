import { batch, createEffect, createSignal, onCleanup } from 'solid-js';
import type { LiveClient } from '../../data/live';
import { withInput } from '../../data/input';
import type { CommandState, Cursor, ThreadItem } from '../../data/types';
import { createCapabilities } from './capabilities';
export type MessageFile = { upload_id: string; name: string };
export type SendMessage = { chat_id: string; text?: string; files?: MessageFile[] };
export type MessageSendState = 'pending' | 'sent' | 'failed' | 'indeterminate';
export type DeliveryNotice = { count: number; command_ids: readonly string[]; text: string };
function messageSendState(command: CommandState): MessageSendState {
    if (command.status === 'indeterminate') return 'indeterminate';
    if (['failed', 'rejected', 'delivery_failed', 'expired'].includes(command.status)) return 'failed';
    return ['observed', 'delivered'].includes(command.status) ? 'sent' : 'pending';
}
export function messageCommandLabel(command: CommandState) {
    return { pending: 'Sending…', sent: 'Sent', failed: 'Not Delivered', indeterminate: 'Not sure if delivered' }[messageSendState(command)];
}
export function createMessageCommands(live: LiveClient) {
    const capabilities = createCapabilities(live);
    const echoes = new Map<string, ThreadItem>(), suppressed = new Set<string>(), resolved = new Set<string>();
    const identities = new Map<string, string>(), displayed = new Map<string, string>(), held = new Map<string, string>();
    const snapshots = new Map<string, { ids: Set<string>; cursor?: Cursor }>();
    const released = new Map<string, Set<string>>(), missingIdentities = new Set<string>();
    const timers = new Map<string, ReturnType<typeof setTimeout>>();
    const [revision, setRevision] = createSignal(0), [timeoutCount, setTimeoutCount] = createSignal(0), [missingIdentityCount, setMissingIdentityCount] = createSignal(0);
    let disposed = false;
    const changed = () => setRevision(value => value + 1);
    const ambiguous = (id: string) => (released.get(id)?.size ?? 0) > 0;
    const hasEcho = (chatID: string, commands: Map<string, CommandState>) => [...echoes].some(([id, item]) => item.data.chat_id === chatID && !suppressed.has(id) && !resolved.has(id) && (!commands.has(id) || messageSendState(commands.get(id)!) !== 'failed'));
    const release = (chatID: string) => {
        clearTimeout(timers.get(chatID)); timers.delete(chatID);
        for (const [id, chat] of held) if (chat === chatID) { held.delete(id); displayed.set(id, chatID); }
    };
    const compatible = (command: CommandState, cursor: Cursor | undefined) => Boolean(command.commit_seq && command.cursor && cursor && cursor.epoch === command.cursor.epoch && cursor.subscription_revision === command.cursor.subscription_revision && cursor.sequence >= command.commit_seq);
    const refresh = () => {
        if (disposed) return;
        const commands = new Map(live.pending().map(command => [command.id, command]));
        for (const [id, item] of echoes) {
            const command = commands.get(id), capture = snapshots.get(String(item.data.chat_id));
            if (!command) continue;
            if (messageSendState(command) === 'sent' && !command.observed_record_ids?.length && !missingIdentities.has(id)) {
                missingIdentities.add(id); setMissingIdentityCount(value => value + 1);
            }
            if (compatible(command, capture?.cursor) && command.observed_record_ids?.length) {
                const captured = capture!.ids;
                if (resolved.has(id) && command.observed_record_ids.every(recordID => !captured.has(recordID))) {
                    for (const recordID of command.observed_record_ids) { identities.delete(recordID); displayed.delete(recordID); }
                    echoes.delete(id); resolved.delete(id); suppressed.delete(id); released.delete(id); missingIdentities.delete(id);
                    continue;
                }
                if (command.observed_record_ids.every(recordID => captured.has(recordID))) {
                    for (const recordID of command.observed_record_ids) { identities.set(recordID, id); held.delete(recordID); displayed.set(recordID, String(item.data.chat_id)); for (const candidates of released.values()) candidates.delete(recordID); }
                    resolved.add(id);
                }
            }
            const state = messageSendState(command);
            if (state === 'failed' || state === 'indeterminate' && !ambiguous(id)) suppressed.delete(id);
        }
        for (const chatID of snapshots.keys()) {
            if (!hasEcho(chatID, commands)) { release(chatID); continue; }
            if (![...held.values()].includes(chatID)) { clearTimeout(timers.get(chatID)); timers.delete(chatID); continue; }
            if (timers.has(chatID)) continue;
            timers.set(chatID, setTimeout(() => batch(() => {
                if (disposed) return;
                const candidates = [...held].filter(([, chat]) => chat === chatID).map(([id]) => id);
                const current = new Map(live.pending().map(command => [command.id, command]));
                for (const [id, item] of echoes) if (item.data.chat_id === chatID && !resolved.has(id) && (!current.has(id) || messageSendState(current.get(id)!) !== 'failed')) {
                    released.set(id, new Set([...(released.get(id) ?? []), ...candidates])); suppressed.add(id);
                }
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
        const commands = new Map(live.pending().map(command => [command.id, command]));
        for (const [chatID, items] of chats) {
            const current = new Set(items.map(item => item.id));
            if (cursor) {
                for (const [id, chat] of displayed) if (chat === chatID && !current.has(id)) displayed.delete(id);
                for (const [id, chat] of held) if (chat === chatID && !current.has(id)) held.delete(id);
            }
            snapshots.set(chatID, { ids: current, cursor });
            const holding = hasEcho(chatID, commands);
            for (const item of items) {
                if (!holding || displayed.has(item.id) || identities.has(item.id) || !item.data.is_from_me) { displayed.set(item.id, chatID); held.delete(item.id); }
                else held.set(item.id, chatID);
            }
        }
        refresh();
    };
    const projectCaptured = (items: readonly ThreadItem[], _cursor: Cursor | undefined): readonly ThreadItem[] => {
        revision();
        if (!held.size && !identities.size) return items;
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
    const notice = (chatID: string): DeliveryNotice | undefined => {
        revision();
        const commands = statuses().filter(command => echoes.get(command.id)?.data.chat_id === chatID);
        const uncertain = commands.filter(command => command.status === 'indeterminate' && suppressed.has(command.id) && !resolved.has(command.id) && ambiguous(command.id));
        const failedChecks = commands.filter(command => command.check_error);
        const ids = [...new Set([...uncertain, ...failedChecks].map(command => command.id))];
        const text = [uncertain.length ? uncertain.length === 1 ? '1 message may not have been delivered' : uncertain.length + ' messages may not have been delivered' : '',
            failedChecks.length ? failedChecks.length === 1 ? "Couldn't check this message. Try again." : "Couldn't check these messages. Try again." : ''].filter(Boolean).join(' · ');
        return ids.length ? { count: ids.length, command_ids: ids, text } : undefined;
    };
    const copy = (event: Event, id: string) => withInput(event, () => {
        const item = echoes.get(id);
        if (!item) throw new Error('Message unavailable');
        return navigator.clipboard.writeText(String(item.data.text ?? ''));
    });
    const dispose = () => { disposed = true; for (const timer of timers.values()) clearTimeout(timer); timers.clear(); snapshots.clear(); echoes.clear(); identities.clear(); held.clear(); displayed.clear(); released.clear(); suppressed.clear(); resolved.clear(); missingIdentities.clear(); capabilities.dispose(); };
    onCleanup(dispose);
    return { send, pending, observe, projectCaptured, statuses, notice, timeoutCount, missingIdentityCount, capabilities, check: (id: string) => live.checkCommand(id), copy, dispose };
}
