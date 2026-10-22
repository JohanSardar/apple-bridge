import { batch, createEffect, createMemo, createSignal, onCleanup, untrack, type Accessor } from 'solid-js';
import type { LiveClient } from '../../data/live';
import type { Cursor, ThreadItem, ViewData } from '../../data/types';
export const threadPageSize=100;
export const threadWindowLimit=500;
type WindowState={cursor?:Cursor;anchor:string;items:ThreadItem[];before?:string;after?:string;older:number;newer:number;unread?:string};
type Page={data:ViewData;cursor?:Cursor};
type Pages={ready:boolean;pages:Page[];token?:string};
type Subscription={data:Accessor<ViewData>;cursor:Accessor<Cursor|undefined>;dispose:()=>void};
export function createThreadWindow(live:LiveClient,chat:string,initialAnchor='unread'){
    const [anchor,setAnchor]=createSignal(initialAnchor),[loading,setLoading]=createSignal(true);
    const [window,setWindow]=createSignal<WindowState>({anchor:initialAnchor,items:[],older:0,newer:0});
    let publishedAnchor='',frame=0;
    let retained=new Map<string,{item:ThreadItem;epoch?:string;sequence:number}>();
    onCleanup(()=>cancelAnimationFrame(frame));
    createEffect(()=>{
        const value=anchor();setLoading(true);
        const main:Subscription=live.subscribe('thread',{chat_id:chat,anchor:value,limit:threadPageSize});
        onCleanup(()=>{main.dispose();cancelAnimationFrame(frame);});
        function prefetch(source:Subscription,direction:'before'|'after',depth:number):Accessor<Pages>{
            const [result,setResult]=createSignal<Pages>({ready:false,pages:[]});
            const token=createMemo(()=>source.data()[direction]);
            createEffect(()=>{
                const requested=token();
                if(!requested){setResult({ready:true,pages:[],token:requested});return;}
                setResult({ready:false,pages:[],token:requested});
                const subscription:Subscription=live.subscribe('thread',{chat_id:chat,[direction]:requested,limit:threadPageSize});
                onCleanup(subscription.dispose);
                const next:Accessor<Pages>=depth>1?prefetch(subscription,direction,depth-1):()=>({ready:true,pages:[]});
                createEffect(()=>{const data=subscription.data(),cursor=subscription.cursor(),following=next();setResult({ready:Boolean(data.items)&&following.ready,pages:[{data,cursor},...following.pages],token:requested});});
            });
            return ()=>{const state=result();return state.token===token()?state:{ready:false,pages:[]};};
        }
        const before=prefetch(main,'before',2),after=prefetch(main,'after',2);
        createEffect(()=>{
            const center={data:main.data(),cursor:main.cursor()},older=before(),newer=after();
            if(!center.data.items||!older.ready||!newer.ready)return;
            cancelAnimationFrame(frame);
            frame=requestAnimationFrame(()=>{
            const pages=[...older.pages,...newer.pages,center].sort((a,b)=>(a.cursor?.sequence??0)-(b.cursor?.sequence??0)),head=center.cursor;
            const coherent=Boolean(head)&&pages.every(page=>page.cursor?.epoch===head?.epoch&&page.cursor?.sequence===head?.sequence);
            const previous=untrack(window),map=new Map<string,ThreadItem>();
            const versions=new Map<string,{item:ThreadItem;epoch?:string;sequence:number}>();
            if(!coherent&&publishedAnchor===value)for(const item of previous.items){map.set(item.id,item);const known=retained.get(item.id);if(known)versions.set(item.id,known);}
            for(const page of pages)for(const item of page.data.items??[]){
                const known=versions.get(item.id),sequence=page.cursor?.sequence??0,epoch=page.cursor?.epoch;
                if(!known||known.epoch!==epoch||sequence>known.sequence||sequence===known.sequence&&item.version>=known.item.version){map.set(item.id,item);versions.set(item.id,{item,epoch,sequence});}
            }
            const ordered=[...map.values()].sort((a,b)=>Date.parse(a.data.sent_at)-Date.parse(b.data.sent_at)||a.id.localeCompare(b.id));
            const middle=ordered.findIndex(item=>item.id===(center.data.items?.[Math.floor((center.data.items?.length??0)/2)]?.id));
            const start=Math.max(0,Math.min(ordered.length-threadWindowLimit,middle-Math.floor(threadWindowLimit/2)));
            const next=ordered.slice(start,start+threadWindowLimit);
            const items=next.length===previous.items.length&&next.every((item,index)=>item===previous.items[index])?previous.items:next;
            const first=older.pages.at(-1)?.data??center.data,last=newer.pages.at(-1)?.data??center.data;
            publishedAnchor=value;retained=new Map(next.map(item=>[item.id,versions.get(item.id)!]));
            batch(()=>{setWindow({cursor:coherent?head:undefined,anchor:value,items,before:first.before,after:last.after,older:Number(first.older??0),newer:Number(last.newer??0),unread:center.data.unread_anchor});setLoading(false);});
            });
        });
    });
    function visible(start:number,end:number){
        const state=untrack(window);if(untrack(loading)||state.items.length<threadPageSize)return;
        const id=start<25&&state.before?state.items[Math.min(start+50,state.items.length-1)]?.id:end>state.items.length-25&&state.after?state.items[Math.max(0,end-50)]?.id:undefined;
        if(id&&id!==untrack(anchor))setAnchor(id);
    }
    return {view:window,items:()=>window().items,anchor,newer:()=>window().newer,older:()=>window().older,loading,unread:()=>window().unread,visible,jump:(id:string)=>setAnchor(id),latest:()=>setAnchor('latest'),oldest:()=>setAnchor('oldest')};
}
