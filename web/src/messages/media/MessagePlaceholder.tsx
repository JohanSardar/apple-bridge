import {createMemo,Show} from 'solid-js';
import type {ThreadItem} from '../../data/types';
import {Icon} from '../../design/Icon';
import {MediaImage} from './MediaImage';
import {messagePlaceholderData} from './placeholders';
import './placeholders.css';
function Glyph(props:{kind:string}){
 const symbol=()=>props.kind==='find_my'?'location':props.kind==='apple_pay'?'apple_cash':['poll','location','apple_cash','check_in','digital_touch','handwriting'].includes(props.kind)?props.kind:'unknown';
 return <svg class="placeholder-glyph" viewBox="0 0 24 24" aria-hidden="true"><use width="24" height="24" href={'/icons.svg#message-'+symbol()}/></svg>;
}
export function MessagePlaceholder(props:{message:ThreadItem}){
 const data=createMemo(()=>messagePlaceholderData(props.message));
 return <div class="message-placeholder" role="group" aria-label={data().title} data-placeholder-kind={data().kind}><span class="placeholder-art"><Glyph kind={data().kind}/><Show when={data().art}>{attachment=><MediaImage attachment={attachment()} variant={data().variant} alt="Captured preview" interactive={false}/>}</Show></span><span class="placeholder-copy"><strong>{data().title}</strong><Show when={data().caption}><span class="placeholder-caption">{data().caption}</span></Show><span class="placeholder-hint">{data().hint}</span></span><Show when={data().mapURL}>{url=><><Icon name="chevron" class="placeholder-chevron"/><a class="placeholder-open" href={url()} target="_blank" rel="noopener noreferrer" aria-label="Open in Maps" title="Open in Maps"/></>}</Show></div>;
}
