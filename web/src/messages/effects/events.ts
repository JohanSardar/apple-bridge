import {normalizeEffect,type ScreenEffect} from './data';
export type EffectOptions={id:string;arrival:boolean;replay?:boolean;text?:string;origin?:{left:number;top:number;width:number;height:number}};
export type ScreenRequest={sequence:number;effect:ScreenEffect;label:string;text?:string;origin?:EffectOptions['origin']};
const seen=new Set<string>(),listeners=new Set<(event:ScreenRequest)=>void>(),watchers=new Set<(event:ScreenRequest)=>void>();let sequence=0,pending:ScreenRequest|undefined;
export function playScreenEffect(raw:unknown,options:EffectOptions):boolean {
 const effect=normalizeEffect(raw);if(effect?.scope!=='screen'||(!options.arrival&&!options.replay))return false;
 const key=options.id+':'+effect.name;if(options.arrival&&!options.replay&&seen.has(key))return false;
 if(options.arrival){seen.add(key);if(seen.size>2048)seen.delete(seen.values().next().value!);}
 const source=options.origin,origin=source&&[source.left,source.top,source.width,source.height].every(Number.isFinite)&&source.width>0&&source.height>0?{left:source.left,top:source.top,width:source.width,height:source.height}:undefined;const event={sequence:++sequence,effect:effect.name,label:effect.label,text:options.text?.slice(0,160),origin};if(!listeners.size)pending=event;for(const watcher of watchers)watcher(event);for(const listener of listeners)listener(event);return true;
}
export function subscribeEffects(listener:(event:ScreenRequest)=>void):()=>void {listeners.add(listener);if(pending){const event=pending;pending=undefined;listener(event);}return ()=>{listeners.delete(listener);};}

export function watchEffects(listener:(event:ScreenRequest)=>void):()=>void {watchers.add(listener);if(pending)listener(pending);return ()=>{watchers.delete(listener);};}

export function discardPending(sequence:number):void {if(pending?.sequence===sequence)pending=undefined;}
