import {matchingFonts} from './fonts';
type Variant={weight?:number;italic?:boolean};
const properties=['font-family','font-size','font-weight','font-style','letter-spacing','font-feature-settings','font-variation-settings','font-kerning','font-variant','font-stretch','word-spacing','text-rendering'] as const;
const measurements=new Map<string,Map<string,number>>();
let textContext:CanvasRenderingContext2D|undefined;
export function createTextMeasurer(){
    const context=textContext??=document.createElement('canvas').getContext('2d')!;
    let cache=new Map<string,number>();
    const pending=new Map<string,{value:string;variant?:Variant}>();
    let descriptor='',typography:Record<(typeof properties)[number],string>,advanced=false,host:HTMLElement;
    function prepare(style:CSSStyleDeclaration,measurementHost:HTMLElement){
        typography=Object.fromEntries(properties.map(property=>[property,style.getPropertyValue(property)])) as Record<(typeof properties)[number],string>;host=measurementHost;
        const share=matchingFonts(typography['font-family']).length===0,next=Object.values(typography).join('|')+':'+share;
        if(next!==descriptor){
            descriptor=next;pending.clear();
            if(share){
                cache=measurements.get(descriptor)!;
                if(!cache){cache=new Map();if(measurements.size>=8)measurements.delete(measurements.keys().next().value!);measurements.set(descriptor,cache);}
            }else cache=new Map();
        }
        advanced=typography['font-feature-settings']!=='normal'||typography['font-variation-settings']!=='normal'||typography['font-variant']!=='normal'||!['normal','100%'].includes(typography['font-stretch'])||!['normal','0px'].includes(typography['word-spacing']);
        context.letterSpacing=typography['letter-spacing']==='normal'?'0px':typography['letter-spacing'];
        context.wordSpacing=typography['word-spacing']==='normal'?'0px':typography['word-spacing'];
        context.fontKerning=typography['font-kerning'] as CanvasFontKerning;
        return descriptor;
    }
    function measure(value:string,variant?:Variant){
        const prefix=(variant?.weight??typography['font-weight'])+':'+Boolean(variant?.italic)+':',key=prefix+value;
        const saved=cache.get(key);if(saved!==undefined)return saved;
        if(cache.size>=10000)cache.clear();
        context.font=`${variant?.italic?'italic':typography['font-style']} ${variant?.weight??typography['font-weight']} ${typography['font-size']} ${typography['font-family']}`;
        const width=context.measureText(value).width;
        if(advanced){pending.set(key,{value,variant});for(const character of value){const characterKey=prefix+character;if(!cache.has(characterKey)&&!pending.has(characterKey))pending.set(characterKey,{value:character,variant});}}else cache.set(key,width);
        return width;
    }
    function flush(){
        if(!pending.size)return false;
        const fragment=document.createDocumentFragment(),nodes:{key:string;element:HTMLSpanElement}[]=[];
        for(const [key,{value,variant}]of pending){
            const element=document.createElement('span');for(const property of properties)element.style.setProperty(property,typography[property]);
            if(variant?.weight)element.style.fontWeight=String(variant.weight);if(variant?.italic)element.style.fontStyle='italic';
            element.style.display='block';element.style.width='max-content';element.style.whiteSpace='pre';element.textContent=value;fragment.append(element);nodes.push({key,element});
        }
        host.append(fragment);
        for(const {key,element}of nodes)cache.set(key,element.getBoundingClientRect().width);
        host.replaceChildren();pending.clear();return true;
    }
    function clear(){cache.clear();pending.clear();}
    return {prepare,measure,flush,clear};
}
