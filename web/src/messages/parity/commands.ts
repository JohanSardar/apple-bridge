import { createEffect, createSignal, onCleanup } from 'solid-js';
import type { LiveClient } from '../../data/live';
import type { BridgeRecord, CommandState } from '../../data/types';
import { withInput } from '../../data/input';
import { createCapabilities } from '../actions/capabilities';
export type ParityOperation = 'pin' | 'mute' | 'set_read_receipts' | 'recover_chat' | 'delete_chat_permanently';
const operations: readonly ParityOperation[] = ['pin', 'mute', 'set_read_receipts', 'recover_chat', 'delete_chat_permanently'];
const kind = (command: CommandState) => operations.includes(command.op.replace(/^messages\./u, '') as ParityOperation);
const uncertain = (command: CommandState) => command.transport_lost || ['indeterminate', 'delivery_failed'].includes(command.status) || ['indeterminate', 'delivery_failed'].includes(command.disposition ?? '');
const failed = (command: CommandState) => ['failed', 'rejected', 'expired'].includes(command.status) && !uncertain(command);
const terminal = (command: CommandState) => ['observed', 'delivered'].includes(command.status) || failed(command);
function targetID(command: CommandState): string {
    const args = command.args;
    return args && typeof args === 'object' && 'id' in args && typeof args.id === 'string' ? args.id : '';
}
export function capturedMute(record: BridgeRecord): boolean | undefined {
    return typeof record.data.muted === 'boolean' ? record.data.muted : undefined;
}
export function createParityCommands(live: LiveClient, targets: () => readonly string[]) {
    const capabilities = createCapabilities(live);
    const [revision, setRevision] = createSignal(0), [problem, setProblem] = createSignal('');
    const changes = new Map<string, { recordID: string }>();
    const views = new Map<string, ReturnType<LiveClient['subscribe']>>();
    const changed = () => setRevision(value => value + 1);
    const pending = (id: string) => live.pending().find(command => command.id === id);
    const captured = (id: string) => { revision(); const view = views.get(id), data = view?.data(); return live.status() === 'connected' && view?.cursor() ? data?.records?.find(record => record.id === id) : undefined; };
    const permitted = (operation: ParityOperation, record: BridgeRecord | undefined) => {
        if (!record || captured(record.id)?.version !== record.version || record.collection !== 'messages.chat' || record.source !== 'messages' || !Number.isSafeInteger(record.version) || record.version < 1 || !capabilities.allowed('messages.' + operation)) return false;
        const deleted = Boolean(record.data.deleted_at);
        if (deleted !== ['recover_chat', 'delete_chat_permanently'].includes(operation)) return false;
        if (operation === 'mute' && capturedMute(record) === undefined) return false;
        if (operation === 'set_read_receipts' && (record.data.service !== 'iMessage' || !['default', 'on', 'off'].includes(String(record.data.read_receipts)))) return false;
        return true;
    };
    const can = (operation: ParityOperation, record: BridgeRecord | undefined) => {
        revision();
        return changes.size < 16 && permitted(operation, record) && ![...changes.values()].some(change => change.recordID === record!.id) && !live.pending().some(command => kind(command) && targetID(command) === record!.id && !terminal(command));
    };
    const submit = (event: Event, operation: ParityOperation, record: BridgeRecord, args: Record<string, unknown> = {}) => {
        if (!event.isTrusted) return;
        try {
            return withInput(event, () => {
                if (!can(operation, record)) throw new Error('Unavailable');
                let values: Record<string, unknown> = {};
                if (operation === 'pin') {
                    if (!Number.isSafeInteger(args.pinned_index) || Number(args.pinned_index) < -1 || Number(args.pinned_index) > 8) throw new Error('Invalid position');
                    values = { pinned_index: args.pinned_index };
                }
                if (operation === 'mute') {
                    if (typeof args.muted !== 'boolean') throw new Error('Invalid setting');
                    values = { muted: args.muted };
                }
                if (operation === 'set_read_receipts') {
                    if (!['on', 'off', 'default'].includes(String(args.read_receipts))) throw new Error('Invalid setting');
                    values = { read_receipts: args.read_receipts };
                }
                const id = live.command('messages.' + operation, { ...values, id: record.id }, undefined, record.version);
                changes.set(id, { recordID: record.id });
                changed(); setProblem('');
                return id;
            });
        } catch { setProblem('Couldn’t update this conversation.'); }
    };
    createEffect(() => {
        revision();
        const enabled = operations.some(operation => capabilities.allowed('messages.' + operation));
        const wanted = new Set(enabled ? [...targets(), ...[...changes.values()].map(change => change.recordID)].filter(Boolean) : []);
        let modified = false;
        for (const [id, view] of views) if (!wanted.has(id)) { view.dispose(); views.delete(id); modified = true; }
        for (const id of wanted) if (!views.has(id)) {
            views.set(id, live.subscribe('record', { collection: 'messages.chat', id }, { cache: false, resetOnReconnect: true }));
            modified = true;
        }
        if (modified) changed();
    });
    createEffect(() => {
        revision(); live.pending();
        for (const [id, change] of changes) {
            const command = pending(id), capture = views.get(change.recordID);
            if (!command) continue;
            if (failed(command)) {
                changes.delete(id); setProblem('Conversation wasn’t changed.'); changed(); continue;
            }
            const cursor = command.cursor, received = capture?.cursor();
            if (!uncertain(command) && ['observed', 'delivered'].includes(command.status) && cursor && received && command.commit_seq && cursor.epoch === received.epoch && cursor.subscription_revision === received.subscription_revision && received.sequence >= command.commit_seq && command.observed_record_ids?.includes(change.recordID)) {
                changes.delete(id); changed();
            }
        }
    });
    const incompatible = (command: CommandState) => {
        const change = changes.get(command.id), received = change ? views.get(change.recordID)?.cursor() : undefined;
        return Boolean(command.cursor && received && (command.cursor.epoch !== received.epoch || command.cursor.subscription_revision !== received.subscription_revision));
    };
    const notices = () => {
        revision();
        return live.pending().filter(command => kind(command) && (!terminal(command) || changes.has(command.id))).map(command => ({
            id: command.id, recordID: targetID(command), check: uncertain(command) || incompatible(command) || Boolean(command.check_error),
            text: command.check_error ? 'Couldn’t check this change. Try again.' : uncertain(command) || incompatible(command) ? 'Not sure if the conversation changed. Check its status before trying again.' : ['observed', 'delivered'].includes(command.status) ? 'Waiting for the updated conversation…' : 'Updating conversation…'
        }));
    };
    const check = (id: string) => { const command = pending(id); if (command && kind(command)) try { live.checkCommand(id); } catch { setProblem('Couldn’t check this change.'); } };
    onCleanup(() => { for (const view of views.values()) view.dispose(); views.clear(); changes.clear(); });
    return { record: captured, allowed: (operation: ParityOperation) => capabilities.allowed('messages.' + operation), can, submit, notices, check, problem };
}
