import { expect, test } from '@playwright/test';
import { prepareSession } from './session';
import { mkdir, writeFile } from 'node:fs/promises';
test('WEB-CACHE-POLICY evicts oldest timestamps without reading cached payloads',async({page},info)=>{
    await prepareSession(page);await page.routeWebSocket('**/web/v1/live',()=>{});
    await page.goto('/__fixtures/cache');await expect(page.getByRole('button',{name:'Persist fixture view'})).toBeVisible();
    await page.evaluate(async()=>{
        const db=await new Promise<IDBDatabase>((resolve,reject)=>{const open=indexedDB.open('ab.views');open.onsuccess=()=>resolve(open.result);open.onerror=()=>reject(open.error);});
        await new Promise<void>((resolve,reject)=>{const tx=db.transaction('views','readwrite'),store=tx.objectStore('views');store.clear();for(let index=0;index<110;index++)store.put({key:'eviction-'+index,owner:'fixture',data:{records:[],payload:'x'.repeat(4096)},at:(110-index)*10});tx.oncomplete=()=>resolve();tx.onerror=()=>reject(tx.error);});db.close();
        const probe={payload_reads:0,key_reads:0,active:true,visible_frame:false,writes_after_frame:[] as boolean[]};
        window.addEventListener('click',()=>requestAnimationFrame(()=>{probe.visible_frame=true;}),{capture:true,once:true});
        const put=IDBObjectStore.prototype.put;IDBObjectStore.prototype.put=function(...args:Parameters<typeof put>){if(probe.active)probe.writes_after_frame.push(probe.visible_frame);return put.apply(this,args);};(window as any).__evictionProbe=probe;
        for(const prototype of [IDBObjectStore.prototype,IDBIndex.prototype])for(const key of ['get','getAll','openCursor','getAllKeys','openKeyCursor'] as const){
            const original=(prototype as any)[key];(prototype as any)[key]=function(...args:any[]){if(probe.active)probe[key==='getAllKeys'||key==='openKeyCursor'?'key_reads':'payload_reads']++;return original.apply(this,args);};
        }
    });
    await page.getByRole('button',{name:'Persist fixture view'}).click();await expect(page.getByRole('status')).toHaveText('Persisted');
    const result=await page.evaluate(async()=>{
        const probe=(window as any).__evictionProbe;probe.active=false;
        const db=await new Promise<IDBDatabase>(resolve=>{const open=indexedDB.open('ab.views');open.onsuccess=()=>resolve(open.result);});
        const keys=await new Promise<IDBValidKey[]>(resolve=>{const read=db.transaction('views').objectStore('views').getAllKeys();read.onsuccess=()=>resolve(read.result);});db.close();
        return {payload_reads:probe.payload_reads,key_reads:probe.key_reads,writes_after_frame:probe.writes_after_frame,keys};
    });
    await mkdir('../lab/evidence/web/metrics',{recursive:true});await writeFile('../lab/evidence/web/metrics/WEB-CACHE-POLICY-eviction-'+info.project.name+'.json',JSON.stringify(result,null,2)+'\n');
    expect(result.keys).toHaveLength(100);expect(result.keys).toEqual(expect.arrayContaining(['eviction-new',...Array.from({length:99},(_,index)=>'eviction-'+index)]));
    expect(result.payload_reads).toBe(0);expect(result.key_reads).toBe(1);expect(result.writes_after_frame).toEqual([true]);
});
