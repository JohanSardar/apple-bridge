import {createComponent,createSignal,onCleanup,onMount,Show} from 'solid-js';
import {Portal} from 'solid-js/web';
import {watchEffects,discardPending} from './events';
import type {CanvasEffectsLayer} from './CanvasEffectsLayer';
export {playScreenEffect} from './events';
export type EffectsLayerProps={onLabel?:(label:string)=>void;onComplete?:(metrics:{frames:number;p95:number;max:number})=>void};
let loading:Promise<typeof CanvasEffectsLayer>|undefined;
export function preloadEffects(){return loading??=import('./CanvasEffectsLayer').then(module=>module.CanvasEffectsLayer).catch(error=>{loading=undefined;throw error;});}
export function EffectsLayer(props:EffectsLayerProps){
 const [layer,setLayer]=createSignal<typeof CanvasEffectsLayer>();let disposed=false;
 onMount(()=>{const stop=watchEffects(event=>{props.onLabel?.(event.label);if(matchMedia('(prefers-reduced-motion:reduce)').matches||document.hidden){discardPending(event.sequence);return;}void preloadEffects().then(component=>{if(!disposed)setLayer(()=>component);}).catch(()=>{});});onCleanup(()=>{disposed=true;stop();});});
 return <Show keyed when={layer()}>{component=><Portal>{createComponent(component,{get onComplete(){return props.onComplete;}})}</Portal>}</Show>;
}
