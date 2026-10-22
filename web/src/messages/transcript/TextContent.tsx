import { createEffect, createMemo, createSignal, For, onCleanup, Show } from 'solid-js';
import {formattingSegments} from '../effects/format';
import {attachTextEffect} from '../effects/animations';
import type {TextEffect} from '../effects/data';
import { array, fields } from '../store/types';
type Mention={start:number;end:number;address:string};
type Segment={value:string;href?:string;mention?:string;styles:string[];effect?:TextEffect};
const detector=/(https?:\/\/[^\s<>]+|\+?\d[\d ()-]{6,}\d|\b\d{1,5}\s+(?:[\p{L}\d]+\s+){0,5}(?:Street|St|Avenue|Ave|Road|Rd|Lane|Ln|Drive|Dr|Boulevard|Blvd)\b)/giu;
type TextContentProps={value:string;arrival?:boolean;formatting?:unknown;offset?:number;mentions?:Mention[]};
const plainDetector=new RegExp(detector.source,'iu');
export function TextContent(props:TextContentProps){
    const plain=createMemo(()=>!array(props.formatting).length&&!props.mentions?.length&&!plainDetector.test(props.value));
    return <Show when={plain()} fallback={<RichTextContent {...props}/>}><span class="message-text">{props.value}</span></Show>;
}
function RichTextContent(props:TextContentProps){
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
        const formats=formattingSegments(props.value,array(props.formatting).map(value=>{const format=fields(value);return {...format,utf16_location:typeof format.utf16_location==='number'?format.utf16_location-(props.offset??0):format.utf16_location};}));
        for(const range of [...formats,...props.mentions??[]]){boundaries.add(clamp(range.start));boundaries.add(clamp(range.end));}
        const points=[...boundaries].filter(Number.isFinite).sort((a,b)=>a-b);
        const segments:Segment[]=points.slice(0,-1).map((start,index)=>{
            const end=points[index+1],mention=props.mentions?.find(value=>start>=value.start&&end<=value.end),link=links.find(value=>start>=value.start&&end<=value.end);
            return {value:props.value.slice(start,end),href:mention?undefined:link?.href,mention:mention?.address,styles:formats.filter(value=>start>=value.start&&end<=value.end).flatMap(value=>value.styles),effect:formats.find(value=>start>=value.start&&end<=value.end)?.effect};
        });
        const result:{href?:string;mention?:string;segments:Segment[]}[]=[];
        for(const segment of segments){const last=result.at(-1);if(last&&last.href===segment.href&&last.mention===segment.mention)last.segments.push(segment);else result.push({href:segment.href,mention:segment.mention,segments:[segment]});}
        return result;
    });
    const spans=(segments:Segment[])=> <For each={segments}>{segment=><TextSegment segment={segment} arrival={props.arrival}/>}</For>;
    return <><span class="message-text"><For each={groups()}>{group=><Show when={group.mention} fallback={<Show when={group.href} fallback={spans(group.segments)}><a href={group.href} target={group.href?.startsWith('http')?'_blank':undefined} rel="noopener noreferrer">{spans(group.segments)}</a></Show>}><button class="message-mention" aria-haspopup="dialog" onClick={()=>setContact({name:group.segments.map(segment=>segment.value).join(''),address:group.mention!})}>{spans(group.segments)}</button></Show>}</For></span><Show when={contact()}>{value=><dialog class="reactions-dialog" aria-label="Contact" ref={element=>queueMicrotask(()=>element.showModal())} onCancel={event=>{event.stopPropagation();setContact(undefined);}}><header><h2>{value().name}</h2><button class="icon-button" aria-label="Close contact" onClick={()=>setContact(undefined)}>×</button></header><p>{value().address}</p></dialog>}</Show></>;
}

function TextSegment(props:{segment:Segment;arrival?:boolean}){
    let element!:HTMLSpanElement;
    createEffect(()=>onCleanup(attachTextEffect(element,props.segment.effect,{arrival:Boolean(props.arrival)})));
    return <span ref={element} data-text-effect={props.segment.effect} style={{display:props.segment.effect?'inline-block':undefined}} classList={{'text-bold':props.segment.styles.includes('bold'),'text-italic':props.segment.styles.includes('italic'),'text-underline':props.segment.styles.includes('underline'),'text-strike':props.segment.styles.includes('strikethrough')}}>{props.segment.value}</span>;
}
