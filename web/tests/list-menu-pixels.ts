import type { Locator, Page } from '@playwright/test';
let stylesheetID=0;
export async function menuStyle(page:Page,content:string){
    const url=new URL('/__list-menu-test-'+(stylesheetID++)+'.css',page.url()).href;
    await page.route(url,route=>route.fulfill({contentType:'text/css',body:content}));
    try{return await page.addStyleTag({url});}finally{await page.unroute(url);}
}
export async function menuPixels(page:Page,menu:Locator){
    const box=(await menu.boundingBox())!,clip={x:Math.floor(box.x),y:Math.floor(box.y),width:Math.ceil(box.x+box.width)-Math.floor(box.x),height:Math.ceil(box.y+box.height)-Math.floor(box.y)};
    const regions=await menu.getByRole('menuitemradio').evaluateAll(buttons=>buttons.flatMap(button=>{
        const text=[...button.childNodes].find(node=>node.nodeType===Node.TEXT_NODE);if(!text)return [];
        const range=document.createRange();range.selectNodeContents(text);const style=getComputedStyle(button);
        return [...range.getClientRects()].map(rect=>({x:rect.x,y:rect.y,width:rect.width,height:rect.height,color:style.color}));
    }));
    const surface=await menu.evaluate(element=>getComputedStyle(element).backgroundColor);
    const mask=await menuStyle(page,'.conversation-filter-menu button{color:transparent!important}');
    let png:Buffer;
    try{png=await page.screenshot({clip,scale:'css'});}finally{await mask.evaluate(element=>element.remove());}
    return page.evaluate(async({encoded,regions,clip,surface})=>{
        const image=new Image();image.src='data:image/png;base64,'+encoded;await image.decode();
        const canvas=document.createElement('canvas');canvas.width=image.width;canvas.height=image.height;const context=canvas.getContext('2d')!;context.drawImage(image,0,0);const pixels=context.getImageData(0,0,image.width,image.height).data;
        const swatch=document.createElement('canvas');swatch.width=swatch.height=1;const color=swatch.getContext('2d')!;
        const rgba=(value:string)=>{color.clearRect(0,0,1,1);color.fillStyle=value;color.fillRect(0,0,1,1);return [...color.getImageData(0,0,1,1).data];};
        const luminance=(rgb:number[])=>rgb.slice(0,3).map(value=>{const channel=value/255;return channel<=.04045?channel/12.92:((channel+.055)/1.055)**2.4;}).reduce((sum,value,index)=>sum+value*[.2126,.7152,.0722][index],0);
        let minimum=Infinity,count=0;const backgrounds:number[][]=[];
        for(const region of regions){
            const foreground=rgba(region.color),total=[0,0,0];let samples=0;
            for(let y=Math.max(0,Math.ceil(region.y-clip.y));y<Math.min(image.height,Math.floor(region.y+region.height-clip.y));y++)for(let x=Math.max(0,Math.ceil(region.x-clip.x));x<Math.min(image.width,Math.floor(region.x+region.width-clip.x));x++){
                const offset=(y*image.width+x)*4,background=[pixels[offset],pixels[offset+1],pixels[offset+2]],ink=background.map((value,index)=>foreground[index]*foreground[3]/255+value*(1-foreground[3]/255));
                const first=luminance(ink),second=luminance(background);minimum=Math.min(minimum,(Math.max(first,second)+.05)/(Math.min(first,second)+.05));
                background.forEach((value,index)=>total[index]+=value);samples++;count++;
            }
            backgrounds.push(total.map(value=>value/samples));
        }
        return {contrast_min:minimum,samples:count,regions:regions.length,backgrounds,fill_alpha:rgba(surface)[3]/255};
    },{encoded:png!.toString('base64'),regions,clip,surface});
}
