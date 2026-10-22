import { createSignal, For, onCleanup, onMount, Show, type JSX } from 'solid-js';
import { strings as s } from '../../app/strings';
import { Portal } from 'solid-js/web';
import type { ChatSummary } from '../../data/types';
import type { ChatActions } from '../actions/chat';
import { conversationTitle } from '../format/list';
import type { Handles } from '../store/types';
import './chat-actions.css';

export function createChatSurface() {
    const [target, setTarget] = createSignal<{ chat: ChatSummary; anchor: HTMLElement; confirm: boolean }>();
    return {
        target,
        open: (chat: ChatSummary, anchor: HTMLElement, confirm = false) => setTarget({ chat, anchor, confirm }),
        close: () => setTarget()
    };
}
export type ChatSurface = ReturnType<typeof createChatSurface>;
export function ChatActionSurface(props: {
    surface: ChatSurface; actions: ChatActions; handles: Handles; onDeleted: (id: string) => void;
    extraActions?: (chat: ChatSummary, anchor: HTMLElement, close: () => void) => JSX.Element
}) {
    function Panel(panel: { value: NonNullable<ReturnType<ChatSurface['target']>> }) {
        const value = panel.value;
        let dialog!: HTMLDialogElement;
        const close = () => {
            props.surface.close();
            if (value.anchor.isConnected)
                (value.anchor.matches('a,button') ? value.anchor : value.anchor.querySelector<HTMLElement>('a,button'))?.focus({ preventScroll: true });
            else document.querySelector<HTMLElement>('.conversation-list [role=listbox]')?.focus({ preventScroll: true });
        };
        const extras = value.confirm ? undefined : props.extraActions?.(value.chat, value.anchor, close);
        onMount(() => {
            dialog.showModal();
            if (!value.confirm) {
                const rect = value.anchor.getBoundingClientRect(), box = dialog.getBoundingClientRect();
                dialog.style.left = Math.max(8, Math.min(innerWidth - box.width - 8, rect.left + 32)) + 'px';
                dialog.style.top = Math.max(8, Math.min(innerHeight - box.height - 8, rect.top + 16)) + 'px';
            }
            dialog.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus({ preventScroll: true });
        });
        const read = () => value.chat.unread_count > 0;
        const keydown = (event: KeyboardEvent) => {
            event.stopPropagation();
            if (!['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) return;
            event.preventDefault();
            const buttons = [...dialog.querySelectorAll<HTMLButtonElement>('button:not(:disabled)')];
            const index = buttons.indexOf(document.activeElement as HTMLButtonElement);
            const next = event.key === 'Home' ? 0 : event.key === 'End' ? buttons.length - 1
                : (index + (event.key === 'ArrowDown' ? 1 : -1) + buttons.length) % buttons.length;
            buttons[next]?.focus();
        };
        return <Portal><dialog ref={dialog} class="chat-action-dialog"
            classList={{ 'chat-delete-confirm': value.confirm }}
            aria-label={value.confirm ? s.deleteConversationDialog : s.conversationActions}
            onCancel={event => { event.preventDefault(); close(); }}
            onKeyDown={keydown}
            onClick={event => {
                if (event.target !== dialog) return;
                const box = dialog.getBoundingClientRect();
                if (event.clientX < box.left || event.clientX > box.right || event.clientY < box.top || event.clientY > box.bottom) close();
            }}>
            <Show when={value.confirm} fallback={<>
                <button disabled={!props.actions.can(read() ? 'mark_read' : 'mark_unread', value.chat.chat.id)}
                    onClick={event => {
                        if (!event.isTrusted) return;
                        if (read()) props.actions.markRead(event, value.chat.chat.id);
                        else props.actions.markUnread(event, value.chat.chat.id);
                        close();
                    }}>{read() ? s.markAsRead : s.markAsUnread}</button>
                <button class="destructive" disabled={!props.actions.can('delete_chat', value.chat.chat.id)}
                    onClick={event => { if (event.isTrusted) props.surface.open(value.chat, value.anchor, true); }}>{s.deleteConversation}</button>
                {extras}
                <button onClick={close}>{s.closeAction}</button>
            </>}>
                <h2>{s.deleteConversationTitle(conversationTitle(value.chat, props.handles))}</h2>
                <p>{s.deleteConversationHint}</p>
                <button autofocus onClick={close}>{s.cancel}</button>
                <button class="destructive" disabled={!props.actions.can('delete_chat', value.chat.chat.id)}
                    onClick={event => {
                        if (!event.isTrusted) return;
                        const id = props.actions.deleteChat(event, value.chat.chat.id);
                        if (id) { props.onDeleted(value.chat.chat.id); close(); }
                    }}>{s.deleteAction}</button>
            </Show>
        </dialog></Portal>;
    }
    return <>
        <Show when={props.surface.target()} keyed>{value => <Panel value={value}/>}</Show>
        <Show when={props.actions.problem()}><p role="status" class="chat-action-notice">{props.actions.problem()}</p></Show>
        <For each={props.actions.notices()}>{notice => <div class="chat-action-notice" role="status">
            {notice.text}
            <Show when={notice.check}><button onClick={() => props.actions.check(notice.id)}>{s.checkAction}</button></Show>
            <Show when={notice.dismiss}><button onClick={() => props.actions.dismiss(notice.id)}>{s.dismiss}</button></Show>
        </div>}</For>
    </>;
}
export function SwipeChatRow(props: {
    chat: ChatSummary; actions: ChatActions; surface: ChatSurface; children: JSX.Element;
    extraTrailing?: (chat: ChatSummary, anchor: HTMLElement, close: () => void) => JSX.Element
}) {
    let root!: HTMLDivElement, origin: { id: number; x: number; y: number; width: number } | undefined, suppress = false;
    const [offset, setOffset] = createSignal(0);
    let trailing: HTMLDivElement | undefined;
    const finish = (event: PointerEvent, cancelled = false) => {
        if (!origin || event.pointerId !== origin.id) return;
        const dx = offset(), width = origin.width;
        origin = undefined;
        if (cancelled) { setOffset(0); return; }
        if (Math.abs(dx) > 8) { suppress = true; event.preventDefault(); }
        if (Math.abs(dx) > width * .7) {
            if (dx < 0) props.surface.open(props.chat, root, true);
            else if (props.chat.unread_count > 0) props.actions.markRead(event, props.chat.chat.id);
            else props.actions.markUnread(event, props.chat.chat.id);
            setOffset(0);
        } else setOffset(Math.abs(dx) > 45 ? (dx < 0 ? -(trailing?.scrollWidth || 88) : 88) : 0);
    };
    onCleanup(() => { origin = undefined; });
    return <div ref={root} class="chat-swipe-row" onDragStart={event => event.preventDefault()}
        onContextMenu={event => { event.preventDefault(); props.surface.open(props.chat, event.currentTarget); }}
        onKeyDown={event => {
            if (event.key === 'ContextMenu' || event.shiftKey && event.key === 'F10') {
                event.preventDefault(); event.stopPropagation(); props.surface.open(props.chat, event.currentTarget);
            }
        }}
        onPointerDown={event => {
            if (event.button !== 0 || (event.target as Element).closest('button')) return;
            suppress = false;
            origin = { id: event.pointerId, x: event.clientX, y: event.clientY, width: event.currentTarget.getBoundingClientRect().width };
        }}
        onPointerMove={event => {
            if (!origin || origin.id !== event.pointerId) return;
            const dx = event.clientX - origin.x, dy = event.clientY - origin.y;
            if (Math.abs(dy) > Math.abs(dx) && Math.abs(dy) > 8) { origin = undefined; setOffset(0); return; }
            if (Math.abs(dx) > 8) {
                root.setPointerCapture(event.pointerId);
                setOffset(Math.max(-origin.width, Math.min(origin.width, dx)));
            }
        }}
        onPointerUp={event => finish(event)} onPointerCancel={event => finish(event, true)}
        onLostPointerCapture={event => finish(event, true)}
        on:click={(event: MouseEvent) => {
            if (suppress) { event.preventDefault(); event.stopPropagation(); suppress = false; }
        }}>
        <Show when={offset() !== 0}><div class="chat-swipe-buttons" data-side={offset() < 0 ? 'delete' : 'read'}>
            <Show when={offset() < 0} fallback={
                <button disabled={!props.actions.can(props.chat.unread_count > 0 ? 'mark_read' : 'mark_unread', props.chat.chat.id)}
                    onClick={event => {
                        if (props.chat.unread_count > 0) props.actions.markRead(event, props.chat.chat.id);
                        else props.actions.markUnread(event, props.chat.chat.id);
                        setOffset(0);
                    }}>{props.chat.unread_count > 0 ? s.readAction : s.unread}</button>
            }>
                <div class="chat-swipe-trailing" ref={trailing}>
                    {props.extraTrailing?.(props.chat, root, () => setOffset(0))}
                    <button class="destructive" disabled={!props.actions.can('delete_chat', props.chat.chat.id)}
                        onClick={() => { props.surface.open(props.chat, root, true); setOffset(0); }}>{s.deleteAction}</button>
                </div>
            </Show>
        </div></Show>
        <div class="chat-swipe-content" style={{ transform: `translateX(${offset()}px)` }}>{props.children}</div>
    </div>;
}
