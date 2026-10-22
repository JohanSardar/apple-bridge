import {AudioBubble} from './AudioBubble';
import { Show } from 'solid-js';
import {MessageCard} from './MessageCard';
import {messageCardData} from './cards';
import {MessagePlaceholder} from './MessagePlaceholder';
import type { BridgeRecord, ThreadItem } from '../../data/types';
import type { Part } from '../store/types';
import { fields, text } from '../store/types';
import { MediaImage } from './MediaImage';
import {MotionBubble} from './MotionBubble';
import {StickerImage} from './StickerImage';
import {Genmoji} from './Genmoji';
import { attachmentHasAlpha } from './source';
import {preloadMediaViewer} from './ChatMediaViewer';
export type MessageMediaProps = { message: ThreadItem; part: Part; width: number; height: number; onOpen?: (attachment: BridgeRecord, rect: DOMRectReadOnly) => void };
export function MessageMedia(props: MessageMediaProps) {
    const preload=()=>{if(props.onOpen)void preloadMediaViewer().catch(()=>{});};
    const attachment = () => props.message.attachments.find(value => value.id === props.part.attachment_id);
    return <Show when={props.part.kind==='balloon'} fallback={<Show when={attachment()}>{value => <Show when={text(value().data.mime_type).startsWith('audio/')} fallback={<div class="media-bubble" data-media-alpha={attachmentHasAlpha(value())} style={{width:`${props.width}px`,height:`${props.height}px`}}><Show when={value().data.genmoji===true} fallback={<Show when={value().data.is_sticker===true} fallback={<Show when={text(value().data.mime_type).startsWith('video/')||fields(value().data.media).live===true} fallback={<><MediaImage attachment={value()}/><button type="button" class="media-open" onPointerEnter={preload} onPointerDown={preload} onFocus={preload} aria-label={`Open ${text(value().data.description) || text(value().data.filename) || 'photo'}`} onClick={event => props.onOpen?.(value(), event.currentTarget.getBoundingClientRect())}/></>}><MotionBubble attachment={value()} onOpen={props.onOpen}/></Show>}><StickerImage attachment={value()} size="100%"/><button type="button" class="media-open" aria-label="Open sticker" onPointerEnter={preload} onPointerDown={preload} onFocus={preload} onClick={event=>props.onOpen?.(value(),event.currentTarget.getBoundingClientRect())}/></Show>}><span class="media-genmoji-slot"><Genmoji attachment={value()} size={Math.min(props.width,props.height)}/></span></Show></div>}><AudioBubble message={props.message} attachment={value()}/></Show>}</Show>}><Show when={messageCardData(props.message)} fallback={<MessagePlaceholder message={props.message}/>}><MessageCard message={props.message}/></Show></Show>;
}
