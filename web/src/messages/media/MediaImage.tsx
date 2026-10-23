import { createEffect, createMemo, createSignal, onCleanup, Show } from 'solid-js';
import type { BridgeRecord } from '../../data/types';
import { fields, text } from '../store/types';
import { decodeThumbHash } from './thumbhash';
import { attachmentHasAlpha, mediaSource, type MediaVariant } from './source';
import './media.css';
const placeholders=new Map<string,ImageData>();
export function MediaImage(props: { attachment: BridgeRecord; variant?: MediaVariant; alt?: string; interactive?: boolean }) {
    const [state, setState] = createSignal<'loading' | 'ready' | 'error'>('loading');
    const [attempt, setAttempt] = createSignal(0);
    let canvas!: HTMLCanvasElement, image:HTMLImageElement|undefined, disposed = false;
    onCleanup(() => { disposed = true; image?.removeAttribute('src'); });
    const source = createMemo(() => mediaSource(props.attachment, props.variant ?? 'thumb'));
    const placeholder = createMemo(() => text(fields(props.attachment.data.media).thumbhash));
    createEffect(() => { source(); attempt(); setState('loading'); });
    createEffect(() => {
        const hash=placeholder();
        const paint=()=>{
            let pixels=placeholders.get(hash);
            if(!pixels){const decoded=decodeThumbHash(hash);if(decoded){pixels=new ImageData(decoded.rgba,decoded.width,decoded.height);if(placeholders.size>=64)placeholders.delete(placeholders.keys().next().value!);placeholders.set(hash,pixels);}}
            canvas.width=pixels?.width??32;canvas.height=pixels?.height??32;
            canvas.dataset.thumbhash = pixels ? 'decoded' : 'missing';
            if(pixels)canvas.getContext('2d')?.putImageData(pixels,0,0);
        };
        if(!hash||placeholders.has(hash))paint();else{canvas.width=32;let frame=requestAnimationFrame(()=>{frame=requestAnimationFrame(paint);});onCleanup(()=>cancelAnimationFrame(frame));}
    });
    const decode = async (image: HTMLImageElement) => {
        const expected = source(),generation=attempt();
        try { await image.decode(); if (!disposed && source() === expected && generation===attempt()) setState('ready'); } catch { if (!disposed && source() === expected && generation===attempt()) setState('error'); }
    };
    return <div class="media-image" data-media-alpha={attachmentHasAlpha(props.attachment)} data-media-id={props.attachment.id} data-media-width={fields(props.attachment.data.media).width} data-media-height={fields(props.attachment.data.media).height} data-media-state={state()}>
        <canvas ref={canvas} class="media-placeholder" aria-hidden="true"/>
        <Show when={source()} fallback={<span class="media-unavailable" role="status">Attachment unavailable</span>}>
            <Show keyed when={`${source()}#${attempt()}`}>{_key => <img ref={image} src={source()} alt={props.alt ?? (text(props.attachment.data.description) || text(props.attachment.data.filename) || 'Photo')} decoding="async" draggable={false} onLoad={event => void decode(event.currentTarget)} onError={() => setState('error')}/>}</Show>
        </Show>
        <Show when={state() === 'error'}><span class="media-unavailable" role="status">Media unavailable<Show when={props.interactive!==false}><button type="button" onClick={event => { event.stopPropagation(); setAttempt(value => value + 1); }}>Try again</button></Show></span></Show>
    </div>;
}
