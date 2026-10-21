import { createSignal, onCleanup, Show } from 'solid-js';
import type { BridgeRecord } from '../../data/types';
import { duration } from '../format';
import { fields } from '../store/types';
import { mediaSource } from './source';
import {MediaImage} from './MediaImage';
type PiPVideo=HTMLVideoElement&{webkitSetPresentationMode?:(mode:string)=>void;webkitSupportsPresentationMode?:(mode:string)=>boolean};
function PlayerIcon(props:{name:'play'|'pause'|'volume'|'muted'|'pip'}){
    const paths={play:'M8 5v14l11-7Z',pause:'M8 5v14M16 5v14',volume:'M11 4 6 9H3v6h3l5 5ZM15 8a6 6 0 0 1 0 8m3-11a10 10 0 0 1 0 14',muted:'M11 4 6 9H3v6h3l5 5ZM16 8l6 8m0-8-6 8',pip:'M3 5h18v14H3ZM12 12h8v6h-8Z'};
    return <svg class="icon" viewBox="0 0 24 24" aria-hidden="true" fill={props.name==='play'?'currentColor':'none'}><path d={paths[props.name]}/></svg>;
}
export function VideoPlayer(props:{attachment:BridgeRecord;live?:boolean;ref?:(video:HTMLVideoElement)=>void}) {
    const [ready,setReady]=createSignal(false);
    const [playing,setPlaying]=createSignal(false),[position,setPosition]=createSignal(0),[length,setLength]=createSignal(Number(fields(props.attachment.data.media).duration_ms??0)/1000),[muted,setMuted]=createSignal(Boolean(props.live)),[status,setStatus]=createSignal('');
    let video!:PiPVideo;
    const connect=(element:HTMLVideoElement)=>{video=element;props.ref?.(element);};
    const play=()=>{setStatus('');if(video.error)video.load();if(video.paused)void video.play().catch(()=>setStatus('Video unavailable'));else video.pause();};
    const pictureInPicture=async()=>{try{if(document.pictureInPictureEnabled&&video.requestPictureInPicture)await video.requestPictureInPicture();else if(video.webkitSetPresentationMode)video.webkitSetPresentationMode('picture-in-picture');}catch{setStatus('Picture in Picture unavailable');}};
    onCleanup(()=>{video.pause();if(document.pictureInPictureElement===video)void document.exitPictureInPicture().catch(()=>{});video.removeAttribute('src');video.load();});
    return <div class="media-player" data-video-ready={ready()}><MediaImage attachment={props.attachment} variant={props.live?'display':'poster'} alt=""/><video ref={connect} src={mediaSource(props.attachment,props.live?'live':'video')} autoplay playsinline muted={props.live} preload="metadata" loop={props.live} aria-label={props.live?'Live Photo':'Video'} onLoadedData={()=>setReady(true)} onPlaying={()=>{setReady(true);setPlaying(true);}} onEnded={()=>setPlaying(false)} onPause={()=>setPlaying(false)} onVolumeChange={()=>setMuted(video.muted)} onTimeUpdate={()=>setPosition(video.currentTime)} onLoadedMetadata={()=>{if(Number.isFinite(video.duration))setLength(video.duration);}} onError={()=>setStatus('Video unavailable')}/><Show when={!props.live}><div class="media-player-controls glass" onPointerDown={event=>event.stopPropagation()}>
        <button type="button" aria-label={playing()?'Pause video':'Play video'} onClick={play}><PlayerIcon name={playing()?'pause':'play'}/></button><span>{duration(position())}</span><input type="range" aria-label="Video position" min="0" max={length()} step="0.1" value={position()} onInput={event=>{if(Number.isFinite(video.duration))video.currentTime=Number(event.currentTarget.value);}}/><span>{duration(length())}</span><button type="button" aria-label={muted()?'Unmute video':'Mute video'} onClick={()=>{video.muted=!video.muted;setMuted(video.muted);}}><PlayerIcon name={muted()?'muted':'volume'}/></button><Show when={document.pictureInPictureEnabled||typeof (HTMLVideoElement.prototype as PiPVideo).webkitSetPresentationMode==='function'}><button type="button" aria-label="Picture in Picture" onClick={()=>void pictureInPicture()}><PlayerIcon name="pip"/></button></Show>
    </div></Show><Show when={status()}><p role="status">{status()}</p></Show></div>;
}
