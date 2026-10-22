import { test, expect } from '@playwright/test';
import { control } from './session';
import {mediaSession} from './media-session';
import {mkdir,readFile,writeFile} from 'node:fs/promises';
import {gzipSync} from 'node:zlib';
test('WEB-MSG-MEDIA image geometry is reserved before bytes and decoded thumb images crossfade without shift',async({page,context,request},info)=>{
    await mediaSession(context,request);
    const ids=await control(request,'/demo-fixtures');
    let release!:()=>void;
    const gate=new Promise<void>(resolve=>{release=resolve;});
    const variants:{record:string;variant:string}[]=[];
    await page.route('**/web/v1/blob/**',async route=>{const url=new URL(route.request().url());variants.push({record:url.searchParams.get('record')??'',variant:url.searchParams.get('variant')??''});await gate;await route.continue();});
    await page.goto('/__fixtures/media?chat='+encodeURIComponent(ids['message-chat']),{waitUntil:'domcontentloaded'});
    const media=page.locator('[data-media-id]');
    await expect(media.first()).toBeVisible();
    await page.evaluate(()=>document.fonts.ready);
    const before=await media.evaluateAll(elements=>elements.map(element=>{const r=element.getBoundingClientRect();return {id:element.getAttribute('data-media-id'),x:r.x,y:r.y,width:r.width,height:r.height};}));
    expect(before.length).toBeGreaterThanOrEqual(5);
    const portrait=page.locator(`[data-message="${ids['demo-message-00-000032']}"] [data-media-width="240"][data-media-height="400"]`);
    await expect(portrait).toHaveCount(1);
    const box=await portrait.boundingBox();expect(box!.width/box!.height).toBeCloseTo(.6,2);
    await expect(page.locator('.media-placeholder').first()).toHaveAttribute('data-thumbhash','decoded');
    expect(await media.locator('img').first().getAttribute('decoding')).toBe('async');
    await page.evaluate(()=>{(window as any).__mediaShifts=0;if(PerformanceObserver.supportedEntryTypes.includes('layout-shift'))new PerformanceObserver(list=>{for(const entry of list.getEntries())(window as any).__mediaShifts+=(entry as any).value;}).observe({type:'layout-shift'});});
    release();
    await expect.poll(()=>page.locator('[data-media-state="ready"]').count()).toBe(before.length);
    const after=await media.evaluateAll(elements=>elements.map(element=>{const r=element.getBoundingClientRect();return {id:element.getAttribute('data-media-id'),x:r.x,y:r.y,width:r.width,height:r.height};}));
    expect(after).toEqual(before);
    expect(await page.evaluate(()=>(window as any).__mediaShifts)).toBe(0);
    const liveIds=await page.locator('.media-motion[data-media-live="true"] [data-media-id]').evaluateAll(elements=>elements.map(element=>element.getAttribute('data-media-id')!));
    expect(liveIds.length).toBeGreaterThan(0);
    expect(before.filter(item=>!liveIds.includes(item.id!)).length).toBeGreaterThan(0);
    for(const item of before){const requests=variants.filter(value=>value.record===item.id);expect(requests.length,`requests for ${item.id}`).toBeGreaterThan(0);expect(requests.every(value=>value.variant===(liveIds.includes(item.id!)?'poster':'thumb')),`variant for ${item.id}`).toBe(true);}
    expect(variants.every(value=>before.some(item=>item.id===value.record)&&value.variant===(liveIds.includes(value.record)?'poster':'thumb'))).toBe(true);
    await expect.poll(()=>media.locator('img').evaluateAll(images=>images.every(image=>getComputedStyle(image).opacity==='1'))).toBe(true);
    await page.screenshot({path:`../lab/evidence/private/web/media-bubbles-${info.project.name}.png`,fullPage:true});
    await info.attach('media-geometry',{body:JSON.stringify({before,after,cls:0,variants}),contentType:'application/json'});
});
test('WEB-MSG-MEDIA a pending variant keeps its slot and retries without downloading the original',async({page,context,request})=>{
    await mediaSession(context,request);const ids=await control(request,'/demo-fixtures');
    let failedId='',failed=false;
    const variants:{record:string;variant:string}[]=[];
    await page.route('**/web/v1/blob/**',async route=>{const url=new URL(route.request().url());variants.push({record:url.searchParams.get('record')??'',variant:url.searchParams.get('variant')??''});if(!failed){failed=true;failedId=url.searchParams.get('record')!;await route.fulfill({status:503,contentType:'application/json',headers:{'Cache-Control':'no-store','Retry-After':'2'},body:JSON.stringify({error:{code:'variant_pending'}})});}else await route.continue();});
    await page.goto('/__fixtures/media?chat='+encodeURIComponent(ids['message-chat']),{waitUntil:'domcontentloaded'});
    await expect.poll(()=>failedId).not.toBe('');
    const image=page.locator(`[data-media-id="${failedId}"]`);
    await expect(image.getByRole('status')).toContainText('Media unavailable');
    const before=await image.boundingBox();
    await image.getByRole('button',{name:'Try again'}).click();
    await expect(image).toHaveAttribute('data-media-state','ready');
    expect(await image.boundingBox()).toEqual(before);
    const liveIds=await page.locator('.media-motion[data-media-live="true"] [data-media-id]').evaluateAll(elements=>elements.map(element=>element.getAttribute('data-media-id')!));
    expect(variants.filter(value=>value.record===failedId).every(value=>value.variant===(liveIds.includes(failedId)?'poster':'thumb'))).toBe(true);
    expect(variants.every(value=>value.variant===(liveIds.includes(value.record)?'poster':'thumb'))).toBe(true);
});
test('WEB-MSG-MEDIA alpha images and placeholders float without a backing tile or rounded clip',async({page,context,request})=>{
    await mediaSession(context,request);const ids=await control(request,'/demo-fixtures');
    let release!:()=>void;const gate=new Promise<void>(resolve=>{release=resolve;});
    await page.route('**/web/v1/blob/**',async route=>{await gate;await route.continue();});
    await page.goto('/__fixtures/media?chat='+encodeURIComponent(ids['message-chat']),{waitUntil:'domcontentloaded'});
    const slot=page.locator(`[data-message="${ids['demo-message-00-000036']}"]`),bubble=slot.locator('.media-bubble'),image=slot.locator('.media-image');
    await expect(image).toBeVisible();
    await expect(bubble).toHaveAttribute('data-media-alpha','true');
    for(const element of [slot,bubble,image]){await expect(element).toHaveCSS('background-color','rgba(0, 0, 0, 0)');await expect(element).toHaveCSS('border-radius','0px');await expect(element).toHaveCSS('overflow','visible');}
    expect(await image.locator('canvas').evaluate((canvas:HTMLCanvasElement)=>canvas.getContext('2d')!.getImageData(0,0,1,1).data[3])).toBeLessThan(255);
    const before=await slot.boundingBox();release();await expect(image).toHaveAttribute('data-media-state','ready');expect(await slot.boundingBox()).toEqual(before);
});
test('WEB-MSG-MEDIA MOTION video and Live Photo reserve geometry, request interoperable variants and play only on intent',async({page,context,request},info)=>{
 await mediaSession(context,request);const ids=await control(request,'/demo-fixtures');
 let release!:()=>void;const gate=new Promise<void>(resolve=>{release=resolve;});await page.route('**/web/v1/blob/**',async route=>{if(['poster','thumb'].includes(new URL(route.request().url()).searchParams.get('variant')??''))await gate;await route.continue();});
 const variants:string[]=[],types:string[]=[],posterRecords=new Set<string>();page.on('response',response=>{if(response.url().includes('/web/v1/blob/')){const url=new URL(response.url()),variant=url.searchParams.get('variant')!;variants.push(variant);if(variant==='poster')posterRecords.add(url.searchParams.get('record')!);if(variant==='video'||variant==='live')types.push(response.headers()['content-type']);}});
 await page.goto('/__fixtures/media?motion=1&chat='+encodeURIComponent(ids['message-chat']));await expect(page.getByRole('region',{name:'Media bubbles'})).toBeVisible();
 for(const [index,kind] of [[38,'video'],[129,'live']] as const){await page.mouse.move(0,0);
  const slot=page.locator(`[data-message="${ids[`demo-message-00-${String(index).padStart(6,'0')}`]}"]`),bubble=slot.locator('.media-motion');await expect(bubble).toBeVisible();await slot.scrollIntoViewIfNeeded();const before=await slot.boundingBox();expect(before!.width).toBe(info.project.name.endsWith('-phone')?240:320);expect(before!.height).toBe(kind==='live'?before!.width:before!.width*180/320);release();await expect(bubble.locator('.media-image')).toHaveAttribute('data-media-state','ready');expect(posterRecords.has((await bubble.locator('[data-media-id]').getAttribute('data-media-id'))!)).toBe(true);expect(await slot.boundingBox()).toEqual(before);
  await expect(bubble.locator('.media-motion-badge')).toHaveText(kind==='live'?'LIVE':'0:01');expect(await bubble.locator('video').count()).toBe(0);expect(variants).not.toContain(kind);
  const button=bubble.locator('.media-open');await button.focus();await page.keyboard.down('Space');const video=bubble.locator('video');await expect.poll(()=>video.evaluate((node:HTMLVideoElement)=>node.currentTime)).toBeGreaterThan(.05);expect(await video.evaluate((node:HTMLVideoElement)=>node.muted&&!node.controls&&node.hasAttribute('playsinline'))).toBe(true);await page.keyboard.up('Space');await expect.poll(()=>video.evaluate((node:HTMLVideoElement)=>node.paused)).toBe(true);expect(await slot.boundingBox()).toEqual(before);await expect(page.getByRole('dialog')).toHaveCount(0);
  await page.waitForTimeout(450);await button.click();const viewer=page.getByRole('dialog',{name:'Media viewer'});await expect(viewer).toBeVisible();await page.keyboard.press('Escape');await expect(viewer).toBeHidden();await expect(button).toBeFocused();expect(await slot.boundingBox()).toEqual(before);
 }
 expect(variants).toContain('poster');expect(variants).not.toContain('thumb');expect(variants).toContain('video');expect(variants).toContain('live');expect(variants).not.toContain('original');expect(types.length).toBeGreaterThanOrEqual(2);expect(types.every(value=>value.startsWith('video/mp4'))).toBe(true);
 const html=await readFile('dist/index.html','utf8'),entry=/<script[^>]+src="([^"]+)"/u.exec(html)![1],bytes=await readFile('dist'+entry),metrics={entry_chunk:entry,entry_bytes:bytes.length,entry_gzip9_bytes:gzipSync(bytes,{level:9}).length,entry_budget_gzip_bytes:60000,variants,content_types:types,decode_geometry_shift_px:0,playback_geometry_shift_px:0};expect(metrics.entry_gzip9_bytes).toBeLessThanOrEqual(60000);
 await mkdir('../lab/evidence/web/metrics',{recursive:true});await writeFile(`../lab/evidence/web/metrics/WEB-MSG-MEDIA-motion-${info.project.name}.json`,JSON.stringify(metrics,null,2)+'\n');
 await info.attach('motion-media-variants',{body:JSON.stringify(metrics),contentType:'application/json'});
 await page.screenshot({path:`../lab/evidence/private/web/media-motion-${info.project.name}.png`,fullPage:true});
});
test('WEB-MSG-MEDIA MOTION pointer hold cancels cleanly and Reduce Motion prevents hover playback',async({page,context,request})=>{
 await mediaSession(context,request);const ids=await control(request,'/demo-fixtures');await page.emulateMedia({reducedMotion:'reduce'});await page.goto('/__fixtures/media?motion=1&chat='+encodeURIComponent(ids['message-chat']));await expect(page.getByRole('region',{name:'Media bubbles'})).toBeVisible();
 const bubble=page.locator(`[data-message="${ids['demo-message-00-000129']}"] .media-motion`);await expect(bubble).toBeVisible();await bubble.scrollIntoViewIfNeeded();const button=bubble.locator('.media-open');await button.hover();await page.waitForTimeout(350);expect(await bubble.locator('video').count()).toBe(0);const before=await bubble.boundingBox();
 await page.mouse.down();await expect.poll(()=>bubble.locator('video').evaluate((node:HTMLVideoElement)=>node.currentTime)).toBeGreaterThan(.05);await page.mouse.up();await expect.poll(()=>bubble.locator('video').evaluate((node:HTMLVideoElement)=>node.paused)).toBe(true);await expect(page.getByRole('dialog')).toHaveCount(0);expect(await bubble.boundingBox()).toEqual(before);
 await page.mouse.down();await button.dispatchEvent('pointercancel',{pointerId:7});await page.mouse.up();await page.waitForTimeout(350);await expect.poll(()=>bubble.locator('video').evaluate((node:HTMLVideoElement)=>node.paused)).toBe(true);
});
test('WEB-MSG-MEDIA MOTION Live Photo hover respects pointer capability and ends when the pointer leaves',async({page,context,request})=>{
 await mediaSession(context,request);const ids=await control(request,'/demo-fixtures');await page.goto('/__fixtures/media?motion=1&chat='+encodeURIComponent(ids['message-chat']));await expect(page.getByRole('region',{name:'Media bubbles'})).toBeVisible();
 const bubble=page.locator(`[data-message="${ids['demo-message-00-000129']}"] .media-motion`);await expect(bubble).toBeVisible();await bubble.scrollIntoViewIfNeeded();const before=await bubble.boundingBox();await bubble.locator('.media-open').hover();
 if(await page.evaluate(()=>matchMedia('(hover:hover) and (pointer:fine)').matches)){await expect.poll(()=>bubble.locator('video').evaluate((node:HTMLVideoElement)=>node.currentTime)).toBeGreaterThan(.05);await page.mouse.move(0,0);await expect.poll(()=>bubble.locator('video').evaluate((node:HTMLVideoElement)=>node.paused)).toBe(true);}else{await page.waitForTimeout(350);expect(await bubble.locator('video').count()).toBe(0);}
 expect(await bubble.boundingBox()).toEqual(before);await expect(page.getByRole('dialog')).toHaveCount(0);
});
test('WEB-MSG-MEDIA CHROME glass play controls, Live badge and first-frame crossfade preserve bubble bounds',async({page,context,request},info)=>{
 await mediaSession(context,request);const ids=await control(request,'/demo-fixtures');
 await page.addInitScript(()=>{document.addEventListener('play',event=>{const node=event.target;if(node instanceof HTMLVideoElement&&node.closest('.media-motion[data-media-live="false"]'))node.loop=true;},true);const prototype=HTMLVideoElement.prototype,original=prototype.requestVideoFrameCallback;(window as any).__nativeFrameForPaint=original;(window as any).__heldFrames=[];if(original)prototype.requestVideoFrameCallback=function(callback){return original.call(this,(at,metadata)=>{(window as any).__heldFrames.push(()=>callback(at,metadata));});};});
 await page.goto('/__fixtures/media?motion=1&chat='+encodeURIComponent(ids['message-chat']));
 for(const [index,kind] of [[38,'video'],[129,'live']] as const){
  await page.mouse.move(0,0);const bubble=page.locator(`[data-message="${ids[`demo-message-00-${String(index).padStart(6,'0')}`]}"] .media-motion`);await expect(bubble).toBeVisible();await bubble.scrollIntoViewIfNeeded();await expect(bubble.locator('.media-image')).toHaveAttribute('data-media-state','ready');const before=await bubble.boundingBox(),badge=bubble.locator('.media-motion-badge');
  if(kind==='video'){
   const play=bubble.locator('.media-motion-play');await expect(play).toHaveCSS('width','44px');await expect(play).toHaveCSS('height','44px');await expect(play).toHaveCSS('color','rgb(255, 255, 255)');
   expect(await play.evaluate(element=>getComputedStyle(element).getPropertyValue('backdrop-filter')||getComputedStyle(element).getPropertyValue('-webkit-backdrop-filter'))).toContain('blur');
   const label=await badge.boundingBox();expect(label!.x-before!.x).toBeCloseTo(6,1);expect(before!.y+before!.height-label!.y-label!.height).toBeCloseTo(6,1);await expect(badge).toHaveText('0:01');
  }else{await expect(badge.locator('svg circle')).toHaveCount(3);await expect(badge).toContainText('LIVE');}
  const plates=info.project.name.startsWith('webkit-')||info.project.name==='chromium-light-desktop';if(plates)await page.screenshot({path:`../lab/evidence/private/web/media-motion-${kind}-rest-${info.project.name}.png`});
  const button=bubble.locator('.media-open');await button.hover();const hover=kind==='live'&&await page.evaluate(()=>matchMedia('(hover:hover) and (pointer:fine)').matches);
  if(!hover){await page.mouse.down();if(kind==='video'){await expect.poll(()=>bubble.locator('.media-motion-play').evaluate(element=>new DOMMatrix(getComputedStyle(element).transform).a)).toBeLessThan(.98);}}
  await expect(bubble).toHaveAttribute('data-media-playing','true');const video=bubble.locator('video'),callback=await video.evaluate(node=>typeof (node as HTMLVideoElement).requestVideoFrameCallback==='function');
  if(callback){await expect.poll(()=>page.evaluate(()=>(window as any).__heldFrames.length)).toBeGreaterThan(0);await expect(video).toHaveCSS('opacity','0');await page.evaluate(()=>{for(const callback of (window as any).__heldFrames.splice(0))callback();});}
  await expect(bubble).toHaveAttribute('data-media-frame','decoded');await expect(video).toHaveCSS('transition-duration','0.12s');await expect(video).toHaveCSS('opacity','1');await expect(badge).toHaveCSS('opacity','0');await expect(badge).toHaveCSS('visibility','hidden');if(kind==='video')await expect(bubble.locator('.media-motion-play')).toHaveCSS('visibility','hidden');const during=await bubble.boundingBox();expect(during).toEqual(before);
  expect(await video.evaluate((node:HTMLVideoElement)=>node.paused)).toBe(false);
  await video.evaluate((node:HTMLVideoElement)=>{node.playbackRate=.125;});
  await video.evaluate((node:HTMLVideoElement)=>new Promise<void>((resolve,reject)=>{
   let stage='video-frame',frame=0,paint=0;const native=(window as any).__nativeFrameForPaint;
   const timeout=setTimeout(()=>{node.cancelVideoFrameCallback(frame);cancelAnimationFrame(paint);reject(new Error(`Presented-frame timeout after 2000ms: ${JSON.stringify({stage,currentTime:node.currentTime,paused:node.paused,readyState:node.readyState,visibility:document.visibilityState})}`));},2000);
   if(typeof native!=='function'){clearTimeout(timeout);reject(new Error('Presented-frame callback unavailable'));return;}
   frame=native.call(node,()=>{stage='first-paint';paint=requestAnimationFrame(()=>{stage='second-paint';paint=requestAnimationFrame(()=>{clearTimeout(timeout);resolve();});});});
  }));
  const clean=await page.screenshot({scale:'css',...(plates?{path:`../lab/evidence/private/web/media-motion-${kind}-playing-clean-${info.project.name}.png`}:{})});
  const {corner,frameComparison}=await video.evaluate(async(node:HTMLVideoElement,{encoded,bounds})=>{
   const frame=document.createElement('canvas');frame.width=Math.round(bounds!.width);frame.height=Math.round(bounds!.height);const decoded=frame.getContext('2d')!,scale=Math.max(frame.width/node.videoWidth,frame.height/node.videoHeight);decoded.drawImage(node,(frame.width-node.videoWidth*scale)/2,(frame.height-node.videoHeight*scale)/2,node.videoWidth*scale,node.videoHeight*scale);
   const image=new Image();image.src='data:image/png;base64,'+encoded;await image.decode();const screenshot=document.createElement('canvas');screenshot.width=image.naturalWidth;screenshot.height=image.naturalHeight;const shot=screenshot.getContext('2d')!;shot.drawImage(image,0,0);
   const mean=(context:CanvasRenderingContext2D,x:number,y:number)=>{const bytes=context.getImageData(x,y,12,12).data,result=[0,0,0];for(let i=0;i<bytes.length;i+=4)for(let c=0;c<3;c++)result[c]+=bytes[i+c]/144;return result;};const points=[[frame.width-36,frame.height-36],[Math.round(frame.width*.4),Math.round(frame.height*.3)]],regions=points.map(([x,y])=>({x,y,screenshot:mean(shot,Math.round(bounds!.x+x),Math.round(bounds!.y+y)),decoded:mean(decoded,x,y)}));const residual=regions[0].screenshot.map((value,i)=>value-regions[0].decoded[i]-(regions[1].screenshot[i]-regions[1].decoded[i]));
   const actual=Array.from(shot.getImageData(Math.round(bounds!.x+2),Math.round(bounds!.y+2),1,1).data);shot.fillStyle=getComputedStyle(document.body).backgroundColor;shot.fillRect(0,0,1,1);return {corner:{actual,background:Array.from(shot.getImageData(0,0,1,1).data)},frameComparison:{regions,residual,controls:node.controls,playing:!node.paused}};
  },{encoded:clean.toString('base64'),bounds:during});
  await info.attach(`playing-${kind}-corner`,{body:JSON.stringify(corner),contentType:'application/json'});expect(corner.actual).toEqual(corner.background);
  await info.attach(`playing-${kind}-frame-comparison`,{body:JSON.stringify(frameComparison),contentType:'application/json'});expect(frameComparison.controls).toBe(false);expect(frameComparison.playing).toBe(true);expect(Math.max(...frameComparison.residual.map(Math.abs))).toBeLessThanOrEqual(3);

  if(plates){await page.evaluate(({before,during})=>{const overlay=document.createElement('div');overlay.id='media-rect-proof';overlay.setAttribute('aria-hidden','true');overlay.style.cssText='position:fixed;inset:0;pointer-events:none;z-index:99999';overlay.innerHTML=`<svg width="100%" height="100%"><rect x="${before!.x}" y="${before!.y}" width="${before!.width}" height="${before!.height}" fill="none" stroke="#00e5ff" stroke-width="6"/><rect x="${during!.x}" y="${during!.y}" width="${during!.width}" height="${during!.height}" fill="none" stroke="#ff3aab" stroke-width="2"/></svg><span style="position:absolute;top:8px;right:8px;background:#000c;color:white;padding:6px;font:12px system-ui">Cyan: before · Pink: playing · 0 px shift</span>`;document.body.append(overlay);},{before,during});await page.screenshot({path:`../lab/evidence/private/web/media-motion-${kind}-playing-${info.project.name}.png`});await page.locator('#media-rect-proof').evaluate(element=>element.remove());}
  if(hover)await page.mouse.move(0,0);else await page.mouse.up();await expect(bubble).toHaveAttribute('data-media-playing','false');await expect(badge).toHaveCSS('visibility','visible');expect(await bubble.boundingBox()).toEqual(before);
 }
});
test('WEB-MSG-MEDIA CHROME loadeddata is the decoded-frame fallback and Reduce Motion uses a short fade',async({page,context,request})=>{
 await mediaSession(context,request);const ids=await control(request,'/demo-fixtures');await page.emulateMedia({reducedMotion:'reduce'});await page.addInitScript(()=>{Object.defineProperty(HTMLVideoElement.prototype,'requestVideoFrameCallback',{value:undefined,configurable:true});});await page.goto('/__fixtures/media?motion=1&chat='+encodeURIComponent(ids['message-chat']));
 const bubble=page.locator(`[data-message="${ids['demo-message-00-000038']}"] .media-motion`);await bubble.locator('.media-open').focus();const before=await bubble.boundingBox();await page.keyboard.down('Space');await expect(bubble).toHaveAttribute('data-media-frame','decoded');await expect(bubble).toHaveAttribute('data-media-frame-source','loadeddata');await expect(bubble.locator('video')).toHaveCSS('transition-duration','0.15s');await expect(bubble.locator('video')).toHaveCSS('opacity','1');await page.keyboard.up('Space');expect(await bubble.boundingBox()).toEqual(before);
});
