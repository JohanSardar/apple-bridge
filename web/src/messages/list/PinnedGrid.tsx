import { strings as s } from '../../app/strings';
import { createEffect, createSignal, For, onCleanup, Show } from 'solid-js';
import type { ChatSummary } from '../../data/types';
import type { Handles } from '../store/types';
import { ChatAvatar } from './Avatar';
import { sampleSpring } from '../../motion/spring';
import { reducedMotion } from '../../motion/animate';
import { conversationTitle } from '../format/list';
type Drag = { id: number; chat: ChatSummary; element: HTMLButtonElement; x: number; y: number; active: boolean; touch: boolean; order: string; over: number };
export function PinnedGrid(props: { chats: readonly ChatSummary[]; handles: Handles; selected?: string; onOpen: (chat: ChatSummary, event: MouseEvent) => void; onContext?: (chat: ChatSummary, anchor: HTMLElement) => void; onReorder?: (event: PointerEvent, chat: ChatSummary, index: number) => void }) {
    const animations = new Set<Animation>();
    let origin: Drag | undefined, hold: ReturnType<typeof setTimeout> | undefined, suppress = false;
    const [drag, setDrag] = createSignal<{ id: string; x: number; y: number; over: number }>();
    const clear = () => {
        const value = origin, current = drag(); clearTimeout(hold); origin = undefined; setDrag();
        if (value) {
            try { if (value.element.hasPointerCapture(value.id)) value.element.releasePointerCapture(value.id); } catch {}
            if (current && value.element.isConnected && !reducedMotion()) {
                const frames = Array.from({length:31},(_,index) => {
                    const position = index === 30 ? 0 : sampleSpring(1,0,0,index/60,{response:.3,damping:.8}).value;
                    return {transform:`translate3d(${current.x*position}px,${current.y*position}px,0) scale(${1+.04*position})`};
                });
                const animation = value.element.animate(frames,{duration:500}); animations.add(animation);
                void animation.finished.finally(()=>animations.delete(animation)).catch(()=>{});
            }
        }
    };
    const order = () => props.chats.map(chat => chat.chat.id).join('|');
    createEffect(() => { const current = order(), enabled = props.onReorder; if (origin && (!enabled || current !== origin.order)) clear(); });
    onCleanup(() => { clear(); for (const animation of animations) animation.cancel(); });
    const begin = (event: PointerEvent, chat: ChatSummary, element: HTMLButtonElement) => {
        if (!event.isTrusted || event.button !== 0 || !props.onReorder) return;
        clear(); for (const animation of animations) animation.cancel(); animations.clear(); suppress = false;
        const value: Drag = { id: event.pointerId, chat, element, x: event.clientX, y: event.clientY, active: false, touch: event.pointerType !== 'mouse', order: order(), over: props.chats.indexOf(chat) };
        origin = value;
        if (value.touch) hold = setTimeout(() => {
            if (origin !== value || !value.element.isConnected) return;
            value.active = true;
            value.element.setPointerCapture(value.id);
            setDrag({ id: chat.chat.id, x: 0, y: 0, over: value.over });
        }, 350);
    };
    const move = (event: PointerEvent) => {
        const value = origin;
        if (!value || event.pointerId !== value.id) return;
        const dx = event.clientX - value.x, dy = event.clientY - value.y;
        if (!value.active && Math.hypot(dx, dy) > 8) {
            if (value.touch) { clear(); return; }
            value.active = true; value.element.setPointerCapture(value.id);
        }
        if (!value.active) return;
        const hit = document.elementsFromPoint(event.clientX, event.clientY).map(element => element.closest<HTMLElement>('[data-pin]')).find(element => element?.dataset.pin !== value.chat.chat.id && element?.dataset.pin);
        const over = props.chats.findIndex(chat => chat.chat.id === hit?.dataset.pin);
        value.over = over;
        event.preventDefault();
        setDrag({ id: value.chat.chat.id, x: dx, y: dy, over });
    };
    const finish = (event: PointerEvent, cancelled = false) => {
        const value = origin;
        if (!value || value.id !== event.pointerId) return;
        const active = value.active, over = value.over;
        clear();
        if (active) { suppress = true; event.preventDefault(); }
        if (!cancelled && active && over >= 0 && over !== props.chats.findIndex(chat => chat.chat.id === value.chat.chat.id) && value.order === order()) props.onReorder?.(event, value.chat, over);
    };
    return <div class="pinned-grid" role="group" aria-label={s.pinnedConversations} onDragStart={event => event.preventDefault()} onKeyDown={event => { if (event.key === 'Escape' && origin) { event.preventDefault(); clear(); } }}><For each={props.chats}>{chat => <button class="pinned-conversation" data-chat={chat.chat.id} data-pin={chat.chat.id} data-reorderable={Boolean(props.onReorder)} data-dragging={drag()?.id === chat.chat.id} data-drop-target={drag()?.over === props.chats.indexOf(chat)} style={drag()?.id === chat.chat.id ? { transform: `translate3d(${drag()!.x}px,${drag()!.y}px,0) scale(1.04)`, 'z-index': 2 } : {}} onContextMenu={event => { event.preventDefault(); props.onContext?.(chat,event.currentTarget); }} onKeyDown={event => { if(event.key === 'ContextMenu' || event.shiftKey && event.key === 'F10') { event.preventDefault(); props.onContext?.(chat,event.currentTarget); } }} aria-label={conversationTitle(chat, props.handles)} aria-pressed={props.selected === chat.chat.id} onClick={event => { if (suppress && event.detail !== 0) { suppress = false; return; } props.onOpen(chat, event); }} onPointerDown={event => begin(event, chat, event.currentTarget)} onPointerMove={move} onPointerUp={event => finish(event)} onPointerCancel={event => finish(event, true)} onLostPointerCapture={event => finish(event, true)}><Show when={chat.unread_count > 0 ? chat.pin_preview : undefined}>{value => <span class="pin-preview"><span>{value().text_preview}</span></span>}</Show><ChatAvatar chat={chat} handles={props.handles}/><span class="pin-name">{conversationTitle(chat, props.handles)}</span></button>}</For></div>;
}
