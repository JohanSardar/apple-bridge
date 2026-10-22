import {For} from 'solid-js';
import type {ThreadItem} from '../../data/types';
import {fields,text} from '../store/types';
import {MessageMedia} from './MessageMedia';
export function AudioFixture(props:{items:readonly ThreadItem[];plain?:boolean}){
 const entries=()=>props.items.flatMap(message=>message.attachments.filter(attachment=>text(attachment.data.mime_type).startsWith('audio/')).map(attachment=>({message:props.plain?{...message,data:{...message.data,audio:{...fields(message.data.audio),transcription:undefined,kept:false,played_at:undefined}}}:message,attachment})));
 return <section class="media-fixture" aria-label="Audio messages"><For each={entries()}>{entry=><div data-audio-slot style={{position:'relative',width:'280px',height:props.plain?'76px':'120px','border-radius':'18px',background:entry.message.data.is_from_me?'var(--blue)':'var(--background-secondary)',color:entry.message.data.is_from_me?'white':'var(--label)'}}><MessageMedia message={entry.message} part={{index:0,kind:'attachment',attachment_id:entry.attachment.id}} width={280} height={props.plain?76:120}/></div>}</For></section>;
}
