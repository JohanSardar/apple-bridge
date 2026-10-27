import { createEffect, createMemo, createSignal } from 'solid-js';
import type { LiveClient } from '../../data/live';
import type { BridgeRecord, CommandState, Cursor } from '../../data/types';
import { withInput } from '../../data/input';
import type { Handles } from '../store/types';
import { array, text } from '../store/types';
import { chatService } from '../store/message';
import { createCapabilities } from '../actions/capabilities';
import { rawRecipient, recipientKey } from '../compose/recipients';

export type GroupAction = 'rename_group' | 'add_member' | 'remove_member' | 'leave_group';
type Editor = { action: GroupAction; version: number; name: string; handle: string };
type Submission = { id: string; action: GroupAction; version: number };
const observed = (state?: CommandState) => Boolean(state && ['observed', 'delivered'].includes(state.status));
const uncertain = (state?: CommandState) => Boolean(state && (state.transport_lost || state.status === 'indeterminate' || state.disposition === 'indeterminate'));
const failed = (state?: CommandState) => Boolean(state && ['failed', 'rejected', 'expired', 'delivery_failed'].includes(state.status) && !uncertain(state));
const compatible = (a: Cursor, b: Cursor) => a.epoch === b.epoch && a.subscription_revision === b.subscription_revision;
export function createGroupCommands(live: LiveClient, record: () => BridgeRecord | undefined, handles: () => Handles, cursor: () => Cursor | undefined) {
    const capabilities = createCapabilities(live);
    const [editor, setEditor] = createSignal<Editor>();
    const [submission, setSubmission] = createSignal<Submission>();
    const [problem, setProblem] = createSignal('');
    const participants = () => array(record()?.data.participants).filter((id): id is string => typeof id === 'string');
    const isGroup = () => record()?.data.style === 'group' && chatService(record()) === 'iMessage' && record()?.data.left !== true;
    const eligible = (action: GroupAction) => isGroup() && participants().length >= (action === 'remove_member' || action === 'leave_group' ? 3 : 2);
    const allowed = (action: GroupAction) => eligible(action) && capabilities.allowed('messages.' + action);
    const state = createMemo(() => live.pending().find(command => command.id === submission()?.id));
    const busy = () => Boolean(submission() && !failed(state()));
    const address = (id: string) => handles().get(id)?.address ?? '';
    const duplicate = (value: string) => participants().some(id => recipientKey(address(id)) === recipientKey(value));
    const inputProblem = () => {
        const value = editor(), chat = record();
        if (!value || !chat) return '';
        if (chat.version !== value.version) return 'This conversation changed. Cancel and open the action again.';
        if (!allowed(value.action)) return 'This action is no longer available.';
        if (value.action === 'rename_group') {
            if (!value.name.trim()) return 'Enter a group name.';
            if (value.name.trim() === text(chat.data.display_name).trim()) return 'Enter a different group name.';
            if (new TextEncoder().encode(value.name.trim()).length > 65536) return 'This name is too long.';
        }
        if (value.action === 'add_member') {
            if (!rawRecipient(value.handle)) return 'Enter an email address or phone number.';
            if (duplicate(value.handle)) return 'This person is already in the conversation.';
        }
        if (value.action === 'remove_member' && !participants().some(id => address(id) === value.handle)) return 'This person is no longer in the conversation.';
        return '';
    };
    const open = (action: GroupAction, handle = '') => {
        const chat = record();
        if (!chat || busy() || !allowed(action)) return;
        setProblem('');
        setSubmission();
        setEditor({ action, handle, version: chat.version, name: text(chat.data.display_name) });
    };
    const cancel = () => {
        if (busy()) return;
        setEditor();
        setSubmission();
        setProblem('');
    };
    const submit = (event: Event) => {
        if (busy()) return;
        const value = editor(), chat = record();
        if (!value || !chat || inputProblem()) return;
        try {
            withInput(event, () => {
                if (!allowed(value.action) || chat.version !== value.version) throw new Error('Conversation unavailable');
                const args: Record<string, unknown> = { id: chat.id };
                if (value.action === 'rename_group') args.name = value.name.trim();
                if (value.action === 'add_member' || value.action === 'remove_member') args.handle = value.handle.trim();
                const id = live.command('messages.' + value.action, args, undefined, chat.version);
                setSubmission({ id, action: value.action, version: chat.version });
                setProblem('');
            });
        } catch {
            setProblem('Couldn’t submit this change.');
        }
    };
    const message = () => {
        if (problem()) return problem();
        const command = state();
        if (!submission()) return '';
        if (command?.check_error) return 'Couldn’t check this change. Check again.';
        if (uncertain(command)) return 'Not sure if this conversation changed. Check before making another change.';
        if (failed(command)) return 'The change wasn’t completed. Cancel and try again.';
        if (observed(command)) return 'Waiting for the updated conversation…';
        return 'Updating conversation…';
    };
    createEffect(() => {
        const pending = submission(), command = state(), chat = record(), capture = cursor();
        if (!pending || !command || !chat || !capture || !observed(command) || !command.cursor || !command.commit_seq) return;
        if (!command.observed_record_ids?.includes(chat.id) || !compatible(command.cursor, capture) || capture.sequence < command.commit_seq || chat.version <= pending.version) return;
        setSubmission();
        setEditor();
        setProblem('');
    });
    const update = (key: 'name' | 'handle', value: string) => {
        if (!busy()) setEditor(previous => previous && { ...previous, [key]: value });
    };
    return { editor, open, cancel, submit, update, allowed, address, busy, inputProblem, message, state, check: () => { const value = submission(); if (value) live.checkCommand(value.id); }, canCheck: () => Boolean(submission() && !failed(state())), canSubmit: () => Boolean(editor() && !busy() && !submission() && !inputProblem()) };
}
