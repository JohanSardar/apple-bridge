import { createEffect, createSignal, onCleanup, Show } from 'solid-js';
import type { CommandState, DeviceSession } from '../../data/types';
import { crossfade, reducedMotion, springAnimation } from '../../motion/animate';
import { messageCommandLabel } from '../actions/commands';
import { readDraft, writeDraft } from './drafts';
import './compose.css';
export type ComposerProps = {
    draftID: string;
    service?: string;
    session: DeviceSession;
    available: boolean;
    onSend: (event: Event, content: { text: string }) => string;
    command?: CommandState;
    onCheck?: () => void;
    onCopy?: (event: Event) => void;
};
export function Composer(props: ComposerProps) {
    const [text, setText] = createSignal(readDraft(props.session, props.draftID)), [error, setError] = createSignal('');
    let field!: HTMLTextAreaElement, frame = 0;
    const sms = () => ['SMS', 'RCS'].includes(props.service ?? '');
    const content = () => Boolean(text().trim());
    const valid = () => new TextEncoder().encode(text()).length <= 65536;
    const update = (value: string) => { setText(value); writeDraft(props.session, props.draftID, value); setError(''); };
    createEffect(() => {
        text(); cancelAnimationFrame(frame);
        frame = requestAnimationFrame(() => {
            const style = getComputedStyle(field), border = parseFloat(style.borderTopWidth) + parseFloat(style.borderBottomWidth);
            const maximum = parseFloat(style.lineHeight) * 6 + parseFloat(style.paddingTop) + parseFloat(style.paddingBottom) + border;
            field.style.height = 'auto';
            field.style.height = Math.min(field.scrollHeight + border, maximum) + 'px';
        });
    });
    onCleanup(() => cancelAnimationFrame(frame));
    const send = (event: Event) => {
        if (!props.available || !content() || !valid()) return;
        try { props.onSend(event, { text: text() }); update(''); }
        catch (error) { setError((error as Error).message); }
    };
    const enter = (element: HTMLButtonElement) => {
        if (reducedMotion()) { const animation = crossfade(element); onCleanup(() => animation.cancel()); return; }
        const animation = springAnimation(value => { element.style.transform = `scale(${value})`; element.style.opacity = String(Math.min(1, value)); }, .6, 1, { response: .35, damping: .72 });
        onCleanup(() => animation.cancel());
    };
    return <section class="message-composer" classList={{ 'text-service': sms() }} aria-label="Message composer">
        <div class="composer-field glass">
            <textarea ref={field} aria-label="Message" placeholder={sms() ? 'Text Message' : 'iMessage'} rows={1} value={text()} onInput={event => update(event.currentTarget.value)} onKeyDown={event => {
                if (event.key !== 'Enter' || event.shiftKey || event.isComposing || event.keyCode === 229) return;
                event.preventDefault(); if (!event.repeat) send(event);
            }}/>
            <Show when={content() && props.available}><button ref={enter} class="composer-send" aria-label="Send message" disabled={!valid()} onClick={send}><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 19V5m-6 6 6-6 6 6"/></svg></button></Show>
        </div>
        <Show when={!valid()}><p class="composer-error" role="alert">Message is too long</p></Show>
        <Show when={error()}><p class="composer-error" role="alert">{error()}</p></Show>
        <Show when={props.command}>{command => <div class="composer-status" role="status">{messageCommandLabel(command())}<Show when={command().status === 'indeterminate'}> · <button class="button subtle" onClick={() => props.onCheck?.()}>Check</button><button class="button subtle" onClick={event => props.onCopy?.(event)}>Copy</button></Show></div>}</Show>
    </section>;
}
