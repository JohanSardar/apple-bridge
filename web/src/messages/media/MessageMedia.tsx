import { Show } from 'solid-js';
import type { BridgeRecord, ThreadItem } from '../../data/types';
import type { Part } from '../store/types';
import { text } from '../store/types';
import { MediaImage } from './MediaImage';
import { attachmentHasAlpha } from './source';
export type MessageMediaProps = { message: ThreadItem; part: Part; width: number; height: number; onOpen?: (attachment: BridgeRecord, rect: DOMRectReadOnly) => void };
export function MessageMedia(props: MessageMediaProps) {
    const attachment = () => props.message.attachments.find(value => value.id === props.part.attachment_id);
    return <Show when={attachment()}>{value => <div class="media-bubble" data-media-alpha={attachmentHasAlpha(value())} style={{width:`${props.width}px`,height:`${props.height}px`}}><MediaImage attachment={value()}/><button type="button" class="media-open" aria-label={`Open ${text(value().data.description) || text(value().data.filename) || 'photo'}`} onClick={event => props.onOpen?.(value(), event.currentTarget.getBoundingClientRect())}/></div>}</Show>;
}
