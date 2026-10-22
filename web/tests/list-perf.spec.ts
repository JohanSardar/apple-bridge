import { expect, type Page, type APIRequestContext, type TestInfo } from '@playwright/test';
import { test } from './list-session';
import { mkdir, writeFile } from 'node:fs/promises';
import { installProbe, startTrace, stopTrace, stats } from './list-performance';
import { installActionProbe, measureActions, observedActions } from './list-actions-performance';
import { control } from './session';
test.beforeEach(async({page})=>{
    await page.addInitScript(()=>{const state={scrolled:false,switches:0};(window as any).__listFilterHistory=state;document.addEventListener('scroll',event=>{const target=event.target;if(target instanceof HTMLElement&&target.matches('.virtual-viewport')&&target.scrollTop!==0)state.scrolled=true;},true);window.addEventListener('click',event=>{if(event.isTrusted&&(event.target as Element).closest('[role=menuitemradio]'))state.switches++;},true);});
    await page.goto('/messages');await expect(page.getByRole('listbox',{name:'Conversations'})).toBeVisible();});
test('WEB-PERF-LIST scroll, selection, first filter switch, search, live reorder and warm cache',async({page,request},info)=>{
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
    const selection=await stopTrace(selectionTracing,'action');
    const input=await page.evaluate(()=>(window as any).__listProbe);
    await measureListActions(page,request,info);
    await expect.poll(()=>page.evaluate(async()=>{const db=await new Promise<IDBDatabase>(resolve=>{const open=indexedDB.open('ab.views');open.onsuccess=()=>resolve(open.result);});const values=await new Promise<any[]>(resolve=>{const read=db.transaction('views').objectStore('views').getAll();read.onsuccess=()=>resolve(read.result);});db.close();return Math.max(0,...values.filter(value=>value.data.snapshot_version===1).map(value=>value.data.chats?.length??0));})).toBeGreaterThanOrEqual(540);
    await page.routeWebSocket('**/web/v1/live',()=>{});
    await page.addInitScript(()=>{const observer=new MutationObserver(()=>{if(!document.querySelector('.conversation-row'))return;observer.disconnect();requestAnimationFrame(()=>requestAnimationFrame(()=>{(window as any).__listCachedAt=performance.now();}));});observer.observe(document,{childList:true,subtree:true});});
    const warm:number[]=[];
    for(let index=0;index<3;index++){await page.reload();await expect(page.locator('.conversation-row').first()).toBeVisible();await expect.poll(async()=>Number(await page.getByRole('listbox',{name:'Conversations'}).locator('[aria-setsize]').first().getAttribute('aria-setsize'))).toBeGreaterThanOrEqual(540);await expect.poll(()=>page.evaluate(()=>(window as any).__listCachedAt??0)).toBeGreaterThan(0);warm.push(await page.evaluate(()=>(window as any).__listCachedAt));}
    await mkdir('../lab/evidence/web/metrics',{recursive:true});
    await mkdir('../lab/evidence/private/web',{recursive:true});
    const {trace:raw,...chromium}=trace??{};
    expect(input.inputRendered).toEqual(Array(20).fill(true));expect(selection?.frames).toHaveLength(20);
    const responsiveness=input.events.filter((entry:any)=>entry.name==='click').map((entry:any,index:number)=>({delay_and_processing_ms:entry.processingEnd-entry.startTime,event_processing_ms:entry.processing,trace_dispatch_processing_ms:selection!.dispatch_processing_ms[index],frame_work_ms:selection!.frames[index],total_ms:entry.processingEnd-entry.startTime+selection!.frames[index]}));
    await writeFile(`../lab/evidence/private/web/list-input-trace-${info.project.name}.json`,selection!.trace);
    if(raw)await writeFile(`../lab/evidence/private/web/list-trace-${info.project.name}.json`,raw);
    const metrics={frame_work:trace?.frame_work??stats(probe.frames.map((frame:any)=>frame.work)),chromium:trace?chromium:null,in_page_frame_work:stats(probe.frames.map((frame:any)=>frame.work)),input_responsiveness_ms:stats(responsiveness.map((sample:any)=>sample.total_ms)),input_responsiveness_samples:responsiveness,input_frame_endpoint:selection!.endpoint,click_to_next_frame_ms:stats(input.input),trusted_click_to_raf_samples_ms:input.input,input_endpoint:'trusted click capture to next requestAnimationFrame callback',input_8ms_proven:false,event_timing_threshold_ms:16,event_timing:input.events,warm_load:stats(warm),long_tasks:probe.longTasks,frame_intervals:stats(probe.frames.slice(1).map((frame:any,index:number)=>frame.at-probe.frames[index].at)),host:'Air, shared heavy-job scheduler, 60 Hz headless; intervals recorded only'};
    await writeFile(`../lab/evidence/web/metrics/WEB-PERF-LIST-list-${info.project.name}.json`,JSON.stringify(metrics,null,2)+'\n');
    expect(metrics.input_responsiveness_ms.p95_ms).toBeLessThanOrEqual(8);expect(metrics.frame_work.p95_ms).toBeLessThanOrEqual(4);expect(metrics.frame_work.max_ms).toBeLessThanOrEqual(8);expect(metrics.long_tasks).toEqual([]);expect(metrics.warm_load.max_ms).toBeLessThan(1000);
});
async function measureListActions(page:Page,request:APIRequestContext,info:TestInfo){
    const earlier=await page.evaluate(()=>(window as any).__listFilterHistory);expect(earlier.switches).toBe(0);expect(earlier.scrolled).toBe(true);
    await page.goto('/messages');
    await mkdir('../lab/evidence/web/metrics',{recursive:true});await mkdir('../lab/evidence/private/web',{recursive:true});
    const rows=page.locator('.conversation-row'),viewport=page.getByRole('listbox',{name:'Conversations'}),project=info.project.name;
    await expect.poll(async()=>Number(await viewport.locator('[aria-setsize]').first().getAttribute('aria-setsize'))).toBeGreaterThanOrEqual(540);
    const beforeFilters=await viewport.evaluate((element,previous)=>({current_document_scrolled:(window as any).__listFilterHistory.scrolled,scroll_top:element.scrollTop,unmeasured_filter_switches:(window as any).__listFilterHistory.switches,earlier_phase_scrolled:previous.scrolled,earlier_unmeasured_filter_switches:previous.switches,page_reloaded_before_filters:true}),earlier);
    expect(beforeFilters.unmeasured_filter_switches).toBe(0);
    await installProbe(page);await installActionProbe(page);
    await page.evaluate(()=>{(window as any).__listProbe.filterExpectedCounts={'All Messages':Number(document.querySelector('[role=listbox][aria-label=Conversations] [aria-setsize]')?.getAttribute('aria-setsize')),'Unread':Number(document.title.match(/^\((\d+)\)/)?.[1])};});
    await measureActions(page,'filters',project,async()=>{
        for(let index=0;index<64;index++){
            for(const label of ['Unread','All Messages']){await page.getByRole('button',{name:'Filters and Edit',exact:true}).click();await page.getByRole('menuitemradio',{name:label,exact:true}).click();await expect(page.locator('.conversation-list')).toHaveAttribute('data-filter',label);}
            await page.waitForTimeout(60);if(await observedActions(page)>=20)break;
        }
    },true,beforeFilters);
    await expect(rows.first()).toContainText('Maya Chen');
    const search=page.getByRole('searchbox',{name:'Search',exact:true}),keys=['m','a','y','a','Backspace','Backspace','Backspace','Backspace'];
    await measureActions(page,'search',project,async()=>{
        for(let index=0;index<128;index++){await search.press(keys[index%keys.length]);if(index%8===3)await expect(rows).toHaveCount(3);await page.waitForTimeout(60);if(index%8===7&&await observedActions(page)>=20)break;}
    },true);
    await expect(search).toHaveValue('');
    await measureActions(page,'live',project,async()=>{await control(request,'/list-fixture?update=1');await expect(rows.first()).toContainText('Weekend Crew');await expect(rows.first()).toContainText('The plan is ready');},false);
    expect(await rows.count()).toBeLessThan(40);
}
