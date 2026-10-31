import {timingFloor} from './timing-floor';
import {test,expect,type WebSocket as Socket} from '@playwright/test';
import {mkdir,writeFile} from 'node:fs/promises';
import {createHash} from 'node:crypto';
import {prepareSession,control} from './session';
import {transcriptInputs,transcriptObservers} from './transcript-performance';
import {installProbe,startTrace,stopTrace,stats} from './list-performance';
test.use({trace:'off'});
test.setTimeout(90000);
test.beforeEach(async({page})=>{await transcriptObservers(page);});
test.afterEach(async({page},info)=>{
    if(info.status==='passed')return;
    const partial=await page.evaluate(()=>{const probe=(window as any).__listProbe;return probe&&{actions:probe.actions?.map((action:any)=>({index:action.index,at:action.at,processed:action.processed,frame:action.frame,resultAt:action.resultAt,reactions:action.reactions})),actionFrames:probe.actionFrames,phaseDone:probe.phaseDone,echoes:[...document.querySelectorAll<HTMLElement>('[data-local-echo]')].map(row=>({message:row.dataset.message,command:row.dataset.commandId,state:row.dataset.sendState,text:row.textContent?.slice(0,80)}))};}).catch(()=>undefined);
    if(!partial)return;
    await mkdir('../lab/evidence/private/web',{recursive:true});
    const action=info.title.includes('sending text')?'send':info.title.includes('reactions')?'react':'other';
    await writeFile(`../lab/evidence/private/web/transcript-failure-${action}-${info.project.name}.json`,JSON.stringify(partial,null,2)+'\n');
});
test('WEB-MSG-PERF WEB-MSG-PERF-FRAME a 100k-message transcript fling records bounded rendering and frame coverage',async({page,request},info)=>{
    await prepareSession(page);const ids=await control(request,'/demo-fixtures');let population=0;const releases:(()=>void)[]=[];
    const observe=(socket:Socket)=>{const read=(event:{payload:string|Buffer})=>{const frame=JSON.parse(String(event.payload));if(frame.t==='snap'&&frame.data?.items?.[0]?.data.chat_id===ids['demo-chat-39'])population=Math.max(population,frame.data.items.length+Number(frame.data.older??0)+Number(frame.data.newer??0));};socket.on('framereceived',read);releases.push(()=>socket.off('framereceived',read));};
    page.on('websocket',observe);await page.goto('/messages/'+encodeURIComponent(ids['demo-chat-39']));
    const transcript=page.locator('.messages-transcript'),viewport=page.getByRole('listbox',{name:'Messages transcript'});
    await expect(transcript).toHaveAttribute('aria-busy','false');await expect.poll(async()=>Number(await transcript.getAttribute('data-window-size'))).toBeGreaterThanOrEqual(200);
    expect(population).toBe(100000);page.off('websocket',observe);releases.forEach(release=>release());
    await expect(page.locator('.transcript')).toHaveCSS('transform','none');await page.waitForTimeout(300);await installProbe(page,true,page.context().browser()?.browserType().name()!=='chromium');
    const stageProfile=Boolean(process.env.AB_WEB_STAGE_PROFILE);
    if(stageProfile)await page.evaluate(()=>{(window as any).__abWork={phase:'fling',samples:[]};});
    const tracing=await startTrace(page),profileCPU=Boolean(process.env.AB_WEB_CPU_PROFILE&&tracing);
    if(profileCPU){await tracing!.send('Profiler.enable');await tracing!.send('Profiler.start');}
    await viewport.evaluate(async element=>{
        const probe=(window as any).__listProbe;probe.scrollStart=performance.now();probe.phase='scroll';
        for(let index=0;index<181;index++)await new Promise<void>(resolve=>requestAnimationFrame(()=>{performance.mark('list-frame-'+index);element.scrollTop=Math.max(0,element.scrollTop-160);resolve();}));
        await new Promise<void>(resolve=>(window as any).__listNativeRAF(()=>resolve()));
        probe.phase='idle';probe.scrollEnd=performance.now();
    });
    const cpu=profileCPU?await tracing!.send('Profiler.stop'):undefined;
    const trace=await stopTrace(tracing,'scroll',true),probe=await page.evaluate(()=>(window as any).__listProbe),work=await page.evaluate(()=>(window as any).__abWork?.samples??[]);
    const {trace:raw,...chromium}=trace??{},metrics={population,stage_profiling:stageProfile,playwright_recording:false,cpu_sampling:profileCPU,frame_work:trace?.frame_work??stats(probe.frames.map((frame:any)=>frame.work)),in_page_frame_work:stats(probe.frames.map((frame:any)=>frame.work)),frame_intervals:stats(probe.frames.slice(1).map((frame:any,index:number)=>frame.at-probe.frames[index].at)),chromium:trace?chromium:null,product_work:work,observer_work:probe.observers??[],long_tasks:probe.longTasks,window_size:Number(await transcript.getAttribute('data-window-size')),rendered:await transcript.locator('[data-message]').count(),endpoint:'main-thread script, style, layout and paint during an upward fling in the bounded window over the 100000-message synthetic chat'};
    await mkdir('../lab/evidence/web/metrics',{recursive:true});await mkdir('../lab/evidence/private/web',{recursive:true});
    await writeFile(`../lab/evidence/web/metrics/WEB-MSG-PERF-fling-${info.project.name}.json`,JSON.stringify({digest:createHash('sha256').update(JSON.stringify(metrics)).digest('hex'),counts:{population,frames:metrics.frame_work.samples,window_size:metrics.window_size,rendered:metrics.rendered,long_tasks:metrics.long_tasks.length},frame_budget_pass:metrics.frame_work.p95_ms<=4&&metrics.frame_work.max_ms<=8&&metrics.long_tasks.length===0,frame_work:metrics.frame_work,native_paint_observed:Boolean(trace),timing_floor:await timingFloor(page),complete_frame_budget_proven:false,app_paint_budget_proven:false,native_layout_observed:Boolean(trace)},null,2)+'\n');if(raw)await writeFile(`../lab/evidence/private/web/transcript-fling-${info.project.name}.json`,raw);if(cpu)await writeFile(`../lab/evidence/private/web/transcript-cpu-${info.project.name}.json`,JSON.stringify(cpu));
    expect(metrics.frame_work.samples).toBeGreaterThanOrEqual(100);expect(metrics.window_size).toBeLessThanOrEqual(500);expect(metrics.rendered).toBeLessThan(100);
    expect(metrics.frame_work.p95_ms).toBeLessThanOrEqual(4);expect(metrics.frame_work.max_ms).toBeLessThanOrEqual(8);expect(metrics.long_tasks).toHaveLength(0);
});

test('WEB-MSG-PERF WEB-MSG-PERF-INPUT open chat records input and phone push/pop frame coverage',async({page},info)=>{
    await prepareSession(page);await page.goto('/messages');await expect(page.locator('.conversation-row').first()).toBeVisible();
    await transcriptInputs(page,info,'open-chat',async()=>{
        for(let index=0;index<24;index++){await page.locator('.conversation-row').nth(index%2).click();await expect(page.locator('.messages-transcript')).toHaveAttribute('aria-busy','false');await expect.poll(()=>page.evaluate(()=>(window as any).__listProbe.actions.at(-1)?.resultAt??0),{intervals:[25]}).toBeGreaterThan(0);if(info.project.name.endsWith('-phone'))await page.getByRole('button',{name:'Back',exact:true}).click();}
    });
});
test('WEB-MSG-PERF WEB-MSG-PERF-INPUT sending text records complete input coverage',async({page,request},info)=>{
    await prepareSession(page);const ids=await control(request,'/demo-fixtures');await page.goto('/messages/'+encodeURIComponent(ids['message-chat']));await expect(page.locator('.messages-transcript')).toHaveAttribute('aria-busy','false');
    await transcriptInputs(page,info,'send',async()=>{
        for(let index=0;index<24;index++){await page.getByRole('textbox',{name:'Message',exact:true}).fill('[ab-test] performance '+index);await page.getByRole('button',{name:'Send message',exact:true}).click();await expect(page.getByRole('textbox',{name:'Message',exact:true})).toHaveValue('');await expect.poll(()=>page.locator('[data-local-echo]').count(),{intervals:[25]}).toBe(0);}
    });
});
test('WEB-MSG-PERF WEB-MSG-PERF-INPUT reactions record complete input coverage',async({page,request},info)=>{
    await prepareSession(page);const ids=await control(request,'/demo-fixtures');await page.goto('/messages/'+encodeURIComponent(ids['message-chat'])+'/'+encodeURIComponent(ids['demo-message-00-000004']));await expect(page.locator('.messages-transcript')).toHaveAttribute('aria-busy','false');
    await transcriptInputs(page,info,'react',async()=>{
        for(let index=0;index<24;index++){await page.locator('[data-message="'+ids['demo-message-00-000004']+'"] .bubble').first().click({button:'right'});const menu=page.getByRole('dialog',{name:'Message actions',exact:true});await menu.getByRole('button',{name:'Love',exact:true}).click();await expect(menu).not.toBeVisible();await expect.poll(()=>page.evaluate(()=>(window as any).__listProbe.actions.at(-1)?.resultAt??0),{intervals:[25]}).toBeGreaterThan(0);}
    });
});
test('WEB-MSG-PERF WEB-MSG-PERF-INPUT opening media records complete input coverage',async({page,request},info)=>{
    await prepareSession(page);const ids=await control(request,'/demo-fixtures');await page.goto('/messages/'+encodeURIComponent(ids['message-chat'])+'/'+encodeURIComponent(ids['demo-message-00-000032']));await expect(page.locator('.messages-transcript')).toHaveAttribute('aria-busy','false');const source=page.locator('[data-message="'+ids['demo-message-00-000032']+'"] .media-open');await expect(source).toBeVisible();await expect.poll(()=>source.locator('..').locator('img').evaluate((image:HTMLImageElement)=>image.complete)).toBe(true);
    await transcriptInputs(page,info,'open-media',async()=>{
        for(let index=0;index<24;index++){await source.click();const viewer=page.getByRole('dialog',{name:'Media viewer',exact:true});await expect(viewer).toBeVisible();await expect(viewer).toHaveAttribute('data-media-loading','false');await expect(viewer.locator('.media-viewer-stage .media-image')).toHaveAttribute('data-media-state','ready');await page.keyboard.press('Escape');await expect(viewer).not.toBeVisible();}
    });
});
