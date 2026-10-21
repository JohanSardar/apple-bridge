let inputDepth=0;
const allowed=new Set(['click','pointerup','keydown','keyup','input','change','submit','touchend']);
export function withInput<T>(event:Event, action:()=>T):T {
  if(!event.isTrusted||event.eventPhase===Event.NONE||!allowed.has(event.type))throw new Error('Trusted input required');
  inputDepth++;
  try{return action();}finally{inputDepth--;}
}
export function requireInput(){if(inputDepth===0)throw new Error('Trusted input required');}
