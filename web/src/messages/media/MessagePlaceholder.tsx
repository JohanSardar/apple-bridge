import {createMemo,Show} from 'solid-js';
import type {ThreadItem} from '../../data/types';
import {Icon} from '../../design/Icon';
import {MediaImage} from './MediaImage';
import {messagePlaceholderData} from './placeholders';
import './placeholders.css';
function Glyph(props:{kind:string}){
 const path=()=>props.kind==='poll'?'M5 17V11M12 17V6M19 17V9':props.kind==='find_my'||props.kind==='location'?'M12 21s7-6 7-12a7 7 0 0 0-14 0c0 6 7 12 7 12Z':props.kind==='apple_cash'||props.kind==='apple_pay'?'M3 6h18v12H3ZM3 10h18M6 15h4':props.kind==='check_in'?'M12 3 4 6v6c0 5 8 9 8 9s8-4 8-9V6ZM8 12l3 3 5-6':props.kind==='digital_touch'?'M12 20 4 12C-1 6 7 1 12 8c5-7 13-2 8 4Z':props.kind==='handwriting'?'m4 18 2-6L16 2l6 6-10 10-6 2ZM6 12l6 6M14 4l6 6':'M4 4h16v12H9l-5 4ZM8 8h8M8 12h5';
 return <svg class="placeholder-glyph" viewBox="0 0 24 24" aria-hidden="true"><path d={path()}/><Show when={props.kind==='find_my'||props.kind==='location'}><circle cx="12" cy="9" r="2.5"/></Show></svg>;
}
export function MessagePlaceholder(props:{message:ThreadItem}){
 const data=createMemo(()=>messagePlaceholderData(props.message));
 return <div class="message-placeholder" role="group" aria-label={data().title} data-placeholder-kind={data().kind}><span class="placeholder-art"><Glyph kind={data().kind}/><Show when={data().art}>{attachment=><MediaImage attachment={attachment()} variant={data().variant} alt="Captured preview" interactive={false}/>}</Show></span><span class="placeholder-copy"><strong>{data().title}</strong><Show when={data().caption}><span class="placeholder-caption">{data().caption}</span></Show><span class="placeholder-hint">{data().hint}</span></span><Show when={data().mapURL}>{url=><><Icon name="chevron" class="placeholder-chevron"/><a class="placeholder-open" href={url()} target="_blank" rel="noopener noreferrer" aria-label="Open in Maps" title="Open in Maps"/></>}</Show></div>;
}
