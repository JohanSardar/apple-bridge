import {timingFloor} from './timing-floor';
import {test,expect,type WebSocket as Socket} from '@playwright/test';
import {mkdir,writeFile} from 'node:fs/promises';
import {createHash,randomUUID} from 'node:crypto';
import {prepareSession,control} from './session';
import {transcriptInputs,transcriptObservers} from './transcript-performance';
import {installProbe,startTrace,stopTrace,stats} from './list-performance';
test.use({trace:'off'});
test.setTimeout(90000);
test.beforeEach(async({page})=>{await transcriptObservers(page);});
test.afterEach(async({page},info)=>{
    if(info.status==='passed')return;
    const partial=await page.evaluate(()=>{const probe=(window as any).__listProbe;return probe&&{actions:probe.actions?.map((action:any)=>({index:action.index,at:action.at,processed:action.processed,frame:action.frame,resultAt:action.resultAt,reactions:action.reactions})),actionFrames:probe.actionFrames,phaseDone:probe.phaseDone,viewports:[...document.querySelectorAll<HTMLElement>('.messages-transcript .virtual-viewport')].map(element=>({top:element.scrollTop,height:element.clientHeight,total:element.scrollHeight,bounds:element.getBoundingClientRect().toJSON(),content:element.firstElementChild?.getAttribute('style')})),rows:[...document.querySelectorAll<HTMLElement>('.message-row')].map(row=>({message:row.dataset.message,command:row.dataset.commandId,state:row.dataset.sendState,text:row.textContent?.slice(0,80),bounds:row.getBoundingClientRect().toJSON()})),echoes:[...document.querySelectorAll<HTMLElement>('[data-local-echo]')].map(row=>({message:row.dataset.message,command:row.dataset.commandId,state:row.dataset.sendState,text:row.textContent?.slice(0,80)}))};}).catch(()=>undefined);
    if(!partial)return;
    await mkdir('../lab/evidence/private/web',{recursive:true});
    const action=info.title.includes('sending text')?'send':info.title.includes('reactions')?'react':'other';
    await writeFile(`../lab/evidence/private/web/transcript-failure-${action}-${info.project.name}.json`,JSON.stringify(partial,null,2)+'\n');
});
test('WEB-MSG-PERF WEB-MSG-PERF-FRAME a 100k-message transcript fling records bounded rendering and frame coverage',async({page,request},info)=>{
    await prepareSession(page);const ids=await control(request,'/demo-fixtures');let population=0;const releases:(()=>void)[]=[];
    const observe=(socket:Socket)=>{const read=(event:{payload:string|Buffer})=>{const frame=JSON.parse(String(event.payload));if(frame.t==='snap'&&frame.data?.items?.[0]?.data.chat_id===ids['demo-chat-39'])population=Math.max(population,frame.data.items.length+Number(frame.data.older??0)+Number(frame.data.newer??0));};socket.on('framereceived',read);releases.push(()=>socket.off('framereceived',read));};
    await page.addInitScript(()=>{
        const states:{count:number;busy:boolean}[]=[];(window as any).__transcriptBatches=states;
        const observer=new MutationObserver(()=>{
            const list=document.querySelector('[role=listbox][aria-label="Messages transcript"]');if(!list)return;
            const state={count:list.querySelectorAll('.virtual-row').length,busy:list.getAttribute('aria-busy')==='true'},previous=states.at(-1);
            if(state.count&&(!previous||previous.count!==state.count||previous.busy!==state.busy))states.push(state);
        });observer.observe(document,{subtree:true,childList:true,attributes:true,attributeFilter:['aria-busy']});(window as any).__stopTranscriptBatches=()=>observer.disconnect();
    });
    page.on('websocket',observe);await page.goto('/messages/'+encodeURIComponent(ids['demo-chat-39']));
    const transcript=page.locator('.messages-transcript'),viewport=page.getByRole('listbox',{name:'Messages transcript'});
    await expect(transcript).toHaveAttribute('aria-busy','false');await expect.poll(async()=>Number(await transcript.getAttribute('data-window-size'))).toBeGreaterThanOrEqual(200);
    await expect(viewport).toHaveAttribute('aria-busy','false');
    const batches=await page.evaluate(()=>{(window as any).__stopTranscriptBatches();return (window as any).__transcriptBatches as {count:number;busy:boolean}[];});
    expect(batches.length).toBeGreaterThan(0);expect(batches[0].count).toBeLessThanOrEqual(1);expect(batches.at(-1)?.busy).toBe(false);if(batches.at(-1)!.count>1)expect(batches.some(state=>state.busy)).toBe(true);
    expect(population).toBe(100000);page.off('websocket',observe);releases.forEach(release=>release());
    await expect(page.locator('.transcript')).toHaveCSS('transform','none');await page.waitForTimeout(300);await installProbe(page,true,page.context().browser()?.browserType().name()!=='chromium');
    const stageProfile=process.env.AB_WEB_STAGE_PROFILE==='1'||process.env.AB_WEB_STAGE_PROFILE==='fling';
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
    await writeFile(`../lab/evidence/web/metrics/WEB-MSG-PERF-fling-${info.project.name}.json`,JSON.stringify({digest:createHash('sha256').update(JSON.stringify(metrics)).digest('hex'),counts:{population,frames:metrics.frame_work.samples,window_size:metrics.window_size,rendered:metrics.rendered,long_tasks:metrics.long_tasks.length},frame_budget_pass:metrics.frame_work.p95_ms<=4&&metrics.frame_work.max_ms<=8&&metrics.long_tasks.length===0,frame_work:metrics.frame_work,native_paint_observed:Boolean(trace),timing_floor:await timingFloor(page),complete_frame_budget_proven:false,app_paint_budget_proven:false,native_layout_observed:Boolean(trace)},null,2)+'\n');await writeFile(`../lab/evidence/private/web/transcript-metrics-fling-${info.project.name}.json`,JSON.stringify(metrics));if(raw)await writeFile(`../lab/evidence/private/web/transcript-fling-${info.project.name}.json`,raw);if(cpu)await writeFile(`../lab/evidence/private/web/transcript-cpu-${info.project.name}.json`,JSON.stringify(cpu));
    expect(metrics.frame_work.samples).toBeGreaterThanOrEqual(100);expect(metrics.window_size).toBeLessThanOrEqual(500);expect(metrics.rendered).toBeLessThan(100);
    expect(metrics.frame_work.p95_ms).toBeLessThanOrEqual(4);expect(metrics.frame_work.max_ms).toBeLessThanOrEqual(8);expect(metrics.long_tasks).toHaveLength(0);
});

test('WEB-MSG-PERF WEB-MSG-PERF-INPUT open chat records input and phone push/pop frame coverage',async({page},info)=>{
    await prepareSession(page);await page.goto('/messages');await expect(page.locator('.conversation-row').first()).toBeVisible();
    await expect.poll(()=>page.locator('.conversation-list .virtual-viewport').evaluate(element=>{const phone=innerWidth<700;return element.querySelectorAll('.virtual-row').length<=Math.ceil(element.clientHeight/(phone?84:68))+(phone?3:7);})).toBe(true);
    await transcriptInputs(page,info,'open-chat',async()=>{
        for(let index=0;index<24;index++){await page.locator('.conversation-row').nth(index%2).click();await expect(page.locator('.messages-transcript')).toHaveAttribute('aria-busy','false');await expect.poll(()=>page.evaluate(()=>(window as any).__listProbe.actions.at(-1)?.resultAt??0),{intervals:[25]}).toBeGreaterThan(0);if(info.project.name.endsWith('-phone'))await page.getByRole('button',{name:'Back',exact:true}).click();}
    });
});
test('WEB-MSG-PERF WEB-MSG-PERF-INPUT sending text records complete input coverage',async({page,request},info)=>{
    await prepareSession(page);const ids=await control(request,'/demo-fixtures');await page.goto('/messages/'+encodeURIComponent(ids['message-chat']));await expect(page.locator('.messages-transcript')).toHaveAttribute('aria-busy','false');
    const prefix='[ab-test] '+randomUUID().slice(0,8).replace(/\d/g,digit=>'ghijklmnop'[Number(digit)]);
    await transcriptInputs(page,info,'send',async()=>{
        for(let index=0;index<24;index++){const content=prefix+' '+index;await page.getByRole('textbox',{name:'Message',exact:true}).fill(content);await page.getByRole('button',{name:'Send message',exact:true}).click();await expect(page.getByRole('textbox',{name:'Message',exact:true})).toHaveValue('');await expect.poll(()=>page.locator('[data-local-echo]').count(),{intervals:[25]}).toBe(0);const sent=page.locator('.messages-transcript .message-row.outgoing').getByText(content,{exact:true});await expect(sent).toHaveCount(1);await expect(page.locator('.messages-transcript .message-row:not(.outgoing)').getByText(content,{exact:true})).toHaveCount(1);await expect.poll(()=>page.locator('.messages-transcript .virtual-viewport').evaluate(element=>element.scrollHeight-element.clientHeight-element.scrollTop),{intervals:[25]}).toBeLessThanOrEqual(2);await expect(sent).toBeInViewport({ratio:1});}
    });
});
test('WEB-MSG-PERF WEB-MSG-PERF-INPUT reactions record complete input coverage',async({page,request},info)=>{
    await prepareSession(page);const ids=await control(request,'/demo-fixtures');await page.goto('/messages/'+encodeURIComponent(ids['message-chat'])+'/'+encodeURIComponent(ids['demo-message-00-000004']));await expect(page.locator('.messages-transcript')).toHaveAttribute('aria-busy','false');
    await transcriptInputs(page,info,'react',async()=>{
        for(let index=0;index<24;index++){await page.locator('[data-message="'+ids['demo-message-00-000004']+'"] .bubble').first().click({button:'right'});const menu=page.getByRole('dialog',{name:'Message actions',exact:true});await menu.getByRole('button',{name:'Love',exact:true}).click();await expect(menu).not.toBeVisible();await expect.poll(()=>page.evaluate(()=>(window as any).__listProbe.actions.at(-1)?.resultAt??0),{intervals:[25]}).toBeGreaterThan(0);}
    });
});
test('WEB-MSG-PERF WEB-MSG-PERF-INPUT opening media records complete input coverage',async({page,request},info)=>{
    await prepareSession(page);const ids=await control(request,'/demo-fixtures');await page.goto('/messages/'+encodeURIComponent(ids['message-chat'])+'/'+encodeURIComponent(ids['demo-message-00-000032']));await expect(page.locator('.messages-transcript')).toHaveAttribute('aria-busy','false');const source=page.locator('[data-message="'+ids['demo-message-00-000032']+'"] .media-open');await expect(source).toBeVisible();await expect(page.getByRole('listbox',{name:'Messages transcript'})).toHaveAttribute('aria-busy','false');await expect.poll(()=>source.locator('..').locator('img').evaluate((image:HTMLImageElement)=>image.complete)).toBe(true);
    await source.scrollIntoViewIfNeeded();await page.waitForTimeout(300);
    await page.evaluate(()=>window.addEventListener('click',event=>{if(event.isTrusted&&(event.target as Element).closest('.media-open'))(window as any).__mediaBackgroundRows=Array.from(document.querySelectorAll('.messages-transcript .virtual-row'));},true));
    let image:import('@playwright/test').ElementHandle<HTMLElement|SVGElement>|null=null;
    await transcriptInputs(page,info,'open-media',async()=>{
        for(let index=0;index<24;index++){await source.click();const viewer=page.getByRole('dialog',{name:'Media viewer',exact:true});await expect(viewer).toBeVisible();await expect(viewer).toHaveAttribute('data-media-loading','false');await expect(viewer.locator('.media-viewer-stage .media-image')).toHaveAttribute('data-media-state','ready');expect(await page.locator('.messages-transcript .virtual-row').evaluateAll(rows=>{const previous=(window as any).__mediaBackgroundRows as Element[];return previous.every(row=>rows.includes(row));})).toBe(true);if(image)expect(await image.evaluate(element=>element===document.querySelector('.media-viewer-stage .media-image img'))).toBe(true);else image=await viewer.locator('.media-viewer-stage .media-image img').elementHandle();await page.keyboard.press('Escape');await expect(viewer).not.toBeVisible();await expect(source).toBeFocused();}
    });
    await source.click();const viewer=page.getByRole('dialog',{name:'Media viewer',exact:true});await expect(viewer).toBeVisible();
    const stage=viewer.locator('.media-viewer-stage'),bounds=await stage.boundingBox();expect(bounds).not.toBeNull();await page.mouse.move(bounds!.x+bounds!.width/2,bounds!.y+bounds!.height/2);await page.mouse.wheel(0,-200);
    await expect.poll(()=>viewer.getAttribute('data-media-zoom').then(Number)).toBeGreaterThan(1);expect(await stage.evaluate(element=>element.getAnimations().filter(animation=>animation.playState==='running').length)).toBe(0);
    await page.keyboard.press('Escape');await expect(viewer).not.toBeVisible();await expect(source).toBeFocused();
});

test('WEB-MSG-PERF performance observers leave layout coalesced until the frame',async({page})=>{
    await page.goto('about:blank');await installProbe(page,false,true);
    const result=await page.evaluate(async()=>{
        const probe=(window as any).__listProbe;probe.phase='scroll';
        let layouts=0,mutations=0;const rect=document.body.getBoundingClientRect.bind(document.body);
        document.body.getBoundingClientRect=()=>{layouts++;return rect();};
        const observer=new MutationObserver(()=>{observer.disconnect();mutations++;document.body.style.padding='2px';});
        observer.observe(document.body,{attributes:true});document.body.style.padding='1px';
        await Promise.resolve();await Promise.resolve();const beforeFrame=layouts;
        await new Promise<void>(resolve=>requestAnimationFrame(()=>resolve()));
        return {beforeFrame,layouts,mutations,frames:probe.frames.length};
    });
    expect(result.mutations).toBe(1);expect(result.beforeFrame).toBe(0);expect(result.layouts).toBeGreaterThan(0);expect(result.frames).toBeGreaterThanOrEqual(1);
});
