import { batch, createEffect, createMemo, createSignal, onCleanup, onMount, Show, untrack, type Accessor, type JSX } from 'solid-js';
import { createStore, reconcile } from 'solid-js/store';
import type { BridgeRecord, Cursor, ThreadItem } from '../../data/types';
import { router } from '../../app/router';
import { ChatMediaViewer } from '../media/ChatMediaViewer';
import { auth } from '../../auth/session';
import { appearance } from '../../app/preferences';
import { VirtualList, type VirtualListHandle } from '../../virtual/VirtualList';
import {enterNewPill} from '../../motion/transcript';
import { beginWork } from '../../motion/work';
import { createThreadWindow } from '../store/thread';
import { createHandlesCache } from '../store/handles';
import { names, wording } from '../format';
import { array, fields, sender, text } from '../store/types';
import { commandPartIndex, commandRenderKey, layout, type TranscriptRow } from './layout';
import { MessageRow } from './Bubble';
import { createTextMeasurer } from './measure';
import { createFontWatcher } from './fonts';
import {EffectsLayer} from '../effects/EffectsLayer';
import {ThreadOverlay} from './ThreadOverlay';
import './transcript.css';
class PresentationLeaf{constructor(value:object){Object.assign(this,value);Object.freeze(this);}}
export type TranscriptProps={
    chatID:string;
    threadOriginatorID?:string;
    onShowInChat?:(id:string)=>void;
    messageID?:string;
    title:(value:string)=>void;
    pending?:Accessor<readonly ThreadItem[]>;
    replyingTo?:Accessor<string|undefined>;
    onOpen?:(attachment:BridgeRecord,origin:DOMRectReadOnly)=>void;
    onObserved?:(items:readonly ThreadItem[],cursor:Cursor|undefined)=>void;
    projectCaptured?:(items:readonly ThreadItem[],cursor:Cursor|undefined)=>readonly ThreadItem[];
    deliveryNotice?:Accessor<{count:number;command_ids:readonly string[];text:string}|undefined>;
    onCheckCommand?:(id:string)=>void;
    onCopyCommand?:(event:Event,id:string,partIndex?:number)=>void|Promise<void>;
    onMessageAction?:(event:MouseEvent|PointerEvent|KeyboardEvent,item:ThreadItem)=>void;
    composer?:JSX.Element;
};
export default function Transcript(props:TranscriptProps){
    const live=auth.live()!,thread=createThreadWindow(live,props.chatID,props.messageID??'unread',props.threadOriginatorID),handles=createHandlesCache(live);
    const metadata=live.subscribe('record',{collection:'messages.chat',id:props.chatID});
    const [captured,setCaptured]=createSignal(thread.view()),[viewer,setViewer]=createSignal<{attachment:BridgeRecord;origin:DOMRectReadOnly}>();
    const leaves=new WeakMap<object,object>(),renderKeys=new Map<string,string>();
    function opaque<T extends object>(value:T):T{let saved=leaves.get(value);if(!saved){saved=new PresentationLeaf(value);leaves.set(value,saved);}return saved as T;}
    const [replies,setReplies]=createSignal<{id:string;trigger:HTMLButtonElement}>();
    const closeReplies=(restoreFocus=true)=>{const trigger=replies()?.trigger;setReplies(undefined);if(restoreFocus)queueMicrotask(()=>trigger?.isConnected&&trigger.focus({preventScroll:true}));};
    const openReplies=(event:MouseEvent,item:ThreadItem)=>setReplies({id:item.id,trigger:event.currentTarget as HTMLButtonElement});
    const local=createMemo<readonly ThreadItem[]>(previous=>{const next=(!props.threadOriginatorID?props.pending?.():undefined)?.filter(item=>item.data.chat_id===props.chatID)??[];return next.length?next:previous.length?[]:previous;},[]);
    const displayed=createMemo<readonly ThreadItem[]>(previous=>{const state=captured(),projected=(!props.threadOriginatorID?props.projectCaptured?.(state.items,state.cursor):undefined)??state.items,next=[...projected,...local()].sort((a,b)=>Date.parse(text(a.data.command_sent_at)||a.data.sent_at)-Date.parse(text(b.data.command_sent_at)||b.data.sent_at)||(text(a.data.command_id)||a.id).localeCompare(text(b.data.command_id)||b.id)||(commandPartIndex(a)??-1)-(commandPartIndex(b)??-1)||a.id.localeCompare(b.id));return next.length===previous.length&&next.every((item,index)=>item===previous[index])?previous:next;},[]);
    const stickers=createMemo(()=>{const targets=new Map<string,BridgeRecord[]>();for(const item of displayed())for(const attachment of item.attachments??[]){const target=text(fields(attachment.data.sticker).target_message_id);if(!target||attachment.data.is_sticker!==true)continue;const values=targets.get(target)??[];values.push(attachment);targets.set(target,values);}return targets;});
    const notice=()=>props.deliveryNotice?.();
    const openMedia=(attachment:BridgeRecord,origin:DOMRectReadOnly)=>{if(props.onOpen)props.onOpen(attachment,origin);else setViewer({attachment,origin});};
    const originFor=(attachment:BridgeRecord)=>container.querySelector<HTMLElement>('[data-media-origin="'+CSS.escape(attachment.id)+'"]')?.getBoundingClientRect();
    const [keyboardFocus,setKeyboardFocus]=createSignal(false);
    const motionPreference=matchMedia('(prefers-reduced-motion:reduce)'),[reduceMotion,setReduceMotion]=createSignal(motionPreference.matches);
    const motionChanged=()=>setReduceMotion(motionPreference.matches);motionPreference.addEventListener('change',motionChanged);onCleanup(()=>motionPreference.removeEventListener('change',motionChanged));
    let threadIntroStarted=false,threadIntroFrame=0;
    const threadIntro=new Map<string,number>();
    const takeThreadIndex=(id:string)=>{const index=threadIntro.get(id);if(index===undefined)return;if(!threadIntroFrame)threadIntroFrame=requestAnimationFrame(()=>threadIntro.clear());threadIntro.delete(id);return index;};
    let previousLocal='',previousLocalIDs=new Set<string>();
    let arrivalAnchor='',arrivalEpoch='',arrivalTime=-Infinity,arrivalKnown=new Set<string>(),arrivalReady=false;
    const arrivals=new Map<string,number>();
    const takeArrival=(id:string)=>{const time=arrivals.get(id);arrivals.delete(id);return time!==undefined&&performance.now()-time<1000;};
    function observeArrivals(state:ReturnType<typeof thread.view>){
        const now=performance.now(),epoch=state.cursor?.epoch??arrivalEpoch;
        for(const [id,time] of arrivals)if(now-time>=1000)arrivals.delete(id);
        if(arrivalReady&&state.anchor===arrivalAnchor&&epoch===arrivalEpoch&&state.newer===0&&!document.hidden){
            for(const item of state.items)if(!arrivalKnown.has(item.id)&&!item.data.command_id&&Date.parse(item.data.sent_at)>=arrivalTime)arrivals.set(item.id,now);
        }
        arrivalReady=Boolean(state.cursor)||arrivalReady&&state.anchor===arrivalAnchor&&epoch===arrivalEpoch;
        arrivalKnown=new Set(state.items.map(item=>item.id));for(const id of arrivals.keys())if(!arrivalKnown.has(id))arrivals.delete(id);arrivalAnchor=state.anchor;arrivalEpoch=epoch;
        arrivalTime=Math.max(...state.items.map(item=>Date.parse(item.data.sent_at)),-Infinity);
    }
    const record=createMemo(()=>metadata.data().records?.find(value=>value.id===props.chatID));
    createEffect(()=>{const state=thread.view(),metadataReady=record();untrack(()=>batch(()=>{observeArrivals(state);if(metadataReady&&!props.threadOriginatorID)props.onObserved?.(state.items,state.cursor);setCaptured(state);}));});
    const participants=createMemo(()=>array(record()?.data.participants).map(value=>text(value)||text(fields(value).id)).filter(Boolean));
    const handleMap=createMemo(()=>handles.read());
    const [viewportHeight,setViewportHeight]=createSignal(window.innerHeight),[width,setWidth]=createSignal(0),[ready,setReady]=createSignal(false),[loadingVisible,setLoadingVisible]=createSignal(false),[expanded,setExpanded]=createSignal<ReadonlySet<string>>(new Set()),[focused,setFocused]=createSignal<string>(),[list,setList]=createSignal<VirtualListHandle>(),[settled,setSettled]=createSignal(false),[pendingJump,setPendingJump]=createSignal<string|undefined>(props.messageID??'unread');
    const [rows,setRows]=createStore<{scope:string;items:TranscriptRow[]}>({scope:'',items:[]});
    let container!:HTMLDivElement,fontProbe!:HTMLSpanElement,measurementHost!:HTMLDivElement,frame=0,jumpFrame=0,loadingTimer:ReturnType<typeof setTimeout>|undefined;
    const measurer=createTextMeasurer(),[fontRevision,setFontRevision]=createSignal(0),[measurementKey,setMeasurementKey]=createSignal('');
    const fonts=createFontWatcher(()=>{measurer.clear();setFontRevision(value=>value+1);});
    createEffect(()=>{handles.ensure([...participants(),...thread.items().flatMap(item=>{const event=fields(item.data.group_event);return[sender(item),text(event.actor_handle_id),text(event.target_handle_id),text(event.handle_id),...(item.reactions??[]).map(sender)];})]);});
    createEffect(()=>{const data=record()?.data,map=handleMap();if(data)props.title(text(data.display_name)||names(participants().map(id=>map.get(id)?.display_name??'Unknown sender'))||'Messages');});
    createEffect(()=>{clearTimeout(loadingTimer);if(thread.loading())loadingTimer=setTimeout(()=>setLoadingVisible(true),150);else setLoadingVisible(false);});
    createEffect(()=>{
        const state=captured(),items=displayed(),pending=local(),data=record()?.data,map=handleMap(),size=width(),viewport=viewportHeight(),scale=appearance.textSize(),open=expanded(),reduced=reduceMotion();
        const signature=pending.map(item=>item.id+':'+text(item.data.send_state)).join('|'),immediate=signature!==previousLocal&&(Boolean(signature)||Boolean(previousLocal)),added=pending.some(item=>!previousLocalIDs.has(item.id));previousLocal=signature;previousLocalIDs=new Set(pending.map(item=>item.id));
        fontRevision();
        cancelAnimationFrame(frame);
        if(!data||!size)return;
        const commit=()=>{
            const finish=beginWork('transcript-layout'),phone=matchMedia('(max-width:699px)').matches;
            const textNode=container.querySelector<HTMLElement>('.bubble:not(.emoji-only):not(.media-reserved) .message-text')??fontProbe;fonts.watch(textNode,fontProbe);const descriptor=measurer.prepare(textNode,measurementHost);
            const prefs={phone,reduceMotion:reduced,selectedMessageID:props.messageID,viewportHeight:viewport,group:data.style==='group'||participants().length>1,service:text(data.service),atLatest:state.newer===0,now:new Date(),scale,measure:measurer.measure,expanded:open};
            let next=layout(items,map,prefs,size);
            if(measurer.flush())next=layout(items,map,prefs,size);
            const counts=new Map<string,number>();for(const row of next){const commandID=text(row.item.data.command_id);row.key=renderKeys.get(row.id)??(commandID?renderKeys.get(commandRenderKey(row.item)):undefined)??row.key;counts.set(row.key,(counts.get(row.key)??0)+1);}
            for(const row of next){if((counts.get(row.key)??0)>1)row.key+=':'+row.id;renderKeys.set(row.id,row.key);row.item=opaque(row.item);if(row.members)row.members=row.members.map(opaque);for(const bubble of row.bubbles){bubble.part=opaque(bubble.part);if(bubble.mediaAttachments)bubble.mediaAttachments=bubble.mediaAttachments.map(opaque);bubble.reactions=bubble.reactions.map(opaque);}}
            const present=new Set(next.map(row=>row.id));for(const id of renderKeys.keys())if(!present.has(id))renderKeys.delete(id);
            if(props.threadOriginatorID&&!threadIntroStarted&&next.length){threadIntroStarted=true;next.forEach((row,index)=>threadIntro.set(row.id,index));}
            batch(()=>{setMeasurementKey(descriptor+':'+untrack(fontRevision)+':'+size+':'+viewport);setRows('items',reconcile(next,{key:'key',merge:true}));setRows('scope',state.anchor);setReady(true);});
            if(added)queueMicrotask(()=>{if(state.newer>0)thread.latest();list()?.scrollToIndex(rows.items.length-1,'end');});finish();
        };
        if(immediate)commit();else frame=requestAnimationFrame(commit);
    });
    let requested=props.messageID;
    createEffect(()=>{const target=props.messageID;if(target&&target!==requested){requested=target;closeReplies();setSettled(false);setPendingJump(target);thread.jump(target);}});
    createEffect(()=>{
        const api=list(),pending=pendingJump();
        if(!api||!ready()||thread.loading()||rows.scope!==thread.view().anchor||settled()&&!pending||!rows.items.length)return;
        const target=pending==='unread'?thread.unread():pending;
        const index=target==='oldest'?0:!target||target==='latest'?rows.items.length-1:rows.items.findIndex(row=>row.id===target||row.members?.some(item=>item.id===target));
        if(index<0)return;
        cancelAnimationFrame(jumpFrame);
        jumpFrame=requestAnimationFrame(()=>{api.scrollToIndex(index,!target||target==='latest'?'end':'start');setFocused(rows.items[index]?.key);batch(()=>{setPendingJump(undefined);setSettled(true);});});
    });
    onMount(()=>{fonts.watch(fontProbe,fontProbe);onCleanup(fonts.dispose);const resize=()=>setViewportHeight(window.visualViewport?.height??window.innerHeight);resize();window.addEventListener('resize',resize);window.visualViewport?.addEventListener('resize',resize);onCleanup(()=>{window.removeEventListener('resize',resize);window.visualViewport?.removeEventListener('resize',resize);});setWidth(container.clientWidth);const observer=new ResizeObserver(entries=>setWidth(entries[0].contentRect.width));observer.observe(container);onCleanup(()=>observer.disconnect());});
    onCleanup(()=>{metadata.dispose();cancelAnimationFrame(threadIntroFrame);cancelAnimationFrame(frame);cancelAnimationFrame(jumpFrame);clearTimeout(loadingTimer);});
    function showInChat(id:string){if(props.threadOriginatorID){props.onShowInChat?.(id);return;}closeReplies();setSettled(false);setPendingJump(id);thread.jump(id);router.navigate("/messages/"+encodeURIComponent(props.chatID)+"/"+encodeURIComponent(id));}
    function latest(){setSettled(false);setPendingJump('latest');thread.latest();}
    const edit=(id:string)=>setExpanded(previous=>{const next=new Set(previous);if(next.has(id))next.delete(id);else next.add(id);return next;});
    return <div ref={container} class="messages-transcript" inert={Boolean(replies())} onPointerDown={()=>setKeyboardFocus(false)} onKeyDown={event=>{if(["ArrowUp","ArrowDown","Home","End","PageUp","PageDown"].includes(event.key))setKeyboardFocus(true);}} classList={{"keyboard-focus":keyboardFocus(),group:record()?.data.style==='group'||participants().length>1}} data-window-size={thread.items().length} data-window-anchor={rows.scope} data-window-start={rows.items[0]?.id} data-window-end={rows.items.at(-1)?.members?.at(-1)?.id??rows.items.at(-1)?.id} aria-busy={!ready()||thread.loading()}><div class="transcript-viewport"><div class="text-measure-host" aria-hidden="true"><span ref={fontProbe} class="text-font-probe">AgMWfi👩🏽‍💻東京مرحبا</span><div ref={measurementHost}/></div><Show when={rows.items.length} fallback={<div class="empty quiet-empty"><Show when={loadingVisible()}><p>Loading messages…</p></Show><Show when={ready()&&!thread.loading()}><p>No Messages</p></Show></div>}><VirtualList items={rows.items.slice()} keyFor={row=>row.key} estimate={row=>row.height} measurementKey={measurementKey()} bottomAnchored={settled()&&local().length===0} windowKey={rows.scope} label="Messages transcript" selectedKey={focused()} ref={setList} onActive={row=>setFocused(row.key)} onVisible={(start,end)=>{if(!untrack(settled))return;const raw=new Map(untrack(thread.items).map((item,index)=>[item.id,index])),visible=rows.items.slice(start,end).flatMap(row=>(row.members??[row.item]).flatMap(item=>raw.has(item.id)?[raw.get(item.id)!]:[]));if(visible.length)thread.visible(Math.min(...visible),Math.max(...visible)+1);}} render={row=><MessageRow row={row} takeArrival={takeArrival} takeThreadIndex={takeThreadIndex} highlight={row.id===props.messageID} handles={handleMap()} stickers={stickers().get(row.id)} edit={edit} onOpen={openMedia} onReplies={props.threadOriginatorID?undefined:openReplies} onMessageAction={props.onMessageAction} onCheckCommand={props.onCheckCommand} canCheck={live.status()==='connected'} onCopyCommand={props.onCopyCommand}/>}/></Show><Show when={thread.newer()>0}><NewMessagesPill onClick={latest} count={thread.newer()}/></Show></div><Show when={notice()}>{value=><aside class="delivery-notice" role="status" aria-label="Uncertain delivery"><span>{value().text}</span><button disabled={!props.onCheckCommand||live.status()!=='connected'} onClick={()=>value().command_ids.forEach(id=>props.onCheckCommand?.(id))}>Check</button></aside>}</Show>{props.composer}<Show when={!props.threadOriginatorID}><EffectsLayer/></Show><Show keyed when={viewer()}>{value=><ChatMediaViewer chatId={props.chatID} attachment={value.attachment} origin={value.origin} originFor={originFor} onClose={()=>setViewer(undefined)} onShowInChat={attachment=>{const id=text(attachment.data.message_id);setViewer(undefined);if(id)showInChat(id);}}/>}</Show><Show keyed when={replies()}>{value=><ThreadOverlay onClose={closeReplies} replyingTo={props.replyingTo}><Transcript chatID={props.chatID} messageID={value.id} threadOriginatorID={value.id} onShowInChat={showInChat} title={()=>{}} onOpen={props.onOpen} onMessageAction={props.onMessageAction}/></ThreadOverlay>}</Show></div>;
}

function NewMessagesPill(props:{count:number;onClick:()=>void}){
    let element!:HTMLButtonElement;
    onMount(()=>onCleanup(enterNewPill(element)));
    return <button ref={element} class="new-messages-pill glass" onClick={props.onClick}>{wording.newMessages(props.count)}</button>;
}
