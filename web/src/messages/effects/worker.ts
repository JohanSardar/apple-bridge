import {EffectScene,type SceneOptions} from './scene';
type Start={canvas:OffscreenCanvas;options:SceneOptions;scale:number};
type Tick={time:number};
const scope=globalThis as unknown as {onmessage:(event:MessageEvent<Start|Tick>)=>void;postMessage:(message:unknown)=>void};
let scene:EffectScene|undefined,started:number|undefined;
const frames:number[]=[];
scope.onmessage=event=>{
 if('canvas' in event.data){const {canvas,options,scale}=event.data,context=canvas.getContext('2d');if(!context){scope.postMessage({type:'error'});return;}context.setTransform(scale,0,0,scale,0,0);scene=new EffectScene(context,options);scope.postMessage({type:'tick'});return;}
 if(!scene)return;const {time}=event.data;started??=time;const before=performance.now(),alive=scene.draw(time-started);frames.push(performance.now()-before);
 if(alive)scope.postMessage({type:'tick',ready:time-started>=60});else {const sorted=frames.sort((a,b)=>a-b);scope.postMessage({type:'done',frames:frames.length,p95:sorted[Math.max(0,Math.ceil(sorted.length*.95)-1)],max:sorted.at(-1)});}
};
