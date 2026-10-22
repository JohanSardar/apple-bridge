import { createEffect, createMemo, createSignal, onCleanup, Show } from 'solid-js';
import type { CommandState, DeviceSession } from '../../data/types';
import { crossfade, reducedMotion, springAnimation } from '../../motion/animate';
import { messageCommandLabel } from '../actions/commands';
import { appearance } from '../../app/preferences';
import { flushDraft, readDraft, writeDraft } from './drafts';
import './compose.css';
export type ComposerProps = {
    draftID: string;
    service?: string;
    session: DeviceSession;
    available: boolean;
    disabled?: boolean;
    disabledReason?: string;
    onSend: (event: Event, content: { text: string }) => string;
    reply?: {key:string;text:string};
    onCancelReply?: () => void;
    command?: CommandState;
    canCheck?: boolean;
    onCheck?: () => void;
    onCopy?: (event: Event) => void;
};
export function Composer(props: ComposerProps) {
    const session = props.session;
    const [text, setText] = createSignal(readDraft(session, props.draftID)), [error, setError] = createSignal('');
    let field!: HTMLTextAreaElement, frame = 0, metrics: { border: number; maximum: number; width: number } | undefined;
    const sms = () => ['SMS', 'RCS'].includes(props.service ?? '');
    const content = () => Boolean(text().trim());
    const bytes = createMemo(() => new TextEncoder().encode(text()).length);
    const valid = () => bytes() <= 65536;
    const problem = () => props.command && ['failed', 'rejected', 'delivery_failed', 'expired', 'indeterminate'].includes(props.command.status) ? props.command : undefined;
    const update = (value: string) => { setText(value); writeDraft(session, props.draftID, value); setError(''); };
    const resize = () => {
        cancelAnimationFrame(frame);
        frame = requestAnimationFrame(() => {
            if (!metrics) {
                const style = getComputedStyle(field), border = parseFloat(style.borderTopWidth) + parseFloat(style.borderBottomWidth);
                metrics = { border, maximum: parseFloat(style.lineHeight) * 6 + parseFloat(style.paddingTop) + parseFloat(style.paddingBottom) + border, width: field.clientWidth };
            }
            field.style.height = 'auto';
            field.style.height = Math.min(field.scrollHeight + metrics.border, metrics.maximum) + 'px';
        });
    };
    const replyKey=createMemo(()=>props.reply?.key);
    const cancelReply=()=>{props.onCancelReply?.();field.focus();};
    createEffect(() => { if(replyKey())queueMicrotask(()=>field.focus()); });
    createEffect(() => { text(); resize(); });
    createEffect(() => { appearance.textSize(); metrics = undefined; resize(); });
    const observer = new ResizeObserver(() => { if (metrics && field.clientWidth !== metrics.width) { metrics.width = field.clientWidth; resize(); } });
    const phone = matchMedia('(max-width:699px)'), typography = () => { metrics = undefined; resize(); };
    phone.addEventListener('change', typography);
    createEffect(() => { observer.observe(field); });
    onCleanup(() => { cancelAnimationFrame(frame); observer.disconnect(); phone.removeEventListener('change', typography); flushDraft(session, props.draftID); });
    const send = (event: Event) => {
        if (!props.available || props.disabled || !content() || !valid()) return;
        try { props.onSend(event, { text: text() }); update(''); }
        catch { setError("Can't send right now. Check your connection."); }
    };
    const enter = (element: HTMLButtonElement) => {
        if (reducedMotion()) { const animation = crossfade(element); onCleanup(() => animation.cancel()); return; }
        const animation = springAnimation(value => { element.style.transform = `scale(${value})`; element.style.opacity = String(Math.min(1, value)); }, .6, 1, { response: .35, damping: .72 }, 0, () => { element.style.transform = ''; element.style.opacity = ''; });
        onCleanup(() => animation.cancel());
    };
    return <section class="message-composer" classList={{ 'text-service': sms() }} aria-label="Message composer">
        <Show when={props.reply}>{reply=><section class="composer-quote" aria-label="Replying to message"><div><small>Replying to message</small><p>{reply().text}</p></div><button class="icon-button" aria-label="Cancel reply" onClick={cancelReply}>×</button></section>}</Show>
        <div class="composer-field glass">
            <textarea ref={field} aria-label="Message" placeholder={sms() ? 'Text Message' : 'iMessage'} rows={1} value={text()} onInput={event => update(event.currentTarget.value)} onKeyDown={event => {
                if(event.key==='Escape'&&props.reply&&!event.isComposing&&event.keyCode!==229){event.preventDefault();cancelReply();return;}
                if (event.key !== 'Enter' || event.shiftKey || event.isComposing || event.keyCode === 229) return;
                event.preventDefault(); if (!event.repeat) send(event);
            }}/>
            <Show when={content() && props.available}><button ref={enter} class="composer-send" aria-label="Send message" disabled={props.disabled || !valid()} onClick={send}><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 19V5m-6 6 6-6 6 6"/></svg></button></Show>
        </div>
        <Show when={props.disabledReason}><p class="composer-status" role="status">{props.disabledReason}</p></Show>
        <Show when={!valid()}><p class="composer-error" role="alert">Message is too long</p></Show>
        <Show when={error()}><p class="composer-error" role="alert">{error()}</p></Show>
        <Show when={problem()}>{command => <div class="composer-status" role="status">{messageCommandLabel(command())}<Show when={command().status === 'indeterminate'}> · <button class="button subtle" disabled={props.canCheck===false} onClick={() => props.onCheck?.()}>Check</button><button class="button subtle" onClick={event => props.onCopy?.(event)}>Copy</button></Show></div>}</Show>
    </section>;
}
