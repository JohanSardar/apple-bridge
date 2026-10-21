import { createMemo, createSignal, For, Show } from 'solid-js';
import { array, fields, text } from '../store/types';
type Mention={start:number;end:number;address:string};
type Segment={value:string;href?:string;mention?:string;styles:string[]};
const detector=/(https?:\/\/[^\s<>]+|\+?\d[\d ()-]{6,}\d|\b\d{1,5}\s+(?:[\p{L}\d]+\s+){0,5}(?:Street|St|Avenue|Ave|Road|Rd|Lane|Ln|Drive|Dr|Boulevard|Blvd)\b)/giu;
export function TextContent(props:{value:string;formatting?:unknown;offset?:number;mentions?:Mention[]}){
    const [contact,setContact]=createSignal<{name:string;address:string}>();
    const groups=createMemo(()=>{
        const boundaries=new Set([0,props.value.length]);
        const clamp=(value:number)=>Math.max(0,Math.min(props.value.length,value));
        const links=[...props.value.matchAll(detector)].map(match=>{
            const value=match[0].startsWith('http')?match[0].replace(/[.,!?:;]+$/u,''):match[0];
            const start=match.index!;boundaries.add(start);boundaries.add(start+value.length);
            const href=value.startsWith('http')?value:/^[+\d ()-]+$/u.test(value)?'tel:'+value.replace(/[^+\d]/gu,''):'https://maps.apple.com/?q='+encodeURIComponent(value);
            return {start,end:start+value.length,href};
        });
        const formats=array(props.formatting).map(value=>{
            const format=fields(value),start=Number(format.utf16_location)-(props.offset??0);
            return {start,end:start+Number(format.utf16_length),styles:array(format.styles).map(text)};
        });
        for(const range of [...formats,...props.mentions??[]]){boundaries.add(clamp(range.start));boundaries.add(clamp(range.end));}
        const points=[...boundaries].filter(Number.isFinite).sort((a,b)=>a-b);
        const segments:Segment[]=points.slice(0,-1).map((start,index)=>{
            const end=points[index+1],mention=props.mentions?.find(value=>start>=value.start&&end<=value.end),link=links.find(value=>start>=value.start&&end<=value.end);
            return {value:props.value.slice(start,end),href:mention?undefined:link?.href,mention:mention?.address,styles:formats.filter(value=>start>=value.start&&end<=value.end).flatMap(value=>value.styles)};
        });
        const result:{href?:string;mention?:string;segments:Segment[]}[]=[];
        for(const segment of segments){const last=result.at(-1);if(last&&last.href===segment.href&&last.mention===segment.mention)last.segments.push(segment);else result.push({href:segment.href,mention:segment.mention,segments:[segment]});}
        return result;
    });
    const spans=(segments:Segment[])=> <For each={segments}>{segment=><span classList={{'text-bold':segment.styles.includes('bold'),'text-italic':segment.styles.includes('italic'),'text-underline':segment.styles.includes('underline'),'text-strike':segment.styles.includes('strikethrough')}}>{segment.value}</span>}</For>;
    return <><span class="message-text"><For each={groups()}>{group=><Show when={group.mention} fallback={<Show when={group.href} fallback={spans(group.segments)}><a href={group.href} target={group.href?.startsWith('http')?'_blank':undefined} rel="noopener noreferrer">{spans(group.segments)}</a></Show>}><button class="message-mention" aria-haspopup="dialog" onClick={()=>setContact({name:group.segments.map(segment=>segment.value).join(''),address:group.mention!})}>{spans(group.segments)}</button></Show>}</For></span><Show when={contact()}>{value=><dialog class="reactions-dialog" aria-label="Contact" ref={element=>queueMicrotask(()=>element.showModal())} onCancel={event=>{event.stopPropagation();setContact(undefined);}}><header><h2>{value().name}</h2><button class="icon-button" aria-label="Close contact" onClick={()=>setContact(undefined)}>×</button></header><p>{value().address}</p></dialog>}</Show></>;
}
