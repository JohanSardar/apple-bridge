import {For} from 'solid-js';
import type {ThreadItem} from '../../data/types';
import {fields,text} from '../store/types';
import {MessageMedia} from './MessageMedia';
export function PlaceholderFixture(props:{items:readonly ThreadItem[];mode:string}){
 const items=()=>props.items.filter(message=>Boolean(message.data.balloon)&&!['link','app'].includes(text(fields(message.data.balloon).kind))).map(message=>{const balloon=fields(message.data.balloon),app=fields(balloon.app),kind=text(balloon.kind);return {...message,data:{...message.data,balloon:{...balloon,...(props.mode==='unknown'&&kind==='unknown'?{kind:'future_type',app_name:'Future app'}:{}),app:{...app,...(props.mode==='plain'?{image_attachment_id:undefined}:{}),...(props.mode==='duplicate'&&kind==='digital_touch'?{caption:' DIGITAL TOUCH '}:{}),...(['map','invalid'].includes(props.mode)&&kind==='find_my'?{url:props.mode==='map'?'https://maps.apple.com/?ll=0,0':'javascript:alert(1)'}:{}),...(props.mode==='unknown'&&kind==='unknown'?{caption:'A captured caption'}:{})}}}};});
 return <section class="media-fixture" aria-label="Unsupported message cards"><For each={items()}>{message=><div style={{position:'relative',width:'260px',height:'88px','border-radius':'18px'}}><MessageMedia message={message} part={{index:0,kind:'balloon'}} width={260} height={88}/></div>}</For></section>;
}
