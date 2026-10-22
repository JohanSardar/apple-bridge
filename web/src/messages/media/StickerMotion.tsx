import {createEffect,createSignal,onCleanup,onMount} from 'solid-js';
import type {BridgeRecord} from '../../data/types';
import {mediaSource} from './source';
export function StickerMotion(props:{attachment:BridgeRecord}) {
 const [active,setActive]=createSignal(false),[decoded,setDecoded]=createSignal(false);
 let video!:HTMLVideoElement,observer:IntersectionObserver|undefined,intersecting=false,disposed=false,generation=0,cancelFrame=()=>{};
 const visibility=()=>setActive(intersecting&&!document.hidden);
 const frame=()=>{cancelFrame();const token=generation;const ready=()=>{if(!disposed&&active()&&token===generation)setDecoded(true);};if(typeof video.requestVideoFrameCallback==='function'){const id=video.requestVideoFrameCallback(ready);cancelFrame=()=>video.cancelVideoFrameCallback?.(id);}else if(video.readyState>=2)ready();};
 onMount(()=>{observer=new IntersectionObserver(entries=>{intersecting=entries.some(entry=>entry.isIntersecting);visibility();});observer.observe(video);document.addEventListener('visibilitychange',visibility);});
 createEffect(()=>{const source=mediaSource(props.attachment,'live'),play=active(),token=++generation;cancelFrame();cancelFrame=()=>{};setDecoded(false);if(play&&source)queueMicrotask(()=>{if(!disposed&&active()&&token===generation)void video.play().catch(()=>{});});else {video.pause();video.load();}});
 onCleanup(()=>{disposed=true;generation++;cancelFrame();observer?.disconnect();document.removeEventListener('visibilitychange',visibility);video.pause();video.removeAttribute('src');video.load();});
 return <video ref={video} class="sticker-motion" src={active()?mediaSource(props.attachment,'live'):undefined} data-frame={decoded()?'decoded':'pending'} muted playsinline loop preload="none" aria-hidden="true" onLoadedData={frame} onPlaying={frame} onError={()=>setDecoded(false)}/>;
}
