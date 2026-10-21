type WorkProbe = { phase:string; samples:{phase:string;kind:string;script:number;layout:number;at:number}[] };
export function beginWork(kind:string) {
    const probe=(window as unknown as {__abWork?:WorkProbe}).__abWork;
    if(!probe)return ()=>{};
    const start=performance.now(),phase=probe.phase;
    return ()=>queueMicrotask(()=>{
        const script=performance.now()-start,layoutStart=performance.now();
        document.body.getBoundingClientRect();
        probe.samples.push({phase,kind,script,layout:performance.now()-layoutStart,at:start});
    });
}
