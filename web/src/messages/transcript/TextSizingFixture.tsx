import { createMemo, createSignal, onMount, Show } from 'solid-js';
import type { ThreadItem } from '../../data/types';
import { VirtualList, type VirtualListHandle } from '../../virtual/VirtualList';
import { MessageRow } from './Bubble';
import { layout } from './layout';
import './transcript.css';
export function TextSizingFixture(){
    const [wide,setWide]=createSignal(false),[ready,setReady]=createSignal(false);
    onMount(()=>{void document.fonts.load('13px InterVariable').then(()=>document.fonts.ready).then(()=>setReady(true));});
    let handle:VirtualListHandle|undefined;
    const words=['Earlier','update','garden','WWWWWWWWWWWWWWWW','éclair','مرحبا','שלום','東京の庭','👩🏽‍💻','🌷','office','AVATAR','e\u0301','https://example.com/garden'];
    let seed=123456789;
    const random=()=>{seed=(Math.imul(seed,1664525)+1013904223)>>>0;return seed/4294967296;};
    const items:ThreadItem[]=Array.from({length:501},(_,index)=>({id:'text-'+index,source:'messages',collection:'messages.message',version:1,data:{sent_at:new Date(1767225600000+index*10000).toISOString(),is_from_me:true,text:index===0?'[ab-test] Earlier update':Array.from({length:1+Math.floor(random()*30)},()=>words[Math.floor(random()*words.length)]).join(' ')},reactions:[],attachments:[],reply_count:0}));
    const context=document.createElement('canvas').getContext('2d')!;
    const rows=createMemo(()=>ready()?layout(items,new Map(),{phone:matchMedia('(max-width:699px)').matches,group:false,now:new Date(),scale:1,expanded:new Set(),measure:value=>{context.font=(matchMedia('(max-width:699px)').matches?'17':'13')+'px -apple-system, BlinkMacSystemFont, InterVariable, sans-serif';context.letterSpacing=matchMedia('(max-width:699px)').matches?'-.204px':'-.192px';return context.measureText(value).width;}},matchMedia('(max-width:699px)').matches?390:600):[]);
    return <><button onClick={()=>handle?.scrollToIndex(0)}>First text</button><button onClick={()=>setWide(value=>!value)}>Change shaping</button><Show when={ready()}><div class="messages-transcript" data-text-fixture style={{height:'65vh',width:'100%','max-width':'600px','font-family':wide()?'monospace':undefined,'font-variation-settings':wide()?'"wght" 750':undefined}}><VirtualList items={rows()} keyFor={row=>row.id} estimate={row=>row.height} label="Text sizing transcript" ref={value=>{handle=value;queueMicrotask(()=>handle?.scrollToIndex(0));}} render={row=><MessageRow row={row} edit={()=>undefined}/>}/></div></Show></>;
}
