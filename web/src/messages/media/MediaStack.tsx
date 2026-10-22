import {For,Show,createEffect,createMemo,createSignal,onCleanup} from 'solid-js';
import type {BridgeRecord} from '../../data/types';
import {reducedMotion,springAnimation} from '../../motion/animate';
import {fields,text} from '../store/types';
import {duration} from '../format';
import {MediaImage} from './MediaImage';
import {attachmentHasAlpha} from './source';
import {preloadMediaViewer} from './ChatMediaViewer';
import './stack.css';
export type MediaStackProps={attachments:readonly BridgeRecord[];width:number;height:number;onOpen?:(attachment:BridgeRecord,rect:DOMRectReadOnly)=>void};
export function MediaStack(props:MediaStackProps){
 const breakpoint=matchMedia('(max-width:699px)'),motion=matchMedia('(prefers-reduced-motion:reduce)');
 const [phone,setPhone]=createSignal(breakpoint.matches),[index,setIndex]=createSignal(0),[progress,setProgress]=createSignal(0),[state,setState]=createSignal('settled');
 const ids=createMemo(()=>props.attachments.map(value=>value.id));
 let root!:HTMLDivElement,animation:ReturnType<typeof springAnimation>|undefined,suppress=false,disposed=false;
 let pointer:{id:number;x:number;y:number;progress:number;index:number;horizontal:boolean;lastX:number;at:number;velocity:number}|undefined;
 const bound=(value:number)=>Math.max(0,Math.min(ids().length-1,value));
 const preload=()=>{if(props.onOpen)void preloadMediaViewer().catch(()=>{});};
 const focus=(selected:number)=>queueMicrotask(()=>{if(!disposed)root.querySelector<HTMLElement>(`[data-stack-position="${selected}"]>.media-open`)?.focus({preventScroll:true});});
 const settle=(selected:number,velocity=0,restoreFocus=false)=>{
  animation?.cancel();selected=bound(selected);setIndex(selected);
  const finish=()=>{setState('settled');if(restoreFocus)focus(selected);};
  if(!phone()||reducedMotion()||Math.abs(progress()-selected)<.0001){setProgress(selected);finish();return;}
  setState('moving');animation=springAnimation(setProgress,progress(),selected,{response:.32,damping:.9},velocity,finish);
 };
 const down=(event:PointerEvent)=>{
  preload();if(!phone()||!event.isPrimary||event.button>0||pointer)return;
  animation?.cancel();suppress=false;pointer={id:event.pointerId,x:event.clientX,y:event.clientY,progress:progress(),index:index(),horizontal:false,lastX:event.clientX,at:event.timeStamp,velocity:0};
 };
 const move=(event:PointerEvent)=>{
  if(pointer?.id!==event.pointerId)return;const active=pointer,dx=event.clientX-active.x,dy=event.clientY-active.y;
  if(!active.horizontal){if(Math.hypot(dx,dy)<8)return;if(Math.abs(dy)>Math.abs(dx)){pointer=undefined;settle(index());return;}active.horizontal=true;suppress=true;root.setPointerCapture(active.id);setState('dragging');}
  event.preventDefault();active.velocity=Math.max(-2,Math.min(2,(event.clientX-active.lastX)/Math.max(1,event.timeStamp-active.at)));active.lastX=event.clientX;active.at=event.timeStamp;
  const value=active.progress-dx/Math.max(1,props.width),last=Math.max(0,ids().length-1);
  setProgress(value<0?-.16*(1-Math.exp(value)):value>last?last+.16*(1-Math.exp(last-value)):value);
 };
 const up=(event:PointerEvent)=>{
  if(pointer?.id!==event.pointerId)return;const active=pointer;pointer=undefined;if(root.hasPointerCapture(event.pointerId))root.releasePointerCapture(event.pointerId);
  const dx=event.clientX-active.x,velocity=event.timeStamp-active.at<80?active.velocity:0;
  const advance=event.type!=='pointercancel'&&active.horizontal&&(Math.abs(dx)>props.width*.2||Math.abs(velocity)>.5);
  const direction=(Math.abs(velocity)>.5?velocity:dx)<0?1:-1;
  settle(advance?active.index+direction:active.index,advance?-velocity/Math.max(1,props.width)*1000:0,active.horizontal&&root.contains(document.activeElement));
 };
 const key=(event:KeyboardEvent)=>{
  if(event.altKey||event.ctrlKey||event.metaKey)return;
  const current=Number((event.target as HTMLElement).closest('[data-stack-position]')?.getAttribute('data-stack-position')??index());
  const next=({ArrowLeft:current-1,ArrowRight:current+1,Home:0,End:ids().length-1} as Record<string,number>)[event.key];
  if(next===undefined)return;event.preventDefault();event.stopPropagation();suppress=false;settle(next,0,true);
 };
 const changed=()=>{animation?.cancel();pointer=undefined;setPhone(breakpoint.matches);setProgress(index());setState('settled');};
 breakpoint.addEventListener('change',changed);motion.addEventListener('change',changed);
 onCleanup(()=>{disposed=true;animation?.cancel();breakpoint.removeEventListener('change',changed);motion.removeEventListener('change',changed);});
 createEffect(()=>{if(index()!==bound(index()))settle(bound(index()));});
 const transform=(position:number)=>{const distance=position-progress(),back=Math.min(2,Math.max(0,distance));return `translate3d(${distance<0?distance*(props.width+12):back*5}px,${-back*4}px,0) scale(${1-back*.035}) rotate(${distance<0?distance*3:back*2}deg)`;};
 return <div ref={root} class="media-stack" role="group" aria-label="Photos" aria-roledescription={phone()?'carousel':undefined} data-stack-index={index()} data-stack-motion={state()} data-media-alpha={props.attachments.length>0&&props.attachments.every(attachmentHasAlpha)} style={{width:`${props.width}px`,height:`${props.height}px`,'--media-stack-rows':Math.max(1,Math.ceil(ids().length/2)),'--media-stack-columns':Math.max(1,Math.min(2,ids().length))}} onPointerDown={down} onPointerMove={move} onPointerUp={up} onPointerCancel={up} onKeyDown={key}>
  <For each={ids()}>{(id,position)=>{const record=createMemo(()=>props.attachments.find(value=>value.id===id));return <Show when={record()}>{attachment=><div class="media-stack-cell" data-message={attachment().data.message_id} data-stack-position={position()} data-media-origin={id} data-stack-active={position()===index()} data-media-alpha={attachmentHasAlpha(attachment())} aria-hidden={phone()&&position()!==index()?true:undefined} inert={phone()&&position()!==index()} style={{display:phone()&&(position()-progress()>=3||position()-progress()<=-1)?'none':undefined,transform:phone()?transform(position()):undefined,opacity:phone()?Math.min(1,Math.max(0,3-position()+progress())):undefined,'z-index':phone()?ids().length-position():undefined}}>
   <MediaImage attachment={attachment()} variant={text(attachment().data.mime_type).startsWith('video/')?'poster':'thumb'} alt=""/>
   <button type="button" class="media-open" tabindex={phone()&&position()!==index()?-1:0} aria-label={`Open ${text(attachment().data.mime_type).startsWith('video/')?'video':'photo'} ${position()+1} of ${ids().length}`} onPointerEnter={preload} onFocus={()=>{preload();if(!phone())setIndex(position());}} onClick={event=>{event.stopPropagation();if(suppress||state()!=='settled'){event.preventDefault();return;}props.onOpen?.(attachment(),event.currentTarget.getBoundingClientRect());}}/>
   <Show when={text(attachment().data.mime_type).startsWith('video/')}><span class="media-stack-video" aria-hidden="true"><svg viewBox="0 0 24 24"><use width="24" height="24" href="/icons.svg#media-motion-play"/></svg>{duration(Number(fields(attachment().data.media).duration_ms??0)/1000)}</span></Show>
   <Show when={phone()&&position()===index()&&ids().length>1}><span class="media-stack-page" aria-hidden="true">{index()+1}/{ids().length}</span></Show>
  </div>}</Show>;}}</For>
  <span class="media-stack-status" role="status">{phone()&&ids().length?`Photo ${index()+1} of ${ids().length}`:''}</span>
 </div>;
}
