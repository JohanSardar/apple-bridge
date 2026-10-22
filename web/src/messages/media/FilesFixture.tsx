import {For} from 'solid-js';
import type {ThreadItem} from '../../data/types';
import {fields,text} from '../store/types';
import {MessageMedia} from './MessageMedia';
export function FilesFixture(props:{items:readonly ThreadItem[];pending?:boolean}){
 const entries=()=>props.items.flatMap(message=>message.attachments.filter(attachment=>text(attachment.data.mime_type)==='application/pdf'||/vcard/iu.test(text(attachment.data.mime_type))).map(value=>{const attachment=props.pending&&text(value.data.mime_type)==='application/pdf'?{...value,data:{...value.data,blob:{...fields(value.data.blob),state:'pending'}}}:value;return {message:{...message,attachments:[attachment]},attachment};}));
 return <section class="media-fixture" aria-label="Files and contacts"><For each={entries()}>{entry=><div style={{position:'relative',width:'260px',height:'88px','border-radius':'18px'}}><MessageMedia message={entry.message} part={{index:0,kind:'attachment',attachment_id:entry.attachment.id}} width={260} height={88}/></div>}</For></section>;
}
