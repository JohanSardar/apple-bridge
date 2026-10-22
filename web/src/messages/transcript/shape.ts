export function bubbleSilhouette(width:number,height:number,origin:{x?:number;y?:number;mirror?:boolean;tail?:boolean;radius?:number}={}){
    const radius=Math.min(origin.radius??18,width/2,height/2),x=(value:number)=>(origin.x??0)+(origin.mirror?width-value:value),y=(value:number)=>(origin.y??0)+value;
    const corner=origin.tail===false?`V${y(height-radius)}Q${x(width)} ${y(height)} ${x(width-radius)} ${y(height)}`:`V${y(height-12)}C${x(width)} ${y(height-5)} ${x(width+1)} ${y(height-2)} ${x(width+5)} ${y(height)}C${x(width-1)} ${y(height)} ${x(width-6)} ${y(height-1)} ${x(width-9)} ${y(height-4)}Q${x(width-12)} ${y(height)} ${x(width-radius)} ${y(height)}`;
    return `M${x(radius)} ${y(0)}H${x(width-radius)}Q${x(width)} ${y(0)} ${x(width)} ${y(radius)}${corner}H${x(radius)}Q${x(0)} ${y(height)} ${x(0)} ${y(height-radius)}V${y(radius)}Q${x(0)} ${y(0)} ${x(radius)} ${y(0)}Z`;
}
