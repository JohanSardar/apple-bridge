import { Show } from 'solid-js';
import {MessageCard} from './MessageCard';
import type { BridgeRecord, ThreadItem } from '../../data/types';
import type { Part } from '../store/types';
import { fields, text } from '../store/types';
import { MediaImage } from './MediaImage';
import {MotionBubble} from './MotionBubble';
import { attachmentHasAlpha } from './source';
import {preloadMediaViewer} from './ChatMediaViewer';
export type MessageMediaProps = { message: ThreadItem; part: Part; width: number; height: number; onOpen?: (attachment: BridgeRecord, rect: DOMRectReadOnly) => void };
export function MessageMedia(props: MessageMediaProps) {
    const preload=()=>{if(props.onOpen)void preloadMediaViewer().catch(()=>{});};
    const attachment = () => props.message.attachments.find(value => value.id === props.part.attachment_id);
    return <Show when={props.part.kind==='balloon'} fallback={<Show when={attachment()}>{value => <div class="media-bubble" data-media-alpha={attachmentHasAlpha(value())} style={{width:`${props.width}px`,height:`${props.height}px`}}><Show when={text(value().data.mime_type).startsWith('video/')||fields(value().data.media).live===true} fallback={<><MediaImage attachment={value()}/><button type="button" class="media-open" onPointerEnter={preload} onPointerDown={preload} onFocus={preload} aria-label={`Open ${text(value().data.description) || text(value().data.filename) || 'photo'}`} onClick={event => props.onOpen?.(value(), event.currentTarget.getBoundingClientRect())}/></>}><MotionBubble attachment={value()} onOpen={props.onOpen}/></Show></div>}</Show>}><MessageCard message={props.message}/></Show>;
}
