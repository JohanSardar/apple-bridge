import {expect,type Page,type TestInfo} from '@playwright/test';
import {mkdir,writeFile} from 'node:fs/promises';
import {installProbe,startTrace,stopTrace,traceWork,stats} from './list-performance';
export async function transcriptObservers(page:Page){
    await page.addInitScript(()=>{
        function measure(kind:string,callback:()=>void){
            const probe=(window as any).__listProbe;if(probe?.phase!=='scroll'){callback();return;}
            const start=performance.now();callback();const styledAt=performance.now();document.body.getBoundingClientRect();const end=performance.now(),last=probe.frames.at(-1);
            if(last)last.work+=end-start;else probe.frames.push({at:start,work:end-start});
            (probe.observers??=[]).push({kind,script:styledAt-start,style_layout:end-styledAt});
        }
        const Resize=window.ResizeObserver,Mutation=window.MutationObserver;(window as any).__transcriptNativeMutation=Mutation;
        window.ResizeObserver=class extends Resize {constructor(callback:ResizeObserverCallback){super((entries,observer)=>measure('resize',()=>callback(entries,observer)));}};
        window.MutationObserver=class extends Mutation {constructor(callback:MutationCallback){super((entries,observer)=>measure('mutation',()=>callback(entries,observer)));}};
    });
}
export async function transcriptInputs(page:Page,info:TestInfo,label:string,actions:()=>Promise<void>){
    await installProbe(page,false);
    await page.evaluate(({label,stageProfile})=>{
        const probe=(window as any).__listProbe;if(stageProfile)(window as any).__abWork={phase:label,samples:[]};probe.actions=[];probe.actionFrames=0;probe.phaseDone=false;
        const native=(window as any).__listNativeRAF as typeof requestAnimationFrame;
        const reactions=()=>Array.from(document.querySelectorAll('.messages-transcript .reaction-badge.own-reaction')).map(element=>element.closest('[data-message]')?.getAttribute('data-message')+':'+element.getAttribute('data-reaction-key')).join('|');
        const capture=(at?:number)=>{
            const action=probe.actions.at(-1);if(!action||action.resultAt)return;
            const transcript=document.querySelector('.messages-transcript');
            const ready=label==='open-chat'?transcript&&transcript!==action.previousTranscript&&transcript.getAttribute('aria-busy')==='false'&&transcript.querySelector('.message-row'):
                label==='send'?Array.from(document.querySelectorAll('.message-row')).some(row=>row.textContent?.includes(action.text)):
                label==='react'?reactions()!==action.reactions:document.querySelector('[aria-label="Media viewer"][data-media-loading="false"]');
            if(!ready)return;
            action.resultAt=performance.now();action.resultFrame=at??0;if(at===undefined)native(frame=>action.resultFrame=frame);delete action.previousTranscript;performance.mark('list-input-result-'+action.index);
        };probe.captureResult=capture;
        const observer=new ((window as any).__transcriptNativeMutation as typeof MutationObserver)(()=>capture());observer.observe(document.body,{subtree:true,childList:true,attributes:true,characterData:true});
        probe.stopResults=()=>{observer.disconnect();delete probe.captureResult;};
        window.addEventListener('click',event=>{
            if(!event.isTrusted||!(event.target as Element).closest('.conversation-row,.composer-send,.tapback-picker button,.media-open'))return;
            const index=probe.actions.length,action={index,at:event.timeStamp,captured:performance.now(),processed:0,frame:0,resultAt:0,resultFrame:0,previousTranscript:document.querySelector('.messages-transcript'),reactions:reactions(),text:(document.querySelector('[aria-label=Message]') as HTMLTextAreaElement)?.value};probe.actions.push(action);performance.mark('list-input-event-'+index);
            native(at=>{action.frame=at;performance.mark('list-input-frame-start-'+index);native(()=>{performance.mark('list-input-frame-end-'+index);probe.actionFrames++;});});
        },true);
        window.addEventListener('click',event=>{if(event.isTrusted&&(event.target as Element).closest('.conversation-row,.composer-send,.tapback-picker button,.media-open'))probe.actions.at(-1).processed=performance.now();});
        probe.phase='scroll';probe.scrollStart=performance.now();
    },{label,stageProfile:Boolean(process.env.AB_WEB_STAGE_PROFILE||process.env.AB_WEB_CPU_PROFILE)});
    const session=await startTrace(page),profileCPU=Boolean(process.env.AB_WEB_CPU_PROFILE&&session);
    if(profileCPU){await session!.send('Profiler.enable');await session!.send('Profiler.start');}
    const sampling=page.evaluate(async()=>{
        const probe=(window as any).__listProbe;
        for(let index=0;index<1801;index++){await new Promise<void>(resolve=>requestAnimationFrame(()=>{performance.mark('list-frame-'+index);resolve();}));if(index>=120&&probe.phaseDone)break;}
        probe.samplingCovered=probe.phaseDone;probe.phase='idle';probe.scrollEnd=performance.now();
    });
    let failure:unknown;try{await actions();await expect.poll(()=>page.evaluate(()=>(window as any).__listProbe.actionFrames)).toBe(24);await expect.poll(()=>page.evaluate(()=>(window as any).__listProbe.actions.filter((action:any)=>action.resultAt).length)).toBe(24);}catch(error){failure=error;}
    await page.evaluate(()=>{const probe=(window as any).__listProbe;probe.phaseDone=true;probe.stopResults();delete probe.stopResults;});await sampling;
    const cpu=profileCPU?await session!.send('Profiler.stop'):undefined;
    const trace=await stopTrace(session),probe=await page.evaluate(()=>(window as any).__listProbe),input=trace&&probe.actions.length>=20?traceWork(trace.trace,'action'):undefined;
    const result=input?resultFrames(trace!.trace):undefined;
    const samples=probe.actions.map((action:any,index:number)=>{
        const event=probe.events.find((entry:any)=>entry.name==='click'&&Math.abs(entry.startTime-action.at)<2);
        const processing=event?event.processingEnd-event.startTime:input?Math.max(0,action.captured-action.at)+input.dispatch_processing_ms[index]:action.processed-action.at;
        const recorded=probe.frames.filter((frame:any)=>frame.at===action.resultFrame);
        if(!result&&!recorded.length)throw new Error('Rendered input frame work missing');
        const frame=result?.frames[index]??recorded.reduce((sum:number,frame:any)=>sum+frame.work,0);
        return {processing_ms:processing,result_frame_ms:frame,total_ms:processing+frame,event_timing:Boolean(event)};
    });
    const frameBudget=label==='open-chat'&&info.project.name.endsWith('-phone');
    const work=await page.evaluate(()=>(window as any).__abWork?.samples??[]);
    const {trace:raw,...chromium}=trace??{},metrics={frame_budget_applies:frameBudget,playwright_recording:false,cpu_sampling:profileCPU,frame_work:trace?.frame_work??stats(probe.frames.map((frame:any)=>frame.work)),input_responsiveness:stats(samples.map((sample:any)=>sample.total_ms)),samples,product_work:work,sampling_covered:probe.samplingCovered,chromium:trace?chromium:null,observer_work:probe.observers??[],long_tasks:probe.longTasks,frame_intervals:stats(probe.frames.slice(1).map((frame:any,index:number)=>frame.at-probe.frames[index].at)),endpoint:'trusted input delay and dispatch processing plus script/style/layout in the actual result frame, excluding the already-counted dispatch'};
    await mkdir('../lab/evidence/web/metrics',{recursive:true});await mkdir('../lab/evidence/private/web',{recursive:true});await writeFile(`../lab/evidence/web/metrics/WEB-MSG-PERF-${label}-${info.project.name}.json`,JSON.stringify(metrics,null,2)+'\n');if(raw)await writeFile(`../lab/evidence/private/web/transcript-${label}-${info.project.name}.json`,raw);if(cpu)await writeFile(`../lab/evidence/private/web/transcript-input-cpu-${label}-${info.project.name}.json`,JSON.stringify(cpu));
    if(failure)throw failure;expect(metrics.sampling_covered).toBe(true);expect(metrics.frame_work.samples).toBeGreaterThanOrEqual(100);expect(samples).toHaveLength(24);expect(input?.frames.length??probe.actions.length).toBe(24);expect(samples.every((sample:any)=>Number.isFinite(sample.processing_ms)&&sample.processing_ms>=0&&Number.isFinite(sample.result_frame_ms)&&sample.result_frame_ms>=0)).toBe(true);expect(metrics.input_responsiveness.p95_ms).toBeLessThanOrEqual(8);if(frameBudget){expect(metrics.frame_work.p95_ms).toBeLessThanOrEqual(4);expect(metrics.frame_work.max_ms).toBeLessThanOrEqual(8);expect(metrics.long_tasks).toEqual([]);}
}

function resultFrames(raw:string){
    const parsed=JSON.parse(raw),events=parsed.traceEvents as {name:string;ts:number;pid:number;tid:number;ph:string}[];
    const frames=events.filter(event=>event.name==='AnimationFrame'&&event.ph==='b').sort((a,b)=>a.ts-b.ts);
    const ends=events.filter(event=>event.name==='AnimationFrame'&&event.ph==='e').sort((a,b)=>a.ts-b.ts);
    const results=events.filter(event=>/^list-input-result-\d+$/.test(event.name)).sort((a,b)=>Number(a.name.slice('list-input-result-'.length))-Number(b.name.slice('list-input-result-'.length)));
    const overlaps:number[]=[];
    for(const result of results){
        const index=result.name.slice('list-input-result-'.length);
        const frame=frames.findLast(frame=>frame.ts<=result.ts&&frame.pid===result.pid&&frame.tid===result.tid),finish=frame&&ends.find(end=>end.ts>=result.ts&&end.ts>frame.ts&&end.pid===frame.pid&&end.tid===frame.tid);
        if(!frame||!finish)throw new Error('Rendered input result frame missing');
        const begin=events.find(event=>event.name==='list-input-frame-start-'+index),end=events.find(event=>event.name==='list-input-frame-end-'+index);
        if(!begin||!end)throw new Error('Input result endpoints missing');
        begin.ts=frame.ts;end.ts=finish.ts;
        const captured=events.find(event=>event.name==='list-input-event-'+index)!;
        const dispatch=parsed.traceEvents.filter((event:any)=>event.pid===captured.pid&&event.tid===captured.tid&&event.name==='EventDispatch'&&event.ph==='X'&&event.ts<=captured.ts&&event.ts+event.dur>=captured.ts).sort((a:any,b:any)=>a.dur-b.dur)[0];
        if(!dispatch)throw new Error('Rendered input dispatch missing');
        overlaps.push(Math.max(0,Math.min(end.ts,dispatch.ts+dispatch.dur)-Math.max(begin.ts,dispatch.ts))/1000);
    }
    if(results.length!==24)throw new Error('Rendered input result marks missing');
    const measured=traceWork(JSON.stringify(parsed),'input');
    return {frames:measured.frames.map((work,index)=>Math.max(0,work-overlaps[index]))};
}
