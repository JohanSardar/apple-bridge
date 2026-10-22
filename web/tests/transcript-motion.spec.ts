import {test,expect,type BrowserContext,type Page,type APIRequestContext} from '@playwright/test';
import {mkdir} from 'node:fs/promises';
import {control} from './session';
let cookies:Parameters<BrowserContext['addCookies']>[0]=[],ids:Record<string,string>;
test.beforeAll(async({request})=>{cookies=[(await control(request,'/session')).cookie];ids=await control(request,'/demo-fixtures');});
async function instrument(page:Page){
    await page.addInitScript(()=>{
        const state=window as any;state.__transcriptMotion=[];state.__transcriptAnimations=[];const previous=new WeakMap<Element,string>();
        const record=(element:Element)=>{if(!(element instanceof HTMLElement)||!element.dataset.motion)return;const signature=element.dataset.motion+'|'+element.style.transform+'|'+element.style.height;if(previous.get(element)===signature)return;previous.set(element,signature);state.__transcriptMotion.push({name:element.dataset.motion,id:element.closest<HTMLElement>('[data-message]')?.dataset.message,transform:element.style.transform,origin:element.style.transformOrigin,height:element.style.height});};
        new MutationObserver(changes=>{for(const change of changes){if(change.type==='attributes')record(change.target as Element);else for(const node of change.addedNodes)if(node instanceof Element){record(node);for(const element of node.querySelectorAll('[data-motion]'))record(element);}}}).observe(document,{subtree:true,childList:true,attributes:true});
        const original=Element.prototype.animate;Element.prototype.animate=function(...args:Parameters<Element['animate']>){if(this instanceof HTMLElement&&this.dataset.motion){const options=args[1];state.__transcriptAnimations.push({name:this.dataset.motion,duration:typeof options==='number'?options:options?.duration,delay:typeof options==='object'?options?.delay:0});}return original.apply(this,args);};
    });
}
async function arrive(page:Page,request:APIRequestContext){
    const known=new Set<string>(),incoming:string[]=[];let armed=false;
    page.on('websocket',socket=>socket.on('framereceived',event=>{let frame:any;try{frame=JSON.parse(String(event.payload));}catch{return;}for(const item of [...frame.data?.items??[],...(frame.changes??[]).map((change:any)=>change.row)]){if(!item||item.collection!=='messages.message'||item.data?.chat_id!==ids['message-chat'])continue;if(armed&&!known.has(item.id)&&item.data.is_from_me===false)incoming.push(item.id);known.add(item.id);}}));
    await page.goto('/messages/'+encodeURIComponent(ids['message-chat']));await expect(page.locator('.messages-transcript')).toHaveAttribute('aria-busy','false');await expect(page.locator('.transcript')).toHaveCSS('transform','none');const viewport=page.getByRole('listbox',{name:'Messages transcript'});await viewport.evaluate(element=>element.scrollTop=element.scrollHeight);await expect.poll(()=>viewport.evaluate(element=>element.scrollHeight-element.scrollTop-element.clientHeight)).toBeLessThanOrEqual(1);await page.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));
    expect(await page.evaluate(()=>(window as any).__transcriptMotion.filter((entry:any)=>entry.name==='receive'))).toEqual([]);armed=true;await control(request,'/incoming');await expect.poll(()=>incoming.length).toBeGreaterThan(0);const id=incoming[0];await expect(page.locator('[data-message="'+id+'"]')).toBeVisible();return id;
}
test('WEB-MSG-MOTION WEB-MSG-LIVE RECEIVE a live incoming bubble scales from its tail with an overshoot while history stays still',async({page,context,request},info)=>{
    await context.addCookies(cookies);await instrument(page);const id=await arrive(page,request);
    await expect.poll(()=>page.evaluate(id=>(window as any).__transcriptMotion.filter((entry:any)=>entry.name==='receive'&&entry.id===id),id)).not.toEqual([]);
    const first=await page.evaluate(id=>(window as any).__transcriptMotion.find((entry:any)=>entry.name==='receive'&&entry.id===id),id);expect(first.transform).toBe('scale(0.92)');expect(first.origin).toBe('left bottom');
    await expect.poll(()=>page.evaluate(id=>(window as any).__transcriptMotion.filter((entry:any)=>entry.name==='receive'&&entry.id===id).some((entry:any)=>Number(/^scale\(([^)]+)\)$/u.exec(entry.transform)?.[1])>1),id)).toBe(true);await expect(page.locator('[data-message="'+id+'"] .bubble-slot').first()).toHaveCSS('transform','none');
    expect(await page.evaluate(()=>([...new Set((window as any).__transcriptMotion.filter((entry:any)=>entry.name==='receive').map((entry:any)=>entry.id))]))).toEqual([id]);
    await mkdir('../lab/evidence/private/web',{recursive:true});await page.screenshot({path:`../lab/evidence/private/web/transcript-motion-receive-${info.project.name}.png`});
});
test('WEB-MSG-MOTION WEB-MSG-LIVE REDUCE-MOTION Reduce Motion requests a150ms receive fade without scaling',async({page,context,request})=>{
    await context.addCookies(cookies);await page.emulateMedia({reducedMotion:'reduce'});await instrument(page);const id=await arrive(page,request);
    await expect.poll(()=>page.evaluate(()=>(window as any).__transcriptAnimations.some((entry:any)=>entry.name==='receive'&&entry.duration===150))).toBe(true);expect(await page.evaluate(id=>(window as any).__transcriptMotion.filter((entry:any)=>entry.name==='receive'&&entry.id===id).some((entry:any)=>entry.transform.includes('scale')),id)).toBe(false);
});
