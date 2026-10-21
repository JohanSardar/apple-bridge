let inputDepth = 0;
const inputEvents = new Set(['click','pointerup','keydown','keyup','touchend']);
const activeInputs = new WeakSet<Event>();
const nativeEventPhase = Object.getOwnPropertyDescriptor(Event.prototype,'eventPhase')!.get!;
for(const type of inputEvents)window.addEventListener(type,event=>{
    if(!event.isTrusted)return;
    activeInputs.add(event);
},{capture:true});
export function withInput<T>(event:Event,action:()=>T):T {
    if(!(event instanceof Event)||!activeInputs.has(event)||!event.isTrusted||nativeEventPhase.call(event)===Event.NONE||!inputEvents.has(event.type))throw new Error('Trusted input required');
    inputDepth++;
    try{return action();}finally{inputDepth--;}
}
export function requireInput(){if(inputDepth===0)throw new Error('Trusted input required');}
