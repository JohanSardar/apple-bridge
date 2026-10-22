import { expect, type Page } from '@playwright/test';
import { writeFile } from 'node:fs/promises';
import { startTrace, stopTrace, stats, traceWork } from './list-performance';
export async function installActionProbe(page:Page){
    await page.evaluate(()=>{
        const capture=(event:Event)=>{
            const target=event.target as Element,filter=target.closest('[role=menuitemradio]'),search=target instanceof HTMLInputElement&&target.closest('.conversation-search');
            if(!event.isTrusted||!(event.type==='click'&&filter||event.type==='keydown'&&search))return;
            const key=(event as KeyboardEvent).key;
            if(search&&key!=='Backspace'&&key.length!==1)return;
            const probe=(window as any).__listProbe,index=probe.actions.length,input=target as HTMLInputElement,expected=filter?.textContent??(key==='Backspace'?input.value.slice(0,-1):input.value+key);
            const action={name:event.type,startTime:event.timeStamp,expected,rendered:false,event_to_next_frame_ms:0};probe.actions.push(action);performance.mark('list-input-event-'+index);const start=performance.now();
            requestAnimationFrame(()=>{performance.mark('list-input-frame-start-'+index);action.event_to_next_frame_ms=performance.now()-start;action.rendered_count=filter?Number(document.querySelector('[role=listbox][aria-label=Conversations] [aria-setsize]')?.getAttribute('aria-setsize')):undefined;action.expected_count=filter?probe.filterExpectedCounts?.[expected]:undefined;action.rendered=filter?document.querySelector<HTMLElement>('.conversation-list')?.dataset.filter===expected&&action.rendered_count===action.expected_count&&(expected!=='Unread'||Array.from(document.querySelectorAll('.conversation-row')).every(row=>row.querySelector('.unread-dot'))):input.value===expected;requestAnimationFrame(()=>{performance.mark('list-input-frame-end-'+index);probe.actionFrames++;});});
        };
        window.addEventListener('click',capture,true);window.addEventListener('keydown',capture,true);
    });
}
export async function observedActions(page:Page){
    return page.evaluate(()=>{const p=(window as any).__listProbe;return p.actions.filter((action:any)=>p.events.some((event:any)=>event.name===action.name&&Math.abs(event.startTime-action.startTime)<2)).length;});
}
export async function measureActions(page:Page,label:string,project:string,actions:()=>Promise<void>,input:boolean,context?:unknown){
    await page.evaluate(()=>{const p=(window as any).__listProbe;p.phaseDone=false;p.phase='scroll';p.frames=[];p.events=[];p.longTasks=[];p.actions=[];p.actionFrames=0;p.samplingCovered=false;p.scrollStart=performance.now();p.scrollEnd=0;});
    const session=await startTrace(page);
    const sampling=page.evaluate(async()=>{const p=(window as any).__listProbe;for(let index=0;index<1201;index++){await new Promise<void>(resolve=>requestAnimationFrame(()=>{performance.mark('list-frame-'+index);resolve();}));if(index>=120&&p.phaseDone)break;}p.samplingCovered=p.phaseDone;p.phase='idle';p.scrollEnd=performance.now();});
    await page.evaluate(()=>new Promise<void>(resolve=>requestAnimationFrame(()=>resolve())));
    let failure:unknown;
    try{await actions();if(input){await expect.poll(()=>observedActions(page)).toBeGreaterThanOrEqual(20);await expect.poll(()=>page.evaluate(()=>{const p=(window as any).__listProbe;return p.actionFrames===p.actions.length;})).toBe(true);}}catch(error){failure=error;}
    await page.evaluate(()=>(window as any).__listProbe.phaseDone=true);await sampling;
    const trace=(await stopTrace(session))!,probe=await page.evaluate(()=>(window as any).__listProbe);
    await writeFile(`../lab/evidence/private/web/list-${label}-trace-${project}.json`,trace.trace);
    const actionTrace=input&&probe.actions.length>=20?traceWork(trace.trace,'action'):undefined,frames=actionTrace?.frames??[];
    const samples=probe.actions.flatMap((action:any,index:number)=>{const event=probe.events.find((entry:any)=>entry.name===action.name&&Math.abs(entry.startTime-action.startTime)<2);return event&&Number.isFinite(frames[index])?[{delay_and_processing_ms:event.processingEnd-event.startTime,event_processing_ms:event.processing,trace_dispatch_processing_ms:actionTrace!.dispatch_processing_ms[index],frame_work_ms:frames[index],total_ms:event.processingEnd-event.startTime+frames[index]}]:[];});
    const first=probe.actions[0],firstEvent=first?probe.events.find((entry:any)=>entry.name===first.name&&Math.abs(entry.startTime-first.startTime)<2):undefined;
    const metrics={context,first_input:first?{action:first,event_timing:firstEvent??null,trace_dispatch_processing_ms:actionTrace?.dispatch_processing_ms[0]??null,frame_work_ms:frames[0]??null}:null,frame_work:trace.frame_work,sampling_covered:probe.samplingCovered,input_frame_endpoint:actionTrace?.endpoint??null,input_responsiveness_ms:input&&samples.length?stats(samples.map((sample:any)=>sample.total_ms)):null,input_responsiveness_samples:samples,input_attempts:probe.actions.length,event_timing_observed:samples.length,event_timing_unobserved:probe.actions.length-samples.length,event_timing:probe.events,event_timing_threshold_ms:16,input_8ms_proven:false,event_to_next_frame_ms:input&&probe.actions.length?stats(probe.actions.map((action:any)=>action.event_to_next_frame_ms)):null,long_tasks:probe.longTasks};
    await writeFile(`../lab/evidence/web/metrics/WEB-PERF-LIST-${label}-${project}.json`,JSON.stringify(metrics,null,2)+'\n');
    if(failure)throw failure;
    expect(metrics.sampling_covered).toBe(true);expect(metrics.frame_work.p95_ms).toBeLessThanOrEqual(4);expect(metrics.frame_work.max_ms).toBeLessThanOrEqual(8);expect(metrics.long_tasks).toEqual([]);
    if(input){expect(samples.length).toBeGreaterThanOrEqual(20);expect(probe.actions.every((action:any)=>action.rendered)).toBe(true);expect(metrics.input_responsiveness_ms!.p95_ms).toBeLessThanOrEqual(8);}
    return metrics;
}
