import { createSignal, onMount, Show } from 'solid-js';
import { Portal } from 'solid-js/web';
import type { ChatSummary } from '../../data/types';
import type { createParityCommands } from './commands';
import { capturedMute } from './commands';
import './parity.css';
export function ParityMenuItems(props: { chat: ChatSummary; commands: ReturnType<typeof createParityCommands>; close: () => void }) {
    const [confirmation, setConfirmation] = createSignal<number>();
    const record = () => props.commands.record(props.chat.chat.id);
    const deleted = () => Boolean(record()?.data.deleted_at);
    const index = () => record()?.data.pinned_index;
    const pinned = () => typeof index() === 'number';
    const submit = (event: Event, kind: Parameters<typeof props.commands.submit>[1], args?: Record<string, unknown>) => { const target = record(); if (target && props.commands.submit(event, kind, target, args)) props.close(); };
    function Confirmation() {
        let dialog!: HTMLDialogElement;
        let trigger: HTMLElement | null = null;
        const current = record;
        const stale = () => current()?.version !== confirmation();
        onMount(() => { trigger = document.activeElement as HTMLElement | null; dialog.showModal(); dialog.querySelector<HTMLButtonElement>('button')?.focus(); });
        const close = () => { setConfirmation(); if (trigger?.isConnected) trigger.focus({ preventScroll: true }); };
        return <Portal><dialog ref={dialog} class="parity-delete-confirm" aria-labelledby="parity-delete-title" onCancel={event => { event.preventDefault(); close(); }} onKeyDown={event => event.stopPropagation()}>
            <h2 id="parity-delete-title">Delete Conversation Permanently?</h2><p>This conversation will be permanently deleted. This can’t be undone.</p>
            <Show when={stale()}><p role="status">The conversation changed. Cancel and open its actions again.</p></Show>
            <div><button type="button" autofocus onClick={close}>Cancel</button><button type="button" class="destructive" disabled={!props.commands.can('delete_chat_permanently', record()) || stale()} onClick={event => { const record = current(); if (!stale() && record && props.commands.submit(event, 'delete_chat_permanently', record)) props.close(); }}>Delete Permanently</button></div>
        </dialog></Portal>;
    }
    return <Show when={record()}><>
        <Show when={!deleted()}>
            <Show when={props.commands.allowed('pin')}><button disabled={!props.commands.can('pin', record())} onClick={event => submit(event, 'pin', { pinned_index: pinned() ? -1 : 0 })}>{pinned() ? 'Unpin Conversation' : 'Pin Conversation'}</button><Show when={pinned() && Number(index()) > 0}><button disabled={!props.commands.can('pin', record())} onClick={event => submit(event, 'pin', { pinned_index: Number(index()) - 1 })}>Move Pin Earlier</button></Show><Show when={pinned() && Number(index()) < 8}><button disabled={!props.commands.can('pin', record())} onClick={event => submit(event, 'pin', { pinned_index: Number(index()) + 1 })}>Move Pin Later</button></Show></Show>
            <Show when={props.commands.allowed('mute') && (record() ? capturedMute(record()!) : undefined) !== undefined}><button disabled={!props.commands.can('mute', record())} onClick={event => submit(event, 'mute', { muted: !(record() ? capturedMute(record()!) : undefined) })}>{(record() ? capturedMute(record()!) : undefined) ? 'Show Alerts' : 'Hide Alerts'}</button></Show>
        </Show>
        <Show when={deleted()}>
            <Show when={props.commands.allowed('recover_chat')}><button disabled={!props.commands.can('recover_chat', record())} onClick={event => submit(event, 'recover_chat')}>Recover Conversation</button></Show>
            <Show when={props.commands.allowed('delete_chat_permanently')}><button class="destructive" disabled={!props.commands.can('delete_chat_permanently', record())} onClick={event => { if (event.isTrusted) setConfirmation(record()?.version); }}>Delete Permanently…</button></Show>
        </Show>
        <Show when={confirmation()}><Confirmation/></Show>
    </></Show>;
}
