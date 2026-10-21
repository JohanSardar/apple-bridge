import { expect, type Page } from '@playwright/test';
import { test } from './list-session';
import { bootstrap, control } from './session';
import { mkdir, writeFile } from 'node:fs/promises';
async function snapshot(page:Page){return page.evaluate(async()=>{
    if(!(await indexedDB.databases()).some(db=>db.name==='ab.views'))return {pages:[],chain:[],missing:'',count:0,ids:[],materialized:null};
    const db=await new Promise<IDBDatabase>((resolve,reject)=>{const request=indexedDB.open('ab.views');request.onsuccess=()=>resolve(request.result);request.onerror=()=>reject(request.error);});
    const values=await new Promise<any[]>((resolve,reject)=>{const request=db.transaction('views').objectStore('views').getAll();request.onsuccess=()=>resolve(request.result);request.onerror=()=>reject(request.error);});db.close();
    const pages=values.flatMap(value=>{try{const [view,args]=JSON.parse(value.key);return view==='chats'&&args.filter==='all'?[{key:value.key,after:args.after??'',next:value.data.next??'',count:value.data.chats?.length??0,cursor:value.cursor,ids:(value.data.chats??[]).map((row:any)=>row.chat.id)}]:[];}catch{return [];}});
    const chain:any[]=[];let after='',missing='';const visited=new Set<string>();
    while(!visited.has(after)){visited.add(after);const page=pages.find(page=>page.after===after);if(!page){missing=after;break;}chain.push(page);if(!page.next)break;after=page.next;}
    const materialized=values.find(value=>{try{const [view,version,filter]=JSON.parse(value.key);return view==='chats-snapshot'&&version===1&&filter==='all';}catch{return false;}});
    const ids=materialized?.data.chats?.map((row:any)=>row.chat.id)??chain.flatMap(page=>page.ids);
    return {pages,chain,missing,count:new Set(ids).size,ids,materialized:materialized?{version:materialized.data.snapshot_version,cursor:materialized.cursor,bytes:new TextEncoder().encode(JSON.stringify(materialized)).length,complete:materialized.data.snapshot_complete}:null};
});}
test('WEB-MSG-CHATS-CACHE offline reload keeps the complete reordered chat view',async({page,request},info)=>{
    let hold=false,tail=0;const subscriptions=new Map<string,any>(),wire:any[]=[],states:any[]=[];
    await page.routeWebSocket('**/web/v1/live',route=>{const server=route.connectToServer();route.onMessage(message=>{const frame=JSON.parse(String(message));if(frame.t==='sub')subscriptions.set(frame.id,frame.args);server.send(message);});server.onMessage(message=>{const frame=JSON.parse(String(message)),args=subscriptions.get(frame.sub);if(hold&&frame.t==='snap'&&args?.after&&++tail===2){wire.push({kind:'held-tail',sub:frame.sub,args,cursor:frame.cursor});return;}route.send(message);});});
    try{
        await page.goto('/sw.js');await page.evaluate(async()=>{
            const db=await new Promise<IDBDatabase>((resolve,reject)=>{const open=indexedDB.open('ab.views',1);open.onupgradeneeded=()=>open.result.createObjectStore('views',{keyPath:'key'});open.onsuccess=()=>resolve(open.result);open.onerror=()=>reject(open.error);});
            await new Promise<void>((resolve,reject)=>{const tx=db.transaction('views','readwrite');tx.objectStore('views').put({key:JSON.stringify(['chats',{filter:'all',after:'stale-upgrade-page'}]),owner:'obsolete',data:{chats:[]}});tx.oncomplete=()=>resolve();tx.onerror=()=>reject(tx.error);});db.close();
        });
        await page.goto('/__fixtures/live');const count=page.getByLabel('Live count',{exact:true}),rows=page.locator('.fixture>p[data-chat]');
        await expect.poll(async()=>Number(await count.textContent())).toBeGreaterThanOrEqual(540);const total=Number(await count.textContent());
        await expect.poll(async()=>(await snapshot(page)).count).toBe(total);states.push({phase:'before',cache:await snapshot(page),rendered:total});
        hold=true;await control(request,'/list-late-incoming');await expect(rows.first()).toHaveText('Conversation 539');await expect(count).toHaveText(String(total));
        await expect.poll(()=>wire.length).toBe(1);
        await expect.poll(async()=>(await snapshot(page)).ids[0]).toBe(await rows.first().getAttribute('data-chat'));
        const ids=await rows.evaluateAll(elements=>elements.map(element=>(element as HTMLElement).dataset.chat));expect(new Set(ids).size).toBe(total);const frozen=await snapshot(page);expect(frozen.count).toBe(total);expect(frozen.ids).toEqual(ids);states.push({phase:'tail-in-flight',cache:frozen,rendered:Number(await count.textContent()),ids});
        await page.routeWebSocket('**/web/v1/live',()=>{});const warmStart=Date.now();await page.reload();await expect(count).toHaveText(String(total));const warmReload=Date.now()-warmStart;
        await expect(rows.first()).toHaveText('Conversation 539');expect(await rows.evaluateAll(elements=>elements.map(element=>(element as HTMLElement).dataset.chat))).toEqual(ids);
        const cached=await snapshot(page);expect(cached.pages).toEqual([]);expect(cached.materialized?.version).toBe(1);expect(cached.materialized?.cursor.epoch).toBeTruthy();expect(cached.materialized?.bytes).toBeLessThanOrEqual(2*1024*1024);expect(cached.ids).toEqual(ids);
        states.push({phase:'offline-complete',cache:cached,rendered:ids.length,warm_reload_ms:warmReload});
        await page.goto('/settings/devices');await page.getByRole('button',{name:'Sign out',exact:true}).click();await expect.poll(()=>page.evaluate(()=>indexedDB.databases())).toEqual([]);
        await page.routeWebSocket('**/web/v1/live',route=>{route.connectToServer();});
        await bootstrap(page.context(),request,'public');await page.goto('/__fixtures/live');await expect(count).toHaveText(String(total));await expect.poll(()=>page.evaluate(()=>indexedDB.databases())).toEqual([]);
        states.push({phase:'public-memory-only',rendered:Number(await count.textContent()),databases:await page.evaluate(()=>indexedDB.databases())});
    }finally{
        states.push({phase:'after',cache:await snapshot(page).catch(()=>null),rendered:await page.getByLabel('Live count',{exact:true}).textContent().catch(()=>null)});
        await mkdir('../lab/evidence/private/web/cache-chain',{recursive:true});await writeFile('../lab/evidence/private/web/cache-chain/'+info.project.name+'.json',JSON.stringify({wire,states},null,2));
    }
});

test('WEB-MSG-CHATS-CACHE device revocation wipes the materialized chats snapshot',async({page,request,browser})=>{
    const sent:any[]=[];page.on('websocket',socket=>socket.on('framesent',event=>{try{sent.push(JSON.parse(String(event.payload)));}catch{}}));
    const current=await page.request.get('/web/v1/session').then(response=>response.json());
    await page.goto('/__fixtures/live');await expect.poll(async()=>(await snapshot(page)).count).toBeGreaterThanOrEqual(540);
    const cursor=(await snapshot(page)).materialized!.cursor;sent.length=0;await page.reload();await expect.poll(()=>sent.some(frame=>frame.t==='sub'&&frame.view==='chats'&&!frame.args.after&&frame.cursor?.epoch===cursor.epoch&&frame.cursor?.sequence===cursor.sequence)).toBe(true);
    await expect(page.getByLabel('Live count',{exact:true})).toHaveText('577');
    const controller=await browser.newContext({baseURL:process.env.AB_WEB_URL});try{
        await bootstrap(controller,request);const response=await controller.request.delete('/web/v1/devices/'+encodeURIComponent(current.device_id),{headers:{Origin:process.env.AB_WEB_URL!}});expect(response.ok()).toBe(true);
        await expect(page.getByRole('heading',{name:'Sign in with a passkey'})).toBeVisible();await expect.poll(()=>page.evaluate(()=>indexedDB.databases())).toEqual([]);
    }finally{await controller.close();}
});
