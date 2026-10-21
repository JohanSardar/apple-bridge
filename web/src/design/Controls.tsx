import { createSignal, onCleanup, Show, type JSX } from 'solid-js';
import './controls.css';
import { Icon } from './Icon';
import { reducedMotion, springAnimation } from '../motion/animate';
export function Toggle(props: { label: string; checked: boolean; onChange: (checked: boolean) => void }) {
    return <button type="button" class="switch" role="switch" aria-label={props.label} aria-checked={props.checked} onClick={() => props.onChange(!props.checked)}><span /></button>;
}
export function Disclosure(props: { title: string; children: JSX.Element; class?: string }) {
    const [open, setOpen] = createSignal(false);
    let chevron!: HTMLSpanElement, animation: ReturnType<typeof springAnimation> | undefined;
    function toggle() {
        const from = open() ? 270 : 180;
        setOpen(!open());
        const target = open() ? 270 : 180;
        if (reducedMotion()) { animation?.cancel(); animation = undefined; chevron.style.transform = `rotate(${target}deg)`; return; }
        if (animation) { animation.retarget(target); return; }
        animation = springAnimation(value => { chevron.style.transform = `rotate(${value}deg)`; }, from, target, { response: .32, damping: .82 });
    }
    onCleanup(() => animation?.cancel());
    return <section class={'disclosure ' + (props.class ?? '')}><button class="disclosure-row" type="button" aria-expanded={open()} onClick={toggle}><span>{props.title}</span><span ref={chevron} class="disclosure-chevron"><Icon name="chevron" /></span></button><Show when={open()}><div class="disclosure-content">{props.children}</div></Show></section>;
}
