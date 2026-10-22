import { createMemo, For, Show } from 'solid-js';
import { matchRanges } from './matches';
export function Highlight(props:{value:string;query:string}){
    const pieces=createMemo(()=>{
        const result:{value:string;match:boolean}[]=[];let original=0;
        for(const range of matchRanges(props.value,props.query)){if(range.start>original)result.push({value:props.value.slice(original,range.start),match:false});result.push({value:props.value.slice(range.start,range.end),match:true});original=range.end;}
        if(original<props.value.length)result.push({value:props.value.slice(original),match:false});return result;
    });
    return <For each={pieces()}>{piece=><Show when={piece.match} fallback={piece.value}><mark>{piece.value}</mark></Show>}</For>;
}
