import { expect } from '@playwright/test';
import { test } from './list-session';
import { mkdir, writeFile } from 'node:fs/promises';
import { installProbe, startTrace, stopTrace, stats } from './list-performance';
test.beforeEach(async({page})=>{await page.goto('/messages');await expect(page.getByRole('listbox',{name:'Conversations'})).toBeVisible();});
test('WEB-MSG-PERF list: 540 conversations, frame work, input timing and cache',async({page},info)=>{
    const viewport=page.getByRole('listbox',{name:'Conversations'});
    await expect.poll(async()=>Number(await viewport.locator('[aria-setsize]').first().getAttribute('aria-setsize'))).toBeGreaterThanOrEqual(540);
    expect(await page.locator('.conversation-row').count()).toBeLessThan(40);
    await page.waitForTimeout(300);
    await installProbe(page);
    const tracing=await startTrace(page);
    await viewport.evaluate(async element=>{
        (window as any).__listProbe.scrollStart=performance.now();(window as any).__listProbe.phase='scroll';
        for(let index=0;index<121;index++)await new Promise<void>(resolve=>requestAnimationFrame(()=>{performance.mark('list-frame-'+index);element.scrollTop=index*200;resolve();}));
        (window as any).__listProbe.phase='idle';(window as any).__listProbe.scrollEnd=performance.now();
    });
    const trace=await stopTrace(tracing);
    const probe=await page.evaluate(()=>(window as any).__listProbe);
    expect(await page.locator('.conversation-row').count()).toBeLessThan(40);
    await viewport.evaluate(element=>element.scrollTop=0);
    const selectionTracing=await startTrace(page);
    for(let index=0;index<20;index++){
        await page.locator('.conversation-row').nth(index%2).click();
        if(info.project.use.viewport?.width===390)await page.getByRole('button',{name:'Back',exact:true}).click();
    }
    await expect.poll(()=>page.evaluate(()=>(window as any).__listProbe.inputFrames)).toBe(20);
    await expect.poll(()=>page.evaluate(()=>(window as any).__listProbe.events.filter((entry:any)=>entry.name==='click').length)).toBe(20);
    const selection=await stopTrace(selectionTracing,'input');
    const input=await page.evaluate(()=>(window as any).__listProbe);
    await expect.poll(()=>page.evaluate(async()=>{const db=await new Promise<IDBDatabase>(resolve=>{const open=indexedDB.open('ab.views');open.onsuccess=()=>resolve(open.result);});const values=await new Promise<any[]>(resolve=>{const read=db.transaction('views').objectStore('views').getAll();read.onsuccess=()=>resolve(read.result);});db.close();return Math.max(0,...values.filter(value=>value.data.snapshot_version===1).map(value=>value.data.chats?.length??0));})).toBeGreaterThanOrEqual(540);
    await page.routeWebSocket('**/web/v1/live',()=>{});
    await page.addInitScript(()=>{const observer=new MutationObserver(()=>{if(!document.querySelector('.conversation-row'))return;observer.disconnect();requestAnimationFrame(()=>requestAnimationFrame(()=>{(window as any).__listCachedAt=performance.now();}));});observer.observe(document,{childList:true,subtree:true});});
    const warm:number[]=[];
    for(let index=0;index<3;index++){await page.reload();await expect(page.locator('.conversation-row').first()).toBeVisible();await expect.poll(async()=>Number(await page.getByRole('listbox',{name:'Conversations'}).locator('[aria-setsize]').first().getAttribute('aria-setsize'))).toBeGreaterThanOrEqual(540);await expect.poll(()=>page.evaluate(()=>(window as any).__listCachedAt??0)).toBeGreaterThan(0);warm.push(await page.evaluate(()=>(window as any).__listCachedAt));}
    await mkdir('../lab/evidence/web/metrics',{recursive:true});
    await mkdir('../lab/evidence/private/web',{recursive:true});
    const {trace:raw,...chromium}=trace??{};
    expect(input.inputRendered).toEqual(Array(20).fill(true));expect(selection?.frames).toHaveLength(20);
    const responsiveness=input.events.filter((entry:any)=>entry.name==='click').map((entry:any,index:number)=>({delay_and_processing_ms:entry.processingEnd-entry.startTime,frame_work_ms:selection!.frames[index],total_ms:entry.processingEnd-entry.startTime+selection!.frames[index]}));
    await writeFile(`../lab/evidence/private/web/list-input-trace-${info.project.name}.json`,selection!.trace);
    if(raw)await writeFile(`../lab/evidence/private/web/list-trace-${info.project.name}.json`,raw);
    const metrics={frame_work:trace?.frame_work??stats(probe.frames.map((frame:any)=>frame.work)),chromium:trace?chromium:null,in_page_frame_work:stats(probe.frames.map((frame:any)=>frame.work)),input_responsiveness_ms:stats(responsiveness.map((sample:any)=>sample.total_ms)),input_responsiveness_samples:responsiveness,click_to_next_frame_ms:stats(input.input),trusted_click_to_raf_samples_ms:input.input,input_endpoint:'trusted click capture to next requestAnimationFrame callback',input_8ms_proven:false,event_timing_threshold_ms:16,event_timing:input.events,warm_load:stats(warm),long_tasks:probe.longTasks,frame_intervals:stats(probe.frames.slice(1).map((frame:any,index:number)=>frame.at-probe.frames[index].at)),host:'Air, shared heavy-job scheduler, 60 Hz headless; intervals recorded only'};
    await writeFile(`../lab/evidence/web/metrics/WEB-MSG-PERF-list-${info.project.name}.json`,JSON.stringify(metrics,null,2)+'\n');
    expect(metrics.input_responsiveness_ms.p95_ms).toBeLessThanOrEqual(8);expect(metrics.frame_work.p95_ms).toBeLessThanOrEqual(4);expect(metrics.frame_work.max_ms).toBeLessThanOrEqual(8);expect(metrics.long_tasks).toEqual([]);expect(metrics.warm_load.max_ms).toBeLessThan(1000);
});
