type Variant={weight?:number;italic?:boolean};
const properties=['font-family','font-size','font-weight','font-style','letter-spacing','font-feature-settings','font-variation-settings','font-kerning','font-variant','font-stretch','word-spacing','text-rendering'] as const;
export function createTextMeasurer(){
    const context=document.createElement('canvas').getContext('2d')!,cache=new Map<string,number>(),pending=new Map<string,{value:string;variant?:Variant}>();
    let descriptor='',style:CSSStyleDeclaration,advanced=false,host:HTMLElement;
    function prepare(element:HTMLElement,measurementHost:HTMLElement){
        style=getComputedStyle(element);host=measurementHost;
        const next=properties.map(property=>style.getPropertyValue(property)).join('|');
        if(next!==descriptor){descriptor=next;cache.clear();pending.clear();}
        advanced=style.fontFeatureSettings!=='normal'||style.fontVariationSettings!=='normal'||style.fontVariant!=='normal'||!['normal','100%'].includes(style.fontStretch)||!['normal','0px'].includes(style.wordSpacing);
        context.letterSpacing=style.letterSpacing==='normal'?'0px':style.letterSpacing;
        context.wordSpacing=style.wordSpacing==='normal'?'0px':style.wordSpacing;
        context.fontKerning=style.fontKerning as CanvasFontKerning;
        return descriptor;
    }
    function measure(value:string,variant?:Variant){
        const prefix=(variant?.weight??style.fontWeight)+':'+Boolean(variant?.italic)+':',key=prefix+value;
        const saved=cache.get(key);if(saved!==undefined)return saved;
        if(cache.size>=10000)cache.clear();
        context.font=`${variant?.italic?'italic':style.fontStyle} ${variant?.weight??style.fontWeight} ${style.fontSize} ${style.fontFamily}`;
        const width=context.measureText(value).width;
        if(advanced){pending.set(key,{value,variant});for(const character of value){const characterKey=prefix+character;if(!cache.has(characterKey)&&!pending.has(characterKey))pending.set(characterKey,{value:character,variant});}}else cache.set(key,width);
        return width;
    }
    function flush(){
        if(!pending.size)return false;
        const fragment=document.createDocumentFragment(),nodes:{key:string;element:HTMLSpanElement}[]=[];
        for(const [key,{value,variant}]of pending){
            const element=document.createElement('span');for(const property of properties)element.style.setProperty(property,style.getPropertyValue(property));
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
