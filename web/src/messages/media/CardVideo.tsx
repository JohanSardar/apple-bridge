import {createSignal,onCleanup,Show} from 'solid-js';
import type {BridgeRecord} from '../../data/types';
import {fields} from '../store/types';
import {duration} from '../format';
import {mediaSource} from './source';
export function CardVideo(props:{attachment:BridgeRecord}) {
    const [ready,setReady]=createSignal(false),[playing,setPlaying]=createSignal(false),[position,setPosition]=createSignal(0),[length,setLength]=createSignal(Number(fields(props.attachment.data.media).duration_ms??0)/1000),[status,setStatus]=createSignal('');
    let video!:HTMLVideoElement,frame:number|undefined,disposed=false;
    const decoded=()=>{if('requestVideoFrameCallback' in video){if(frame===undefined)frame=video.requestVideoFrameCallback(()=>{frame=undefined;if(!disposed)setReady(true);});}else setReady(true);};
    const toggle=()=>{if(video.paused){setStatus('');void video.play().catch(()=>setStatus('Video unavailable'));}else video.pause();};
    onCleanup(()=>{disposed=true;if(frame!==undefined)video.cancelVideoFrameCallback(frame);video.pause();video.removeAttribute('src');video.load();});
    return <div class="message-card-player" data-ready={ready()}><video ref={video} src={mediaSource(props.attachment,'video')} playsinline autoplay muted preload="metadata" aria-label="Video preview" onLoadedData={decoded} onPlaying={()=>{setPlaying(true);decoded();}} onPause={()=>setPlaying(false)} onEnded={()=>setPlaying(false)} onTimeUpdate={()=>setPosition(video.currentTime)} onLoadedMetadata={()=>{if(Number.isFinite(video.duration))setLength(video.duration);}} onError={()=>setStatus('Video unavailable')}/>
        <div class="message-card-controls"><button type="button" aria-label={playing()?'Pause video':'Play video'} onClick={toggle}><svg viewBox="0 0 24 24" aria-hidden="true"><Show when={playing()} fallback={<path d="M9 5.5v13l10-6.5Z" fill="currentColor"/>}><path d="M8 6v12m8-12v12" fill="none" stroke="currentColor" stroke-width="3"/></Show></svg></button><input type="range" aria-label="Video position" min="0" max={length()} step="0.1" value={position()} onInput={event=>{if(Number.isFinite(video.duration))video.currentTime=Number(event.currentTarget.value);}}/><span>{duration(position())}</span></div><Show when={status()}><span class="message-card-missing" role="status">{status()}</span></Show>
    </div>;
}
