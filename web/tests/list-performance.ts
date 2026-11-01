import type { Page, CDPSession } from '@playwright/test';
export function stats(values:number[]) {
    const sorted=[...values].sort((a,b)=>a-b),at=(q:number)=>sorted[Math.min(sorted.length-1,Math.ceil(sorted.length*q)-1)]??0;
    return {samples:values.length,p50_ms:at(.5),p95_ms:at(.95),max_ms:at(1)};
}
export async function installProbe(page:Page,selectionInputs=true,measureLayout=true) {
    await page.evaluate(({selectionInputs,measureLayout})=>{
        const probe={phase:'idle',scrollStart:0,scrollEnd:0,frames:[] as {at:number;work:number}[],input:[] as number[],inputFrames:0,inputNext:0,inputRendered:[] as boolean[],events:[] as {name:string;duration:number;startTime:number;processingEnd:number;processing:number;interaction:number}[],longTasks:[] as number[]};
        (window as any).__listProbe=probe;
        const native=window.requestAnimationFrame.bind(window);(window as any).__listNativeRAF=native;
        window.requestAnimationFrame=callback=>native(at=>{
            if(probe.phase==='scroll'){
                const start=performance.now();callback(at);(probe as any).captureResult?.(at);if(measureLayout)document.body.getBoundingClientRect();
                const last=probe.frames.at(-1);if(last?.at===at)last.work+=performance.now()-start;else probe.frames.push({at,work:performance.now()-start});
            }else callback(at);
        });
        window.addEventListener('click',event=>{if(selectionInputs&&event.isTrusted&&(event.target as Element).closest('.conversation-row')){const start=performance.now(),index=probe.inputNext++,row=(event.target as Element).closest('.conversation-row')!;performance.mark('list-input-event-'+index);native(()=>{performance.mark('list-input-frame-start-'+index);probe.input.push(performance.now()-start);probe.inputRendered.push(row.closest('[role=option]')?.getAttribute('aria-selected')==='true');native(()=>{performance.mark('list-input-frame-end-'+index);probe.inputFrames++;});});}},{capture:true});
        if(PerformanceObserver.supportedEntryTypes.includes('event'))new PerformanceObserver(list=>{for(const item of list.getEntries()){const entry=item as PerformanceEventTiming;if(entry.interactionId)probe.events.push({name:entry.name,duration:entry.duration,startTime:entry.startTime,processingEnd:entry.processingEnd,processing:entry.processingEnd-entry.processingStart,interaction:entry.interactionId});}}).observe({type:'event',durationThreshold:0});
        if(PerformanceObserver.supportedEntryTypes.includes('longtask'))new PerformanceObserver(list=>{for(const entry of list.getEntries())if(entry.startTime>=probe.scrollStart&&(!probe.scrollEnd||entry.startTime<=probe.scrollEnd))probe.longTasks.push(entry.duration);}).observe({type:'longtask'});
    },{selectionInputs,measureLayout});
}
type TraceEvent={name:string;ts:number;dur?:number;pid:number;tid:number;ph:string;args?:any};
export async function startTrace(page:Page):Promise<CDPSession|undefined> {
    if(page.context().browser()?.browserType().name()!=='chromium')return;
    const session=await page.context().newCDPSession(page);
    await session.send('Tracing.start',{categories:process.env.AB_WEB_TRACE_CATEGORIES||'devtools.timeline,blink.user_timing',transferMode:'ReturnAsStream'});
    return session;
}
export async function stopTrace(session:CDPSession|undefined,phase:'scroll'|'input'|'action'='scroll',nativeFrames=false) {
    const raw=await finishTrace(session,nativeFrames);
    return raw===undefined?undefined:traceWork(raw,phase,nativeFrames);
}
export async function finishTrace(session:CDPSession|undefined,nativeFrames=false) {
    if(!session)return undefined;
    if(nativeFrames)await session.send('Runtime.evaluate',{expression:'new Promise(requestAnimationFrame)',awaitPromise:true});
    const ready=new Promise<string>(resolve=>session.once('Tracing.tracingComplete',event=>resolve(event.stream)));
    await session.send('Tracing.end');
    const handle=await ready;let raw='';
    while(true){const part=await session.send('IO.read',{handle});raw+=part.data;if(part.eof)break;}
    await session.send('IO.close',{handle});await session.detach();
    return raw;
}
export function traceWork(raw:string,phase:'scroll'|'input'|'action'='scroll',nativeFrames=false) {
    const events=(JSON.parse(raw) as {traceEvents:TraceEvent[]}).traceEvents;
    let marks=events.filter(event=>phase==='scroll'?/^list-frame-\d+$/.test(event.name):/^list-input-frame-start-\d+$/.test(event.name)).sort((a,b)=>a.ts-b.ts);
    if(marks.length<(phase==='scroll'?100:20))throw new Error('Chromium frame marks missing');
    if(nativeFrames&&phase==='scroll'){
        const first=marks[0],last=marks.at(-1)!;
        const completed=events.filter(event=>event.name==='AnimationFrame'&&event.ph==='e'&&event.pid===first.pid&&event.tid===first.tid).sort((a,b)=>a.ts-b.ts),finish=completed.find(event=>event.ts>=last.ts);
        if(!finish)throw new Error('Chromium closing frame endpoint missing');
        marks=[first,...completed.filter(event=>event.ts>first.ts&&event.ts<=finish.ts)];
        if(marks.length<101)throw new Error('Chromium completed frame endpoints missing');
    }
    const dispatches=phase==='action'?marks.map(mark=>{
        const input=events.find(event=>event.name===mark.name.replace('frame-start','event'));
        if(!input)throw new Error('Chromium input capture mark missing');
        const dispatch=events.filter(event=>event.pid===input.pid&&event.tid===input.tid&&event.name==='EventDispatch'&&event.ph==='X'&&event.dur&&event.ts<=input.ts&&event.ts+event.dur>=input.ts).sort((a,b)=>a.dur!-b.dur!)[0];
        if(!dispatch||dispatch.ts+dispatch.dur!>mark.ts)throw new Error('Chromium input dispatch endpoint missing');
        return dispatch;
    }):[];
    const ranges=phase==='scroll'?marks.slice(0,-1).map((mark,index)=>[mark,marks[index+1]]):marks.map((mark,index)=>[phase==='action'?{...mark,ts:dispatches[index].ts+dispatches[index].dur!}:mark,events.find(event=>event.name===mark.name.replace('-start-','-end-'))!]);
    if(ranges.some(range=>!range[1]))throw new Error('Chromium input frame endpoint missing');
    const first=ranges[0][0],last=ranges.at(-1)![1];
    const names=new Set(['FunctionCall','EvaluateScript','EventDispatch','RunMicrotasks','TimerFire','FireAnimationFrame','UpdateLayoutTree','Layout','PrePaint','Paint','Layerize','UpdateLayerTree','HitTest']);
    const spans=events.filter(event=>event.pid===first.pid&&event.tid===first.tid&&event.ph==='X'&&event.dur&&names.has(event.name)&&event.ts+event.dur!>first.ts&&event.ts<last.ts).map(event=>[event.ts,event.ts+event.dur!] as const).sort((a,b)=>a[0]-b[0]);
    const merged:number[][]=[];
    for(const [start,end] of spans){const previous=merged.at(-1);if(previous&&previous[1]>=start)previous[1]=Math.max(previous[1],end);else merged.push([start,end]);}
    const work=ranges.map(([begin,finish])=>merged.reduce((sum,[start,end])=>sum+Math.max(0,Math.min(end,finish.ts)-Math.max(start,begin.ts)),0)/1000);
    const category=(name:string)=>events.filter(event=>event.pid===first.pid&&event.tid===first.tid&&event.name===name&&event.ph==='X'&&event.ts>=first.ts&&event.ts<=last.ts).reduce((sum,event)=>sum+(event.dur??0),0)/1000;
    return {frame_work:stats(work),frames:work,dispatch_processing_ms:dispatches.map(event=>event.dur!/1000),script_style_layout_ms:{layout:category('Layout'),style:category('UpdateLayoutTree'),function_calls:category('FunctionCall'),prepaint:category('PrePaint'),paint:category('Paint'),layerize:category('Layerize')},endpoint:phase==='action'?'union of Chromium main-thread script/style/layout/paint after input dispatch through the result frame':nativeFrames?'union of Chromium main-thread script/style/layout/paint spans between completed native frames':'union of Chromium main-thread script/style/layout/paint spans between rAF marks',trace:raw};
}

export function resultFrames(raw:string){
    const parsed=JSON.parse(raw),events=parsed.traceEvents as {name:string;ts:number;pid:number;tid:number;ph:string}[];
    const ends=events.filter(event=>event.name==='AnimationFrame'&&event.ph==='e').sort((a,b)=>a.ts-b.ts);
    const results=events.filter(event=>/^list-input-result-\d+$/.test(event.name)).sort((a,b)=>Number(a.name.slice('list-input-result-'.length))-Number(b.name.slice('list-input-result-'.length)));
    const overlaps:number[]=[];
    for(const result of results){
        const index=result.name.slice('list-input-result-'.length);
        const finish=ends.find(end=>end.ts>=result.ts&&end.pid===result.pid&&end.tid===result.tid);
        if(!finish)throw new Error('Rendered input result frame missing');
        const previous=ends.findLast(end=>end.ts<finish.ts&&end.pid===result.pid&&end.tid===result.tid);
        const begin=events.find(event=>event.name==='list-input-frame-start-'+index),end=events.find(event=>event.name==='list-input-frame-end-'+index);
        if(!begin||!end)throw new Error('Input result endpoints missing');
        const captured=events.find(event=>event.name==='list-input-event-'+index)!;
        begin.ts=previous?.ts??captured.ts;end.ts=finish.ts;
        const dispatch=parsed.traceEvents.filter((event:any)=>event.pid===captured.pid&&event.tid===captured.tid&&event.name==='EventDispatch'&&event.ph==='X'&&event.ts<=captured.ts&&event.ts+event.dur>=captured.ts).sort((a:any,b:any)=>a.dur-b.dur)[0];
        if(!dispatch)throw new Error('Rendered input dispatch missing');
        overlaps.push(Math.max(0,Math.min(end.ts,dispatch.ts+dispatch.dur)-Math.max(begin.ts,dispatch.ts))/1000);
    }
    if(results.length!==24)throw new Error('Rendered input result marks missing');
    const measured=traceWork(JSON.stringify(parsed),'input');
    return {frames:measured.frames.map((work,index)=>Math.max(0,work-overlaps[index]))};
}
