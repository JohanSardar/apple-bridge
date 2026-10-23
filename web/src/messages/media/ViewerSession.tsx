import {Show,onCleanup,createSignal,createEffect,createMemo} from 'solid-js';
import {auth} from '../../auth/session';
import type {BridgeRecord} from '../../data/types';
import {createMediaCollection} from './collection';
import {MediaViewer} from './MediaViewer';
import type {ChatMediaViewerProps} from './ChatMediaViewer';
import {createHandlesCache} from '../store/handles';
import {fields,text} from '../store/types';
import {duration,header,name,wording} from '../format';
import {mediaSource} from './source';
export function ViewerSession(props:ChatMediaViewerProps){
    let hidden:HTMLElement|undefined,visibility='';
    const [selected,setSelected]=createSignal(props.attachment);
    const restore=()=>{if(hidden){hidden.style.visibility=visibility;hidden=undefined;}};
    const source=(attachment:BridgeRecord)=>(props.sourceRoot??document).querySelector<HTMLElement>(`[data-media-origin="${CSS.escape(attachment.id)}"]:not([inert],[inert] *)`);
    const hide=(element?:HTMLElement|null)=>{if(element){hidden=element;visibility=element.style.visibility;element.style.visibility='hidden';}};
    const originFor=(attachment:BridgeRecord)=>{
        const image=source(attachment),rect=image?.getBoundingClientRect();restore();
        if(rect&&rect.width>0&&rect.height>0&&rect.bottom>0&&rect.top<innerHeight&&rect.right>0&&rect.left<innerWidth){hide(image);return rect;}
    };
    createEffect(()=>{if(props.open!==false)hide(source(props.attachment));else setSelected(props.attachment);onCleanup(restore);});
    const close=()=>{const attachment=props.attachment;restore();props.onClose();queueMicrotask(()=>{source(attachment)?.querySelector<HTMLElement>('.media-open')?.focus({preventScroll:true});});};
    const shareUrl=(attachment:BridgeRecord)=>new URL(`/messages/${encodeURIComponent(props.chatId)}/${encodeURIComponent(String(attachment.data.message_id??''))}`,location.origin).href;
    return <Show keyed when={auth.live()}>{live=>{
        const [presented,setPresented]=createSignal(false);
        createEffect(()=>{if(props.open===false)return;let frame=requestAnimationFrame(()=>{frame=requestAnimationFrame(()=>setPresented(true));});onCleanup(()=>{cancelAnimationFrame(frame);setPresented(false);});});
        const collection=createMemo(()=>presented()?createMediaCollection(live,props.chatId,Math.max(1,Math.min(200,props.pageSize??200))):undefined),handles=createHandlesCache(live),cache=new Map<string,BridgeRecord>();
        const [record,setRecord]=createSignal<ReturnType<typeof live.subscribe>>();
        const ownerId=createMemo(()=>text(selected().data.message_id));
        createEffect(()=>{const id=ownerId();setRecord(undefined);if(!presented()||!id)return;let subscription:ReturnType<typeof live.subscribe>|undefined;const timer=setTimeout(()=>{subscription=live.subscribe('record',{collection:'messages.message',id});setRecord(subscription);},80);onCleanup(()=>{clearTimeout(timer);subscription?.dispose();});});
        const owner=createMemo(()=>{if(!presented())return;const id=ownerId(),message=record()?.data().records?.find(value=>value.id===id);if(message){cache.delete(id);cache.set(id,message);if(cache.size>64)cache.delete(cache.keys().next().value!);}return message??cache.get(id);});
        createEffect(()=>{const sender=text(owner()?.data.sender_handle_id);if(sender)handles.ensure([sender]);});
        const timestamp=()=>text(owner()?.data.sent_at)||text(selected().data.sent_at);
        const subtitle=()=>Number.isFinite(Date.parse(timestamp()))?header(timestamp(),props.now,props.locale):'';
        const senderName=()=>owner()?.data.is_from_me?wording.you:name(text(owner()?.data.sender_handle_id),handles.read());
        return <MediaViewer {...props} onClose={close} onSelection={setSelected} senderName={senderName()} timestamp={timestamp()} subtitle={subtitle()} originFor={originFor} shareUrl={shareUrl} attachments={collection()?.attachments()??[]} loading={collection()?.loading()??true} error={collection()?.error()} onRetry={()=>collection()?.retry()}/>;
    }}</Show>;
}

export function CardVideo(props:{attachment:BridgeRecord}) {
    const [ready,setReady]=createSignal(false),[playing,setPlaying]=createSignal(false),[position,setPosition]=createSignal(0),[length,setLength]=createSignal(Number(fields(props.attachment.data.media).duration_ms??0)/1000),[status,setStatus]=createSignal('');
    let video!:HTMLVideoElement,frame:number|undefined,disposed=false;
    const decoded=()=>{if('requestVideoFrameCallback' in video){if(frame===undefined)frame=video.requestVideoFrameCallback(()=>{frame=undefined;if(!disposed)setReady(true);});}else setReady(true);};
    const toggle=()=>{if(video.paused){setStatus('');void video.play().catch(()=>setStatus('Video unavailable'));}else video.pause();};
    onCleanup(()=>{disposed=true;if(frame!==undefined)video.cancelVideoFrameCallback(frame);video.pause();video.removeAttribute('src');video.load();});
    return <div class="message-card-player" data-ready={ready()}><video ref={video} src={mediaSource(props.attachment,'video')} playsinline autoplay muted preload="metadata" aria-label="Video preview" onLoadedData={decoded} onPlaying={()=>{setPlaying(true);decoded();}} onPause={()=>setPlaying(false)} onEnded={()=>setPlaying(false)} onTimeUpdate={()=>setPosition(video.currentTime)} onLoadedMetadata={()=>{if(Number.isFinite(video.duration))setLength(video.duration);}} onError={()=>setStatus('Video unavailable')}/>
        <div class="message-card-controls"><button type="button" aria-label={playing()?'Pause video':'Play video'} onClick={toggle}><svg viewBox="0 0 24 24" aria-hidden="true"><Show when={playing()} fallback={<use width="24" height="24" href="/icons.svg#message-card-play"/>}><use width="24" height="24" href="/icons.svg#message-card-pause"/></Show></svg></button><input type="range" aria-label="Video position" min="0" max={length()} step="0.1" value={position()} onInput={event=>{if(Number.isFinite(video.duration))video.currentTime=Number(event.currentTarget.value);}}/><span>{duration(position())}</span></div><Show when={status()}><span class="message-card-missing" role="status">{status()}</span></Show>
    </div>;
}
