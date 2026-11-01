import { batch, createEffect, createMemo, createSignal, onCleanup, onMount, Show, untrack, type Accessor, type JSX } from 'solid-js';
import type { BridgeRecord, Cursor, ThreadItem } from '../../data/types';
import { router } from '../../app/router';
import { ChatMediaViewer } from '../media/ChatMediaViewer';
import { auth } from '../../auth/session';
import { appearance } from '../../app/preferences';
import {viewportHeight} from '../../app/viewport';
import { VirtualList, type VirtualListHandle } from '../../virtual/VirtualList';
import {enterNewPill} from '../../motion/transcript';
import { beginWork } from '../../motion/work';
import { createThreadWindow } from '../store/thread';
import { createHandlesCache } from '../store/handles';
import { names, wording } from '../format';
import { array, fields, text } from '../store/types';
import { chatService, messageParts, sender } from '../store/message';
import { commandRenderKey, createTranscriptLayout, type TranscriptRow } from './layout';
import { MessageRow } from './Bubble';
import { createTextMeasurer } from './measure';
import { createFontWatcher } from './fonts';
import { orderTranscriptItems } from './order';
import {EffectsLayer} from '../effects/EffectsLayer';
import {ThreadOverlay} from './ThreadOverlay';
import './transcript.css';
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
    onRetryCommand?:(event:Event,id:string,partIndex?:number)=>void;
    canRetryCommand?:(id:string,partIndex?:number)=>boolean;
    onCheckCommand?:(id:string)=>void;
    onCopyCommand?:(event:Event,id:string,partIndex?:number)=>void|Promise<void>;
    onMessageAction?:(event:MouseEvent|PointerEvent|KeyboardEvent,item:ThreadItem)=>void;
    onMessagePointer?:(event:PointerEvent,item:ThreadItem)=>void;
    composer?:JSX.Element;
};
export default function Transcript(props:TranscriptProps){
    const live=auth.live()!,thread=createThreadWindow(live,props.chatID,props.messageID??'unread',props.threadOriginatorID),handles=createHandlesCache(live);
    const metadata=live.subscribe('record',{collection:'messages.chat',id:props.chatID});
    const [captured,setCaptured]=createSignal(thread.view()),[viewer,setViewer]=createSignal<{attachment:BridgeRecord;origin:DOMRectReadOnly}>();
    const retainedViewer=createMemo<ReturnType<typeof viewer>>(previous=>{
        const value=viewer()??previous;
        if(!value)return;
        if(thread.items().some(item=>item.id===value.attachment.data.message_id&&item.data.chat_id===props.chatID&&!item.data.retracted_at&&!item.data.deleted_at&&item.attachments.some(attachment=>attachment.id===value.attachment.id)&&messageParts(item).some(part=>part.attachment_id===value.attachment.id&&!array(item.data.retracted_parts).includes(part.index))))return value;
        setViewer(undefined);
    });
    const originals=new WeakMap<object,ThreadItem>(),renderKeys=new Map<string,string>();
    function linkOriginals(raw:readonly ThreadItem[],projected:readonly ThreadItem[]){
        if(projected!==raw){
            const byID=new Map(raw.map(item=>[item.id,item]));
            for(const item of projected){const original=byID.get(item.id);if(original!==item&&original?.version===item.version)originals.set(item,original);}
        }
        return projected;
    }
    const dispatchAction=(event:MouseEvent|PointerEvent|KeyboardEvent,item:ThreadItem)=>props.onMessageAction?.(event,originals.get(item)??item);
    const dispatchPointer=(event:PointerEvent,item:ThreadItem)=>props.onMessagePointer?.(event,originals.get(item)??item);
    const [replies,setReplies]=createSignal<{id:string;trigger:HTMLButtonElement}>();
    const closeReplies=(restoreFocus=true)=>{const trigger=replies()?.trigger;setReplies(undefined);if(restoreFocus)queueMicrotask(()=>trigger?.isConnected&&trigger.focus({preventScroll:true}));};
    const openReplies=(event:MouseEvent,item:ThreadItem)=>setReplies({id:item.id,trigger:event.currentTarget as HTMLButtonElement});
    const sameItems=(a:readonly ThreadItem[],b:readonly ThreadItem[])=>a===b||a.length===b.length&&a.every((item,index)=>item===b[index]);
    const local=createMemo<readonly ThreadItem[]>(()=> (!props.threadOriginatorID?props.pending?.():undefined)?.filter(item=>item.data.chat_id===props.chatID)??[],[],{equals:sameItems});
    const projected=createMemo<readonly ThreadItem[]>(previous=>{const state=captured(),next=(!props.threadOriginatorID?props.projectCaptured?.(state.items,state.cursor):undefined)??state.items;return sameItems(next,previous)?previous:linkOriginals(state.items,next);},[]);
    const displayed=createMemo<readonly ThreadItem[]>(()=>orderTranscriptItems([...projected(),...local()]),[],{equals:sameItems});
    const stickers=createMemo(()=>{const targets=new Map<string,BridgeRecord[]>();for(const item of displayed())for(const attachment of item.attachments??[]){const target=text(fields(attachment.data.sticker).target_message_id);if(!target||attachment.data.is_sticker!==true)continue;const values=targets.get(target)??[];values.push(attachment);targets.set(target,values);}return targets;});
    const notice=()=>props.deliveryNotice?.();
    const openMedia=(attachment:BridgeRecord,origin:DOMRectReadOnly)=>{if(props.onOpen)props.onOpen(attachment,origin);else setViewer({attachment,origin});};
    const [keyboardFocus,setKeyboardFocus]=createSignal(false);
    const motionPreference=matchMedia('(prefers-reduced-motion:reduce)'),[reduceMotion,setReduceMotion]=createSignal(motionPreference.matches);
    const motionChanged=()=>setReduceMotion(motionPreference.matches);motionPreference.addEventListener('change',motionChanged);onCleanup(()=>motionPreference.removeEventListener('change',motionChanged));
    let threadIntroStarted=false,threadIntroFrame=0;
    const threadIntro=new Map<string,number>();
    const takeThreadIndex=(id:string)=>{const index=threadIntro.get(id);if(index===undefined)return;if(!threadIntroFrame)threadIntroFrame=requestAnimationFrame(()=>threadIntro.clear());threadIntro.delete(id);return index;};
    let previousLocal:readonly ThreadItem[]=[];
    let arrivalAnchor='',arrivalEpoch='',arrivalTime=-Infinity,arrivalKnown=new Set<string>(),arrivalReady=false;
    const arrivals=new Map<string,number>();
    const takeArrival=(id:string)=>{const time=arrivals.get(id);arrivals.delete(id);return time!==undefined&&performance.now()-time<1000;};
    function observeArrivals(state:ReturnType<typeof thread.view>){
        const now=performance.now(),epoch=state.cursor?.epoch??arrivalEpoch;
        for(const [id,time] of arrivals)if(now-time>=1000)arrivals.delete(id);
        if(arrivalReady&&state.anchor===arrivalAnchor&&epoch===arrivalEpoch&&state.newer===0&&!document.hidden){
            for(const item of state.items)if(!arrivalKnown.has(item.id)&&!item.data.command_id&&Date.parse(item.data.sent_at)>=arrivalTime)arrivals.set(item.id,now);
        }
        arrivalReady=state.newer===0&&(Boolean(state.cursor)||arrivalReady&&state.anchor===arrivalAnchor&&epoch===arrivalEpoch);
        arrivalKnown=new Set(state.items.map(item=>item.id));for(const id of arrivals.keys())if(!arrivalKnown.has(id))arrivals.delete(id);arrivalAnchor=state.anchor;arrivalEpoch=epoch;
        arrivalTime=Math.max(...state.items.map(item=>Date.parse(item.data.sent_at)),-Infinity);
    }
    const record=createMemo(()=>metadata.data().records?.find(value=>value.id===props.chatID));
    createEffect(()=>{const state=thread.view(),metadataReady=record();untrack(()=>batch(()=>{observeArrivals(state);if(metadataReady&&!props.threadOriginatorID)props.onObserved?.(state.items,state.cursor);setCaptured(state);if(state.cursor&&!metadataReady)thread.presented();}));});
    const participants=createMemo(()=>array(record()?.data.participants).map(value=>text(value)||text(fields(value).id)).filter(Boolean));
    const handleMap=createMemo(()=>handles.read());
    const rawIndexes=createMemo(()=>new Map(thread.items().map((item,index)=>[item.id,index])));

    const [width,setWidth]=createSignal(0),[ready,setReady]=createSignal(false),[loadingVisible,setLoadingVisible]=createSignal(false),[expanded,setExpanded]=createSignal<ReadonlySet<string>>(new Set()),[focused,setFocused]=createSignal<string>(),[list,setList]=createSignal<VirtualListHandle>(),[settled,setSettled]=createSignal(false),[pendingJump,setPendingJump]=createSignal<string|undefined>(props.messageID??'unread');
    const [followLatest,setFollowLatest]=createSignal(!props.messageID),[initialPrefetch,setInitialPrefetch]=createSignal(true);
    const [rows,setRows]=createSignal<{scope:string;items:TranscriptRow[]}>({scope:'',items:[]});
    let container!:HTMLDivElement,transcriptViewport!:HTMLDivElement,initialHeight=0,fontProbe!:HTMLSpanElement,measurementHost!:HTMLDivElement,frame=0,jumpFrame=0,sendScrollFrame=0,loadingTimer:ReturnType<typeof setTimeout>|undefined;
    let textMeasurementRevision=0;
    const layout=createTranscriptLayout(),measurer=createTextMeasurer(),[fontRevision,setFontRevision]=createSignal(0),[measurementKey,setMeasurementKey]=createSignal('');
    const fonts=createFontWatcher(()=>{measurer.clear();setFontRevision(value=>value+1);});
    createEffect(()=>{handles.ensure([...participants(),...thread.items().flatMap(item=>{const event=fields(item.data.group_event);return[sender(item),text(event.actor_handle_id),text(event.target_handle_id),text(event.handle_id),...(item.reactions??[]).map(sender)];})]);});
    createEffect(()=>{const data=record()?.data,map=handleMap();if(data)props.title(text(data.display_name)||names(participants().map(id=>map.get(id)?.display_name??'Unknown sender'))||'Messages');});
    createEffect(()=>{clearTimeout(loadingTimer);if(thread.loading())loadingTimer=setTimeout(()=>setLoadingVisible(true),150);else setLoadingVisible(false);});
    const layoutContext=createMemo(()=>JSON.stringify([Boolean(record()),captured().anchor,captured().newer===0,record()?.data.style,chatService(record()),participants().length]));
    createEffect(()=>{
        layoutContext();
        const state=untrack(captured),items=displayed(),pending=local(),data=untrack(record)?.data,map=handleMap(),loading=thread.loading(),prefetching=thread.prefetching(),size=width(),viewport=viewportHeight(),scale=appearance.textSize(),open=expanded(),reduced=reduceMotion();
        const immediate=pending!==previousLocal&&Boolean(pending.length||previousLocal.length),added=pending.some(item=>!previousLocal.some(previous=>previous.id===item.id));
        if(added){setFollowLatest(true);setPendingJump('latest');untrack(thread.latest);}
        fontRevision();
        cancelAnimationFrame(frame);
        if(!data||!size||(!untrack(ready)&&loading&&!pending.length))return;
        const staged=!immediate&&(untrack(initialPrefetch)||state.anchor!==untrack(()=>rows().scope));
        let prepared=0,descriptor:string|undefined,active=true;
        const phone=matchMedia('(max-width:699px)').matches;
        onCleanup(()=>{active=false;});
        const commit=()=>{
            if(!active)return;
            const finish=beginWork('transcript-layout');
            if(descriptor===undefined){const textNode=container.querySelector<HTMLElement>('.bubble:not(.emoji-only):not(.media-reserved) .message-text')??fontProbe;const style=getComputedStyle(textNode);fonts.watch(style);descriptor=measurer.prepare(style,measurementHost);}
            const prefs={phone,reduceMotion:reduced,selectedMessageID:props.messageID,viewportHeight:viewport,group:data.style==='group'||participants().length>1,service:chatService(record()),atLatest:state.newer===0,now:new Date(),scale,measure:measurer.measure,expanded:open};
            const layoutKey=descriptor+':'+untrack(fontRevision)+':'+textMeasurementRevision;
            if(staged){
                const started=performance.now();
                do{const end=Math.min(items.length,prepared+8);layout(items,map,prefs,size,layoutKey,{start:prepared,end});prepared=end;}while(prepared<items.length&&performance.now()-started<2);
                if(prepared<items.length){finish();frame=requestAnimationFrame(commit);return;}
            }
            let next=layout(items,map,prefs,size,layoutKey);
            if(measurer.flush()){textMeasurementRevision++;next=layout(items,map,prefs,size,descriptor+':'+untrack(fontRevision)+':'+textMeasurementRevision);}
            const counts=new Map<string,number>();for(const row of next){const commandID=text(row.item.data.command_id);row.key=renderKeys.get(row.id)??(commandID?renderKeys.get(commandRenderKey(row.item)):undefined)??row.key;counts.set(row.key,(counts.get(row.key)??0)+1);}
            for(const row of next){if((counts.get(row.key)??0)>1)row.key+=':'+row.id;renderKeys.set(row.id,row.key);}
            const present=new Set(next.map(row=>row.id));for(const id of renderKeys.keys())if(!present.has(id))renderKeys.delete(id);
            if(props.threadOriginatorID&&!threadIntroStarted&&next.length){threadIntroStarted=true;next.forEach((row,index)=>threadIntro.set(row.id,index));}
            finish();
            const publish=()=>{
                const finishPublish=beginWork('transcript-publish');
                previousLocal=pending;
                batch(()=>{setInitialPrefetch(value=>value&&prefetching);setMeasurementKey(descriptor+':'+untrack(fontRevision)+':'+size+':'+viewport);setRows({scope:state.anchor,items:next});setReady(true);});
                if(!next.length)thread.presented();
                if(added){cancelAnimationFrame(sendScrollFrame);sendScrollFrame=requestAnimationFrame(()=>list()?.scrollToIndex(rows().items.length-1,'end'));}
                finishPublish();
            };
            if(staged)frame=requestAnimationFrame(publish);else publish();
        };
        if(immediate)queueMicrotask(commit);else frame=requestAnimationFrame(commit);
    });
    const initialTarget=()=>{const pending=pendingJump();return pending==='unread'?thread.unread():pending;};
    const initialIndex=()=>{const target=initialTarget();return target==='oldest'?0:!target||target==='latest'?rows().items.length-1:rows().items.findIndex(row=>row.id===target||row.members?.some(item=>item.id===target));};
    let requested=props.messageID;
    createEffect(()=>{const target=props.messageID;if(target&&target!==requested){requested=target;closeReplies();setFollowLatest(false);setSettled(false);setPendingJump(target);thread.jump(target);}});
    createEffect(()=>{
        const api=list(),pending=pendingJump();
        cancelAnimationFrame(jumpFrame);
        if(!api||!ready()||thread.loading()||rows().scope!==thread.view().anchor||settled()&&!pending||!rows().items.length)return;
        const target=initialTarget(),index=initialIndex();
        if(index<0)return;
        cancelAnimationFrame(sendScrollFrame);
        jumpFrame=requestAnimationFrame(()=>{api.scrollToIndex(index,!target||target==='latest'?'end':'start');setFocused(rows().items[index]?.key);batch(()=>{setPendingJump(undefined);setSettled(true);});thread.presented();});
    });
    onMount(()=>{
        const editable=(target:EventTarget|null)=>target instanceof Element&&Boolean(target.closest('input,textarea,select,[contenteditable]:not([contenteditable="false"])'));
        const key=(event:KeyboardEvent)=>{if(editable(event.target)||event.isComposing||event.altKey||event.shiftKey||!(event.metaKey||event.ctrlKey)||!['ArrowUp','ArrowDown'].includes(event.key))return;event.preventDefault();event.stopPropagation();setKeyboardFocus(true);setSettled(false);const oldest=event.key==='ArrowUp';setFollowLatest(!oldest);setPendingJump(oldest?'oldest':'latest');if(oldest)thread.oldest();else thread.latest();};
        const ownerDocument=container.ownerDocument;
        const copy=(event:ClipboardEvent)=>{if(!container.contains(ownerDocument.activeElement)||editable(ownerDocument.activeElement)||editable(event.target)||window.getSelection()?.toString()||!event.clipboardData)return;const row=rows().items.find(value=>value.key===focused());if(!row)return;const value=text(row.item.data.text)||row.bubbles.map(bubble=>text(bubble.part.text)).filter(Boolean).join('\n');if(!value)return;event.clipboardData.setData('text/plain',value);event.preventDefault();};
        container.addEventListener('keydown',key,true);ownerDocument.addEventListener('copy',copy,true);onCleanup(()=>{container.removeEventListener('keydown',key,true);ownerDocument.removeEventListener('copy',copy,true);});
    });
    onMount(()=>{const observer=new ResizeObserver(entries=>batch(()=>{for(const entry of entries)if(entry.target===transcriptViewport){initialHeight=entry.contentRect.height;setWidth(entry.contentRect.width);}else fonts.resized(entry.contentRect);}));observer.observe(transcriptViewport);observer.observe(fontProbe);onCleanup(()=>{observer.disconnect();fonts.dispose();});});
    onCleanup(()=>{metadata.dispose();cancelAnimationFrame(threadIntroFrame);cancelAnimationFrame(frame);cancelAnimationFrame(jumpFrame);cancelAnimationFrame(sendScrollFrame);clearTimeout(loadingTimer);});
    function showInChat(id:string){if(props.threadOriginatorID){props.onShowInChat?.(id);return;}closeReplies();setFollowLatest(false);setSettled(false);setPendingJump(id);thread.jump(id);router.navigate("/messages/"+encodeURIComponent(props.chatID)+"/"+encodeURIComponent(id));}
    function latest(){setFollowLatest(true);setSettled(false);setPendingJump('latest');thread.latest();}
    const edit=(id:string)=>setExpanded(previous=>{const next=new Set(previous);if(next.has(id))next.delete(id);else next.add(id);return next;});
    return <div ref={container} class="messages-transcript" inert={Boolean(replies())} onPointerDown={()=>setKeyboardFocus(false)} onKeyDown={event=>{if(["ArrowUp","ArrowDown","Home","End","PageUp","PageDown"].includes(event.key))setKeyboardFocus(true);}} classList={{"keyboard-focus":keyboardFocus(),group:record()?.data.style==='group'||participants().length>1}} data-window-size={thread.items().length} data-window-anchor={rows().scope} data-window-start={rows().items[0]?.id} data-window-end={rows().items.at(-1)?.members?.at(-1)?.id??rows().items.at(-1)?.id} aria-busy={!ready()||thread.loading()||rows().scope!==thread.view().anchor}><div ref={transcriptViewport} class="transcript-viewport"><div class="text-measure-host" aria-hidden="true"><span ref={fontProbe} class="text-font-probe">AgMWfi</span><div ref={measurementHost}/></div><Show when={rows().items.length} fallback={<div class="empty quiet-empty"><Show when={loadingVisible()}><p>Loading messages…</p></Show><Show when={ready()&&!thread.loading()}><p>No Messages</p></Show></div>}><VirtualList initialBatchSize={1} initialHeight={initialHeight} initialPrefetch={initialPrefetch()} overscan={settled()&&!initialPrefetch()?2:0} initialKey={rows().items[initialIndex()]?.key} initialAlign={!initialTarget()||initialTarget()==='latest'?'end':'start'} items={rows().items} keyFor={row=>row.key} estimate={row=>row.height} measurementKey={measurementKey()} bottomAnchored={settled()&&followLatest()&&local().length===0&&!rows().items.some(row=>row.item.id.startsWith('command:'))} windowKey={rows().scope} label="Messages transcript" selectedKey={focused()} ref={setList} onActive={row=>setFocused(row.key)} onVisible={(start,end)=>{if(!untrack(settled)||untrack(()=>rows().scope)!==untrack(thread.view).anchor)return;const raw=untrack(rawIndexes),visible=rows().items.slice(start,end).flatMap(row=>(row.members??[row.item]).flatMap(item=>raw.has(item.id)?[raw.get(item.id)!]:[]));if(visible.length)thread.visible(Math.min(...visible),Math.max(...visible)+1);}} renderReactive={row=><MessageRow row={row()} takeArrival={takeArrival} takeThreadIndex={takeThreadIndex} highlight={row().id===props.messageID} handles={handleMap()} stickers={stickers().get(row().id)} edit={edit} onOpen={openMedia} onReplies={props.threadOriginatorID?undefined:openReplies} onMessageAction={props.onMessageAction?dispatchAction:undefined} onMessagePointer={props.onMessagePointer?dispatchPointer:undefined} onRetryCommand={props.onRetryCommand} canRetryCommand={props.canRetryCommand} onCheckCommand={props.onCheckCommand} canCheck={live.status()==='connected'} onCopyCommand={props.onCopyCommand}/>}/></Show><Show when={thread.newer()>0}><NewMessagesPill onClick={latest} count={thread.newer()}/></Show></div><Show when={notice()}>{value=><aside class="delivery-notice" role="status" aria-label="Uncertain delivery"><span>{value().text}</span><button disabled={!props.onCheckCommand||live.status()!=='connected'} onClick={()=>value().command_ids.forEach(id=>props.onCheckCommand?.(id))}>Check</button></aside>}</Show>{props.composer}<Show when={!props.threadOriginatorID}><EffectsLayer/></Show><Show when={retainedViewer()}>{value=><ChatMediaViewer open={Boolean(viewer())} chatId={props.chatID} attachment={value().attachment} origin={value().origin} sourceRoot={container} onClose={()=>setViewer(undefined)} onShowInChat={attachment=>{const id=text(attachment.data.message_id);setViewer(undefined);if(id)showInChat(id);}}/>}</Show><Show keyed when={replies()}>{value=><ThreadOverlay onClose={closeReplies} replyingTo={props.replyingTo}><Transcript chatID={props.chatID} messageID={value.id} threadOriginatorID={value.id} onShowInChat={showInChat} title={()=>{}} onOpen={props.onOpen} onMessageAction={props.onMessageAction} onMessagePointer={props.onMessagePointer}/></ThreadOverlay>}</Show></div>;
}

function NewMessagesPill(props:{count:number;onClick:()=>void}){
    let element!:HTMLButtonElement;
    onMount(()=>onCleanup(enterNewPill(element)));
    return <button ref={element} class="new-messages-pill glass" onClick={props.onClick}>{wording.newMessages(props.count)}</button>;
}
