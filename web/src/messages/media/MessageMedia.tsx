import {FileChip} from './FileChip';
import {ContactCard} from './ContactCard';
import {attachmentIsContact} from './files';
import {AudioBubble} from './AudioBubble';
import { Match, Show, Switch } from 'solid-js';
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
    const visual=(value:BridgeRecord)=>/^(image|video)\//u.test(text(value.data.mime_type))||value.data.genmoji===true||value.data.is_sticker===true||fields(value.data.media).live===true;
    const attachment = () => props.message.attachments.find(value => value.id === props.part.attachment_id);
    const kind=(value:BridgeRecord)=>value.data.genmoji===true?'genmoji':value.data.is_sticker===true?'sticker':text(value.data.mime_type).startsWith('video/')||fields(value.data.media).live===true?'motion':'image';
    return <Show when={props.part.kind==='balloon'} fallback={<Show when={attachment()}>{value=><Switch>
        <Match when={text(value().data.mime_type).startsWith('audio/')}><AudioBubble message={props.message} attachment={value()}/></Match>
        <Match when={visual(value())}><div class="media-bubble" data-media-alpha={attachmentHasAlpha(value())} style={{width:`${props.width}px`,height:`${props.height}px`}}>
            <Switch fallback={<MediaImage attachment={value()}/>}>
                <Match when={kind(value())==='genmoji'}><span class="media-genmoji-slot"><Genmoji attachment={value()} size={Math.min(props.width,props.height)}/></span></Match>
                <Match when={kind(value())==='sticker'}><StickerImage attachment={value()} size="100%"/></Match>
                <Match when={kind(value())==='motion'}><MotionBubble attachment={value()} onOpen={props.onOpen}/></Match>
            </Switch>
            <Show when={kind(value())==='image'||kind(value())==='sticker'}><button type="button" class="media-open" onPointerEnter={preload} onPointerDown={preload} onFocus={preload} aria-label={kind(value())==='sticker'?'Open sticker':`Open ${text(value().data.description)||text(value().data.filename)||'photo'}`} onClick={event=>props.onOpen?.(value(),event.currentTarget.getBoundingClientRect())}/></Show>
        </div></Match>
        <Match when={attachmentIsContact(value())}><ContactCard attachment={value()}/></Match>
        <Match when={true}><FileChip attachment={value()}/></Match>
    </Switch>}</Show>}><Show when={messageCardData(props.message)} fallback={<MessagePlaceholder message={props.message}/>}><MessageCard message={props.message}/></Show></Show>;
}
