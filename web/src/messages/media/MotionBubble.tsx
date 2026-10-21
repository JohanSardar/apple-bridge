import {createEffect,createSignal,onCleanup,Show} from 'solid-js';
import type {BridgeRecord} from '../../data/types';
import {springAnimation} from '../../motion/animate';
import {duration} from '../format';
import {fields,text} from '../store/types';
import {MediaImage} from './MediaImage';
import {mediaSource} from './source';
import {preloadMediaViewer} from './ChatMediaViewer';
import './motion.css';
export function MotionBubble(props:{attachment:BridgeRecord;onOpen?:(attachment:BridgeRecord,rect:DOMRectReadOnly)=>void}){
 const live=()=>fields(props.attachment.data.media).live===true,[mounted,setMounted]=createSignal(false),[playing,setPlaying]=createSignal(false),[decoded,setDecoded]=createSignal(false),[frameSource,setFrameSource]=createSignal(''),[status,setStatus]=createSignal('');
 const reduced=matchMedia('(prefers-reduced-motion:reduce)');let video:HTMLVideoElement|undefined,control:HTMLSpanElement|undefined,timer:ReturnType<typeof setTimeout>|undefined,held=false,active=false,disposed=false,suppressedUntil=0,generation=0,scale=1,down:{id:number;x:number;y:number}|undefined,pressAnimation:ReturnType<typeof springAnimation>|undefined,cancelFrame=()=>{};
 const clear=()=>{clearTimeout(timer);timer=undefined;};
 const press=(target:number)=>{if(!control)return;if(reduced.matches){pressAnimation?.cancel();pressAnimation=undefined;scale=1;control.style.transform='';return;}if(pressAnimation)pressAnimation.retarget(target);else pressAnimation=springAnimation(value=>{scale=value;control!.style.transform=`scale(${value})`;},scale,target,{response:.26,damping:.8});};
 const stop=()=>{clear();active=false;generation++;cancelFrame();cancelFrame=()=>{};if(video){video.pause();try{video.currentTime=0;}catch{}}setPlaying(false);setDecoded(false);setFrameSource('');};
 const firstFrame=(element:HTMLVideoElement)=>{
  cancelFrame();const token=generation;
  const reveal=(source:string)=>{if(active&&!disposed&&token===generation){setFrameSource(source);setDecoded(true);}};
  if(typeof element.requestVideoFrameCallback==='function'){const id=element.requestVideoFrameCallback(()=>reveal('callback'));cancelFrame=()=>element.cancelVideoFrameCallback?.(id);}else if(element.readyState>=2)reveal('loadeddata');
 };
 const play=()=>{if(disposed||document.hidden||active&&video&&!video.paused)return;active=true;setStatus('');setMounted(true);queueMicrotask(()=>{if(!active||!video||disposed)return;if(video.error)video.load();if(video.readyState>=2)firstFrame(video);void video.play().catch(()=>{if(active&&!disposed){setPlaying(false);setStatus('Media unavailable');}});});};
 const hold=()=>{held=true;suppressedUntil=performance.now()+350;press(.92);play();};
 const finish=()=>{if(held)suppressedUntil=performance.now()+350;held=false;down=undefined;press(1);stop();};
 const preload=()=>{if(props.onOpen)void preloadMediaViewer().catch(()=>{});};
 const visibility=()=>{if(document.hidden)finish();};
 createEffect(()=>{props.attachment.id;stop();setMounted(false);setStatus('');});
 const preference=()=>finish();reduced.addEventListener('change',preference);document.addEventListener('visibilitychange',visibility);window.addEventListener('blur',finish);
 onCleanup(()=>{disposed=true;pressAnimation?.cancel();stop();reduced.removeEventListener('change',preference);document.removeEventListener('visibilitychange',visibility);window.removeEventListener('blur',finish);if(video){video.removeAttribute('src');video.load();}});
 return <div class="media-motion" data-media-playing={playing()} data-media-live={live()} data-media-frame={decoded()?'decoded':'pending'} data-media-frame-source={frameSource()}><MediaImage attachment={props.attachment} variant={live()?'thumb':'poster'}/><Show when={mounted()}><video ref={video} src={mediaSource(props.attachment,live()?'live':'video')} muted playsinline preload="none" loop={live()} aria-label={live()?'Live Photo preview':'Video preview'} onLoadedData={event=>firstFrame(event.currentTarget)} onPlaying={()=>{if(active)setPlaying(true);else video?.pause();}} onEnded={()=>{setPlaying(false);active=false;}} onError={()=>{if(active)setStatus('Media unavailable');}}/></Show>
  <Show when={!live()}><span ref={control} class="media-motion-play" aria-hidden="true"><svg viewBox="0 0 24 24" fill="currentColor"><path d="m8 5 11 7-11 7Z"/></svg></span></Show><span class="media-motion-badge" aria-hidden="true"><Show when={live()}><svg viewBox="0 0 24 24" fill="none" stroke="currentColor"><circle cx="12" cy="12" r="2" fill="currentColor" stroke="none"/><circle cx="12" cy="12" r="6"/><circle cx="12" cy="12" r="10" stroke-dasharray="1 2.5" stroke-linecap="round"/></svg></Show>{live()?'LIVE':duration(Math.max(0,Number(fields(props.attachment.data.media).duration_ms)||0)/1000)}</span>
  <button type="button" class="media-open" aria-label={`Open ${text(props.attachment.data.description)||text(props.attachment.data.filename)||(live()?'Live Photo':'video')}`} onFocus={preload} onPointerEnter={event=>{preload();if(live()&&event.pointerType==='mouse'&&!reduced.matches&&matchMedia('(hover:hover) and (pointer:fine)').matches)play();}} onPointerLeave={()=>{if(!down)stop();}} onPointerDown={event=>{if(event.button!==0)return;preload();clear();press(.92);held=false;down={id:event.pointerId,x:event.clientX,y:event.clientY};event.currentTarget.setPointerCapture(event.pointerId);timer=setTimeout(hold,250);}} onPointerMove={event=>{if(down&&Math.hypot(event.clientX-down.x,event.clientY-down.y)>8){suppressedUntil=performance.now()+350;finish();}}} onPointerUp={finish} onPointerCancel={()=>{suppressedUntil=performance.now()+350;finish();}} onLostPointerCapture={()=>{if(down)finish();}} onKeyDown={event=>{if(event.key===' '||event.key==='Spacebar'){event.preventDefault();if(!event.repeat)hold();}}} onKeyUp={event=>{if(event.key===' '||event.key==='Spacebar'){event.preventDefault();finish();}}} onBlur={finish} onClick={event=>{if(performance.now()<suppressedUntil){event.preventDefault();return;}finish();props.onOpen?.(props.attachment,event.currentTarget.getBoundingClientRect());}}/>
  <Show when={status()}><span class="media-motion-status" role="status">{status()}</span></Show>
 </div>;
}
