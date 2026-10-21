import {test,expect,type Page} from '@playwright/test';
import {mkdir,writeFile} from 'node:fs/promises';
import {prepareSession,control} from './session';
test.use({deviceScaleFactor:2});
async function marker(page:Page,png:Uint8Array,width:number,height:number){
 return page.evaluate(async({bytes,width,height})=>{
  const image=await createImageBitmap(new Blob([new Uint8Array(bytes)],{type:'image/png'}));
  const canvas=document.createElement('canvas');canvas.width=image.width;canvas.height=image.height;
  const context=canvas.getContext('2d')!;context.drawImage(image,0,0);image.close();
  const data=context.getImageData(0,0,canvas.width,canvas.height).data;
  let left=canvas.width,top=canvas.height,right=-1,bottom=-1,count=0;
  for(let y=0;y<canvas.height;y++)for(let x=0;x<canvas.width;x++){
   const index=(y*canvas.width+x)*4,r=data[index],g=data[index+1],b=data[index+2];
   if(r>100&&r>g*1.8&&r>b*1.8){left=Math.min(left,x);right=Math.max(right,x);top=Math.min(top,y);bottom=Math.max(bottom,y);count++;}
  }
  const scaleX=canvas.width/width,scaleY=canvas.height/height,edge=(Math.floor(canvas.height/2)*canvas.width+Math.min(canvas.width-1,Math.round(8*scaleX)))*4;
  return{width:(right-left+1)/scaleX,height:(bottom-top+1)/scaleY,centerX:(left+right+1)/2/scaleX,centerY:(top+bottom+1)/2/scaleY,count,edge:[data[edge],data[edge+1],data[edge+2]],pixels:{width:canvas.width,height:canvas.height}};
 },{bytes:Array.from(png),width,height});
}
test('WEB-MSG-MEDIA-MOTION real transcript covers an ultrawide poster and playing frame without distortion',async({page,request},info)=>{
 await prepareSession(page);const ids:Record<string,string>=await control(request,'/demo-fixtures');
 await page.mouse.move(0,0);await page.goto('/messages/'+encodeURIComponent(ids['message-chat'])+'/'+encodeURIComponent(ids['demo-message-00-000130']));
 const row=page.locator('[data-message="'+ids['demo-message-00-000130']+'"]'),slot=row.locator('.media-reserved'),motion=slot.locator('.media-motion'),button=motion.locator('button.media-open');
 await expect(motion).toBeVisible();await expect(page.locator('.messages-transcript')).toHaveAttribute('aria-busy','false');
 const image=motion.locator('img').first();await expect.poll(()=>image.evaluate((node:HTMLImageElement)=>node.complete&&node.naturalWidth>0)).toBe(true);
 await expect(page.locator('.transcript')).toHaveCSS('transform','none');await expect(page.locator('.transcript')).toHaveCSS('opacity','1');
 await button.focus();await page.evaluate(()=>new Promise<void>(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>resolve()))));const bounds=(await slot.boundingBox())!;
 expect(bounds.width).toBeGreaterThanOrEqual(120);expect(bounds.height).toBeGreaterThanOrEqual(120);
 const phone=info.project.name.endsWith('-phone');expect(bounds.width).toBeLessThanOrEqual(phone?273:480);
 const natural=await image.evaluate((node:HTMLImageElement)=>({width:node.naturalWidth,height:node.naturalHeight}));expect(natural.width/natural.height).toBeCloseTo(1920/200,2);
 await mkdir('../lab/evidence/private/web',{recursive:true});const evidence:Record<string,unknown>={bounds,natural,expectedMarker:160*Math.max(bounds.width/1920,bounds.height/200)};
 async function capture(kind:string){
  const png=await slot.screenshot({animations:'allow'});await writeFile(`../lab/evidence/private/web/transcript-video-cover-${kind}-${info.project.name}.png`,png);
  const measured=await marker(page,png,bounds.width,bounds.height),expected=160*Math.max(bounds.width/1920,bounds.height/200);evidence[kind]=measured;
  expect(measured.count).toBeGreaterThan(100);expect(Math.abs(measured.width-expected)).toBeLessThanOrEqual(3);expect(Math.abs(measured.height-expected)).toBeLessThanOrEqual(3);
  expect(Math.abs(measured.width/measured.height-1)).toBeLessThan(.05);expect(Math.abs(measured.centerX-bounds.width/2)).toBeLessThanOrEqual(2);expect(Math.abs(measured.centerY-bounds.height/2)).toBeLessThanOrEqual(2);
  expect(measured.edge[2]).toBeGreaterThan(80);expect(measured.edge[2]).toBeGreaterThan(measured.edge[0]*1.5);expect(await slot.boundingBox()).toEqual(bounds);
 }
 await capture('poster');await page.keyboard.down('Space');
 try{
  const video=motion.locator('video');await expect(motion).toHaveAttribute('data-media-frame','decoded');await expect(video).toHaveCSS('opacity','1');
  await video.evaluate((node:HTMLVideoElement)=>{node.loop=true;});await expect.poll(()=>video.evaluate((node:HTMLVideoElement)=>node.currentTime)).toBeGreaterThan(.05);
  const decoded=await video.evaluate((node:HTMLVideoElement)=>({width:node.videoWidth,height:node.videoHeight,playing:!node.paused}));expect(decoded).toEqual({width:1920,height:200,playing:true});evidence.decoded=decoded;await capture('playing');
 }finally{await page.keyboard.up('Space');}
 await writeFile(`../lab/evidence/private/web/transcript-video-cover-${info.project.name}.json`,JSON.stringify(evidence,null,2)+'\n');
});
