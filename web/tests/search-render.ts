import type { Page } from '@playwright/test';
export async function probeSearchRender(page:Page){await page.addInitScript(()=>{
    const Original=WebSocket,active=new Set<string>(),pending=new Map<string,{at:number;ids:string[]}>();
    (window as any).__searchHighlights=[];(window as any).__searchRender={dom:[] as number[],frame:[] as number[]};
    window.WebSocket=class extends Original{
        constructor(url:string|URL,protocols?:string|string[]){super(url,protocols);this.addEventListener('message',event=>{try{const frame=JSON.parse(String(event.data));if(frame.t==='snap'&&active.has(frame.sub)&&frame.data?.items?.length)pending.set(frame.sub,{at:performance.now(),ids:frame.data.items.map((item:any)=>item.id)});}catch{}});}
        send(data:any){if(typeof data==='string')try{const frame=JSON.parse(data);if(frame.t==='sub'&&frame.view==='search')active.add(frame.id);if(frame.t==='unsub'){active.delete(frame.id);pending.delete(frame.id);}}catch{}super.send(data);}
    };
    new MutationObserver(records=>{for(const record of records)if(record.type==='attributes'&&(record.target as Element).hasAttribute('data-search-hit'))(window as any).__searchHighlights.push((record.target as HTMLElement).dataset.message);const ids=[...document.querySelectorAll('.search-message')].map(element=>element.getAttribute('data-message-id'));for(const [key,sample]of pending)if(ids.length===sample.ids.length&&sample.ids.every(id=>ids.includes(id))){pending.delete(key);(window as any).__searchRender.dom.push(performance.now()-sample.at);requestAnimationFrame(()=>(window as any).__searchRender.frame.push(performance.now()-sample.at));}}).observe(document,{subtree:true,childList:true,attributes:true,attributeFilter:['data-search-hit']});
});}

export async function probeSearchHighlight(page:Page){await page.addInitScript(()=>{(window as any).__searchHighlights=[];new MutationObserver(records=>{for(const record of records)if((record.target as Element).hasAttribute('data-search-hit'))(window as any).__searchHighlights.push((record.target as HTMLElement).dataset.message);}).observe(document,{subtree:true,attributes:true,attributeFilter:['data-search-hit']});});}
