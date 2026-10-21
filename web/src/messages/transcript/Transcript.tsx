import { batch, createEffect, createMemo, createSignal, onCleanup, onMount, Show, untrack } from 'solid-js';
import { createStore, reconcile } from 'solid-js/store';
import { auth } from '../../auth/session';
import { appearance } from '../../app/preferences';
import { VirtualList, type VirtualListHandle } from '../../virtual/VirtualList';
import { beginWork } from '../../motion/work';
import { createThreadWindow } from '../store/thread';
import { createHandlesCache } from '../store/handles';
import { names, wording } from '../format';
import { array, fields, sender, text } from '../store/types';
import { layout, type TranscriptRow } from './layout';
import { MessageRow } from './Bubble';
import './transcript.css';
export default function Transcript(props:{chatID:string;messageID?:string;title:(value:string)=>void}){
    const live=auth.live()!,thread=createThreadWindow(live,props.chatID,props.messageID??'unread'),handles=createHandlesCache(live);
    const metadata=live.subscribe('record',{collection:'messages.chat',id:props.chatID});
    const record=createMemo(()=>metadata.data().records?.find(value=>value.id===props.chatID));
    const participants=createMemo(()=>array(record()?.data.participants).map(value=>text(value)||text(fields(value).id)).filter(Boolean));
    const handleMap=createMemo(()=>handles.read());
    const [viewportHeight,setViewportHeight]=createSignal(window.innerHeight),[width,setWidth]=createSignal(0),[ready,setReady]=createSignal(false),[loadingVisible,setLoadingVisible]=createSignal(false),[expanded,setExpanded]=createSignal<ReadonlySet<string>>(new Set()),[focused,setFocused]=createSignal<string>(),[list,setList]=createSignal<VirtualListHandle>(),[settled,setSettled]=createSignal(false),[pendingJump,setPendingJump]=createSignal<string|undefined>(props.messageID??'unread');
    const [rows,setRows]=createStore<{scope:string;items:TranscriptRow[]}>({scope:'',items:[]});
    let measuredFont='',measuredSpacing='';
    let container!:HTMLDivElement,frame=0,jumpFrame=0,loadingTimer:ReturnType<typeof setTimeout>|undefined;
    const canvas=document.createElement('canvas'),context=canvas.getContext('2d')!,measures=new Map<string,number>();
    createEffect(()=>{handles.ensure([...participants(),...thread.items().flatMap(item=>{const event=fields(item.data.group_event);return[sender(item),text(event.actor_handle_id),text(event.target_handle_id),text(event.handle_id),...(item.reactions??[]).map(sender)];})]);});
    createEffect(()=>{const data=record()?.data,map=handleMap();if(data)props.title(text(data.display_name)||names(participants().map(id=>map.get(id)?.display_name??'Unknown sender'))||'Messages');});
    createEffect(()=>{clearTimeout(loadingTimer);if(thread.loading())loadingTimer=setTimeout(()=>setLoadingVisible(true),150);else setLoadingVisible(false);});
    createEffect(()=>{
        const state=thread.view(),data=record()?.data,map=handleMap(),size=width(),viewport=viewportHeight(),scale=appearance.textSize(),open=expanded();
        cancelAnimationFrame(frame);
        if(!data||!size)return;
        frame=requestAnimationFrame(()=>{
            const finish=beginWork('transcript-layout'),phone=matchMedia('(max-width:699px)').matches;
            const style=getComputedStyle(container),font=`${(phone?17:13)*scale}px ${style.fontFamily}`;
            if(measuredFont!==font||measuredSpacing!==style.letterSpacing){context.font=font;context.letterSpacing=style.letterSpacing;measuredFont=font;measuredSpacing=style.letterSpacing;measures.clear();}
            const measure=(value:string,variant?:{weight?:number;italic?:boolean})=>{const key=(variant?.weight??400)+':'+Boolean(variant?.italic)+':'+value;let result=measures.get(key);if(result===undefined){context.font=`${variant?.italic?'italic ':''}${variant?.weight??400} ${font}`;result=context.measureText(value).width;if(measures.size>10000)measures.clear();measures.set(key,result);}return result;};
            const next=layout(state.items,map,{phone,viewportHeight:viewport,group:data.style==='group'||participants().length>1,service:text(data.service),atLatest:state.newer===0,now:new Date(),scale,measure,expanded:open},size);
            batch(()=>{setRows('items',reconcile(next,{key:'id',merge:true}));setRows('scope',state.anchor);setReady(true);});finish();
        });
    });
    let requested=props.messageID;
    createEffect(()=>{const target=props.messageID;if(target&&target!==requested){requested=target;setSettled(false);setPendingJump(target);thread.jump(target);}});
    createEffect(()=>{
        const api=list(),pending=pendingJump();
        if(!api||!ready()||thread.loading()||rows.scope!==thread.view().anchor||settled()&&!pending||!rows.items.length)return;
        const target=pending==='unread'?thread.unread():pending;
        const index=target==='oldest'?0:!target||target==='latest'?rows.items.length-1:rows.items.findIndex(row=>row.id===target);
        if(index<0)return;
        cancelAnimationFrame(jumpFrame);
        jumpFrame=requestAnimationFrame(()=>{api.scrollToIndex(index,!target||target==='latest'?'end':'start');setFocused(rows.items[index]?.id);batch(()=>{setPendingJump(undefined);setSettled(true);});});
    });
    onMount(()=>{const resize=()=>setViewportHeight(window.visualViewport?.height??window.innerHeight);resize();window.addEventListener('resize',resize);window.visualViewport?.addEventListener('resize',resize);onCleanup(()=>{window.removeEventListener('resize',resize);window.visualViewport?.removeEventListener('resize',resize);});setWidth(container.clientWidth);const observer=new ResizeObserver(entries=>setWidth(entries[0].contentRect.width));observer.observe(container);onCleanup(()=>observer.disconnect());});
    onCleanup(()=>{metadata.dispose();cancelAnimationFrame(frame);cancelAnimationFrame(jumpFrame);clearTimeout(loadingTimer);});
    function latest(){setSettled(false);setPendingJump('latest');thread.latest();}
    const edit=(id:string)=>setExpanded(previous=>{const next=new Set(previous);if(next.has(id))next.delete(id);else next.add(id);return next;});
    return <div ref={container} class="messages-transcript" classList={{group:record()?.data.style==='group'||participants().length>1}} data-window-size={thread.items().length} data-window-anchor={rows.scope} data-window-start={rows.items[0]?.id} data-window-end={rows.items.at(-1)?.id} aria-busy={!ready()||thread.loading()}><Show when={rows.items.length} fallback={<div class="empty quiet-empty"><Show when={loadingVisible()}><p>Loading messages…</p></Show><Show when={ready()&&!thread.loading()}><p>No Messages</p></Show></div>}><VirtualList items={rows.items.slice()} keyFor={row=>row.id} estimate={row=>row.height} fixedHeights bottomAnchored={settled()} windowKey={rows.scope} label="Messages transcript" selectedKey={focused()} ref={setList} onActive={row=>setFocused(row.id)} onVisible={(start,end)=>{if(untrack(settled))thread.visible(start,end);}} render={row=><MessageRow row={row} edit={edit}/>}/></Show><Show when={thread.newer()>0}><button class="new-messages-pill glass" onClick={latest}>{wording.newMessages(thread.newer())}</button></Show></div>;
}
