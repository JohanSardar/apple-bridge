import { createEffect, createSignal, onCleanup, untrack } from 'solid-js';
import type { LiveClient } from '../../data/live';
import type { ChatSummary, CommandState, Cursor } from '../../data/types';
import { strings as s } from '../../app/strings';
import { withInput } from '../../data/input';

export type ChatAction = 'mark_read' | 'mark_unread' | 'delete_chat';
type Intent = { id: string; chat: ChatSummary; kind: ChatAction; complete: boolean };
const uncertain = (state: CommandState | undefined) => Boolean(state?.transport_lost
    || ['indeterminate', 'delivery_failed'].includes(state?.status ?? '')
    || ['indeterminate', 'delivery_failed'].includes(state?.disposition ?? ''));
const failed = (state: CommandState | undefined) => ['failed', 'rejected', 'expired'].includes(state?.status ?? '');
const observed = (state: CommandState | undefined) => ['observed', 'delivered'].includes(state?.status ?? '');

export function createChatActions(live: LiveClient, allowed: (operation: string) => boolean) {
    const [revision, setRevision] = createSignal(0), [problem, setProblem] = createSignal('');
    const intents = new Map<string, Intent>();
    let captured: readonly ChatSummary[] = [], cursor: Cursor | undefined;
    const unconfirmed = (intent: Intent, command: CommandState | undefined) => observed(command)
        && (!command?.cursor || !command.commit_seq || !command.observed_record_ids?.includes(intent.chat.chat.id)
            || Boolean(cursor && (command.cursor.epoch !== cursor.epoch
                || command.cursor.subscription_revision !== cursor.subscription_revision)));
    const changed = () => setRevision(value => value + 1);
    const state = (id: string) => live.pending().find(value => value.id === id);
    const active = (chatID: string) => [...intents.values()].some(value =>
        value.chat.chat.id === chatID && !value.complete && !failed(state(value.id)));
    const can = (kind: ChatAction, chatID: string) => {
        revision();
        return live.status() === 'connected' && Boolean(cursor) && allowed('messages.' + kind)
            && captured.some(value => value.chat.id === chatID) && !active(chatID);
    };
    const dispatch = (event: Event, chatID: string, kind: ChatAction) => {
        if (!event.isTrusted || !can(kind, chatID)) return;
        const chat = captured.find(value => value.chat.id === chatID)!;
        try {
            return withInput(event, () => {
                const id = live.command('messages.' + kind, { id: chatID }, undefined,
                    kind === 'delete_chat' ? chat.chat.version : undefined);
                for (const [key, value] of intents) if (value.complete) intents.delete(key);
                intents.set(id, { id, chat, kind, complete: false });
                setProblem('');
                changed();
                return id;
            });
        } catch {
            setProblem(s.chatChangeConnection);
        }
    };
    const reconcile = () => {
        let update = false;
        for (const intent of intents.values()) {
            if (intent.complete) continue;
            const command = state(intent.id);
            if (!observed(command) || !command?.cursor || !command.commit_seq || !cursor
                || command.cursor.epoch !== cursor.epoch
                || command.cursor.subscription_revision !== cursor.subscription_revision
                || cursor.sequence < command.commit_seq
                || !command.observed_record_ids?.includes(intent.chat.chat.id)) continue;
            const row = captured.find(value => value.chat.id === intent.chat.chat.id);
            const matches = intent.kind === 'delete_chat' ? !row
                : intent.kind === 'mark_read' ? row?.unread_count === 0
                : Boolean(row && row.unread_count > 0);
            if (matches) {
                intent.complete = true;
                update = true;
            }
        }
        if (update) changed();
    };
    createEffect(() => {
        live.pending();
        revision();
        untrack(reconcile);
    });
    createEffect(() => {
        if (live.status() !== 'connected') {
            cursor = undefined;
            changed();
        }
    });
    const observe = (rows: readonly ChatSummary[], value: Cursor | undefined) => {
        if (!value || live.status() !== 'connected') return;
        captured = rows;
        cursor = value;
        reconcile();
        changed();
    };
    const project = (rows: readonly ChatSummary[]) => {
        revision();
        const pending = new Map<string, Intent>();
        for (const intent of intents.values()) {
            const command = state(intent.id);
            if (!intent.complete && !failed(command) && !uncertain(command)
                && !unconfirmed(intent, command))
                pending.set(intent.chat.chat.id, intent);
        }
        return rows.flatMap(row => {
            const intent = pending.get(row.chat.id);
            if (!intent) return [row];
            if (intent.kind === 'delete_chat') return [];
            return [{ ...row, unread_count: intent.kind === 'mark_read' ? 0 : Math.max(1, row.unread_count) }];
        });
    };
    const notices = () => {
        revision();
        return [...intents.values()].filter(value => !value.complete).map(value => {
            const command = state(value.id);
            return {
                id: value.id,
                text: failed(command) ? s.chatChangeFailed
                    : uncertain(command) || unconfirmed(value, command) ? s.chatChangeUnknown
                    : observed(command) ? s.chatChangeWaiting : s.chatChangeUpdating,
                check: uncertain(command) || observed(command),
                dismiss: failed(command) || uncertain(command) || unconfirmed(value, command)
            };
        });
    };
    onCleanup(() => intents.clear());
    return {
        can, observe, project, problem, notices,
        markRead: (event: Event, id: string) => dispatch(event, id, 'mark_read'),
        markUnread: (event: Event, id: string) => dispatch(event, id, 'mark_unread'),
        deleteChat: (event: Event, id: string) => dispatch(event, id, 'delete_chat'),
        check: (id: string) => live.checkCommand(id),
        dismiss: (id: string) => { intents.delete(id); changed(); }
    };
}
export type ChatActions = ReturnType<typeof createChatActions>;
