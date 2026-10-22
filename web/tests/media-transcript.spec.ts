import {test,expect,type Page,type TestInfo} from '@playwright/test';
import {control} from './session';
import {mediaSession} from './media-session';
const native=(index:number)=>'demo-message-00-'+String(index).padStart(6,'0');
async function jump(page:Page,ids:Record<string,string>,index:number){
 await page.goto('/messages/'+encodeURIComponent(ids['message-chat'])+'/'+encodeURIComponent(ids[native(index)]));
 const row=page.locator('[data-message="'+ids[native(index)]+'"]');await expect(row).toBeVisible();
 await expect.poll(()=>page.locator('.transcript').evaluate(element=>getComputedStyle(element).transform)).toBe('none');return row;
}
test('WEB-MSG-MEDIA TRANSCRIPT source videos dispatch into the real reserved transcript slot',async({page,request})=>{
 await mediaSession(page.context(),request);const ids=await control(request,'/demo-fixtures');await page.clock.setFixedTime(new Date('2026-10-20T16:00:00Z'));
 const row=await jump(page,ids,38),bubble=row.locator('.media-reserved');await expect(bubble.locator('.media-motion')).toBeVisible();
 await expect(bubble.locator('.media-motion .media-image')).toHaveAttribute('data-media-state','ready');const bounds=await bubble.boundingBox();expect(bounds!.width).toBeGreaterThanOrEqual(120);expect(bounds!.height).toBeGreaterThanOrEqual(120);
});
async function playbackPlates(page:Page,ids:Record<string,string>,info:TestInfo,cases:readonly (readonly [number,string,string])[],evidenceName='composed-media-geometry'){
 const plates=info.project.name.startsWith('webkit-')||info.project.name==='chromium-light-desktop',proofs=[];
 for(const [index,kind,direction] of cases){
  await page.mouse.move(0,0);const row=await jump(page,ids,index),bubble=row.locator('.media-motion');await expect(bubble).toBeVisible();if(direction==='sent')await expect(row).toHaveClass(/outgoing/u);else await expect(row).not.toHaveClass(/outgoing/u);
  if(index<129||index>130){await expect(row.locator('.bubble')).toHaveCount(2);await expect(row.locator('.bubble').first()).toContainText('synthetic studio.');}
  await expect(bubble.locator('.media-image')).toHaveAttribute('data-media-state','ready');await page.evaluate(()=>document.fonts.ready);await row.scrollIntoViewIfNeeded();await page.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));const chrome=await page.locator('.transcript>.toolbar').evaluate(element=>({rect:element.getBoundingClientRect().toJSON(),opacity:getComputedStyle(element).opacity,visibility:getComputedStyle(element).visibility,text:element.textContent,scrollY,icons:Array.from(element.querySelectorAll('svg')).map(icon=>icon.getBoundingClientRect().toJSON())}));const before=await bubble.boundingBox();expect(Math.min(before!.width,before!.height)).toBeGreaterThanOrEqual(120);
  const path=`../lab/evidence/private/web/media-transcript-${kind}-${direction}`;if(plates)await page.screenshot({path:`${path}-rest-${info.project.name}.png`});
  const button=bubble.locator('.media-open');await button.hover();await page.mouse.down();const video=bubble.locator('video');await expect(video).toBeVisible();await video.evaluate((element:HTMLVideoElement)=>{element.loop=true;});await expect(bubble).toHaveAttribute('data-media-frame','decoded');await expect(video).toHaveCSS('opacity','1');await expect.poll(()=>video.evaluate((element:HTMLVideoElement)=>element.currentTime)).toBeGreaterThan(.05);await expect(video).toHaveCSS('object-fit','cover');expect(await video.evaluate((element:HTMLVideoElement)=>element.muted&&!element.controls&&element.hasAttribute('playsinline'))).toBe(true);
  const during=await bubble.boundingBox();expect(during).toEqual(before);const decoded=await video.evaluate((element:HTMLVideoElement)=>({width:element.videoWidth,height:element.videoHeight}));proofs.push({index,kind,direction,before,during,decoded,chrome,shift:0});
  if(plates){await page.screenshot({path:`${path}-playing-clean-${info.project.name}.png`});await page.evaluate(({before,during})=>{const overlay=document.createElement('div');overlay.id='media-rect-proof';overlay.style.cssText='position:fixed;inset:0;pointer-events:none;z-index:99999';overlay.innerHTML=`<svg width="100%" height="100%"><rect x="${before!.x}" y="${before!.y}" width="${before!.width}" height="${before!.height}" fill="none" stroke="#00e5ff" stroke-width="6"/><rect x="${during!.x}" y="${during!.y}" width="${during!.width}" height="${during!.height}" fill="none" stroke="#ff3aab" stroke-width="2"/></svg>`;document.body.append(overlay);},{before,during});await page.screenshot({path:`${path}-playing-bounds-${info.project.name}.png`});await page.locator('#media-rect-proof').evaluate(element=>element.remove());}
  await page.mouse.up();await expect.poll(()=>video.evaluate((element:HTMLVideoElement)=>element.paused)).toBe(true);expect(await bubble.boundingBox()).toEqual(before);
 }
 await info.attach(evidenceName,{body:JSON.stringify(proofs),contentType:'application/json'});
}
test('WEB-MSG-MEDIA TRANSCRIPT native portrait, landscape and Live playback preserve composed bounds',async({page,request},info)=>{
 await mediaSession(page.context(),request);const ids=await control(request,'/media-playback-scene');await page.clock.setFixedTime(new Date('2026-10-20T16:00:00Z'));
 await playbackPlates(page,ids,info,[[38,'landscape','received'],[131,'portrait','received'],[132,'portrait','sent'],[133,'landscape','sent'],[129,'live','sent']]);
 const received=await control(request,'/media-playback-scene?direction=received');try{await playbackPlates(page,received,info,[[129,'live','received'],[130,'ultrawide','received']],'composed-received-live-wide-geometry');}finally{await control(request,'/media-playback-scene');}
});
test('WEB-MSG-MEDIA ULTRAWIDE the minimum-sized native panorama preserves composed playback bounds',async({page,request},info)=>{
 await mediaSession(page.context(),request);const ids=await control(request,'/demo-fixtures');await page.clock.setFixedTime(new Date('2026-10-20T16:00:00Z'));
 await playbackPlates(page,ids,info,[[130,'ultrawide','sent']]);
});
