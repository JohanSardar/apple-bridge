import {test,expect} from '@playwright/test';
import {control} from './session';
import {mediaSession} from './media-session';

test('WEB-MSG-MEDIA POSTER a retina panorama keeps source geometry and uses immutable derivative identity',async({page,request},info)=>{
 await mediaSession(page.context(),request);const ids=await control(request,'/demo-fixtures');
 let release!:()=>void;const gate=new Promise<void>(resolve=>{release=resolve;}),responses:{digest:string;etag:string}[]=[];
 await page.route('**/web/v1/blob/**',async route=>{if(new URL(route.request().url()).searchParams.get('variant')==='poster')await gate;await route.continue();});
 page.on('response',response=>{const url=new URL(response.url());if(url.pathname.startsWith('/web/v1/blob/')&&url.searchParams.get('variant')==='poster'&&response.status()===200)responses.push({digest:url.pathname.split('/').at(-1)!,etag:(response.headers()['etag']??'').replace(/^"|"$/gu,'')});});
 await page.mouse.move(0,0);await page.goto('/messages/'+encodeURIComponent(ids['message-chat'])+'/'+encodeURIComponent(ids['demo-message-00-000130']));
 const slot=page.locator('[data-message="'+ids['demo-message-00-000130']+'"] .media-reserved'),image=slot.locator('.media-image');
 await expect(slot).toBeVisible();await expect(page.locator('.messages-transcript')).toHaveAttribute('aria-busy','false');await expect(page.locator('.transcript')).toHaveCSS('transform','none');await expect(page.locator('.transcript')).toHaveCSS('opacity','1');
 await page.evaluate(()=>document.fonts.ready);await slot.locator('.media-open').focus();await page.evaluate(()=>new Promise<void>(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>resolve()))));const before=await slot.boundingBox();
 expect(Math.min(before!.width,before!.height)).toBeGreaterThanOrEqual(120);await expect(image).toHaveAttribute('data-media-width','1920');await expect(image).toHaveAttribute('data-media-height','200');
 await page.evaluate(()=>{(window as any).__posterLayoutShift=null;if(PerformanceObserver.supportedEntryTypes.includes('layout-shift')){(window as any).__posterLayoutShift=0;new PerformanceObserver(list=>{for(const entry of list.getEntries())(window as any).__posterLayoutShift+=(entry as any).value;}).observe({type:'layout-shift'});}});
 release();await expect(image).toHaveAttribute('data-media-state','ready');const natural=await image.locator('img').evaluate((node:HTMLImageElement)=>({width:node.naturalWidth,height:node.naturalHeight}));
 expect.soft(Math.min(natural.width,natural.height)).toBeGreaterThanOrEqual(360);expect(natural.width*natural.height).toBeLessThanOrEqual(1500000);expect(natural.width/natural.height).toBeCloseTo(1920/200,2);
 await expect.poll(()=>responses.length).toBeGreaterThan(0);for(const response of responses)expect.soft(response.digest).toBe(response.etag);
 await expect.poll(()=>image.locator('img').evaluate(node=>getComputedStyle(node).opacity)).toBe('1');
 const trigger=slot.locator('.media-open');await expect(trigger).toBeFocused();const focusOutline=await trigger.evaluate(node=>({visible:node.matches(':focus-visible'),width:getComputedStyle(node).outlineWidth,color:getComputedStyle(node).outlineColor}));await trigger.evaluate(node=>(node as HTMLButtonElement).blur());await expect(trigger).not.toBeFocused();await expect(trigger).toHaveCSS('outline-style','none');await expect(trigger).toHaveCSS('border-top-width','0px');await expect(slot).toHaveCSS('outline-style','none');
 await page.screenshot({path:'../lab/evidence/private/web/media-poster-'+(Math.min(natural.width,natural.height)>=360?'retina':'baseline')+'-'+info.project.name+'-clean.png'});
 await page.evaluate(()=>new Promise<void>(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>resolve()))));const cls=await page.evaluate(()=>(window as any).__posterLayoutShift);if(cls!==null)expect(cls).toBe(0);
 expect(await slot.boundingBox()).toEqual(before);await info.attach('panorama-poster-resolution-and-cache-identity',{body:JSON.stringify({before,after:await slot.boundingBox(),natural,responses,cls,focusOutline}),contentType:'application/json'});
});

test('WEB-MSG-MEDIA POSTER a Live Photo uses its high resolution key still without moving the reserved bubble',async({page,request},info)=>{
 await mediaSession(page.context(),request);const ids=await control(request,'/demo-fixtures');
 await page.mouse.move(0,0);await page.goto('/messages/'+encodeURIComponent(ids['message-chat'])+'/'+encodeURIComponent(ids['demo-message-00-000129']));
 const slot=page.locator('[data-message="'+ids['demo-message-00-000129']+'"] .media-reserved'),image=slot.locator('.media-image');
 await expect(slot).toBeVisible();await expect(page.locator('.messages-transcript')).toHaveAttribute('aria-busy','false');await expect(page.locator('.transcript')).toHaveCSS('transform','none');await expect(image).toHaveAttribute('data-media-state','ready');
 await expect(page.locator('.transcript')).toHaveCSS('opacity','1');await page.evaluate(()=>document.fonts.ready);await slot.locator('.media-open').focus();await page.evaluate(()=>new Promise<void>(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>resolve()))));
 const before=await slot.boundingBox(),natural=await image.locator('img').evaluate((node:HTMLImageElement)=>({width:node.naturalWidth,height:node.naturalHeight,source:node.currentSrc}));
 expect(new URL(natural.source).searchParams.get('variant')).toBe('poster');expect(Math.min(natural.width,natural.height)).toBeGreaterThanOrEqual(360);expect(natural.width*natural.height).toBeLessThanOrEqual(1500000);
 await page.keyboard.down('Space');await expect(slot.locator('.media-motion')).toHaveAttribute('data-media-frame','decoded');expect(await slot.boundingBox()).toEqual(before);await page.keyboard.up('Space');expect(await slot.boundingBox()).toEqual(before);await expect.poll(()=>slot.locator('video').evaluate(node=>getComputedStyle(node).opacity)).toBe('0');await expect.poll(()=>slot.locator('.media-motion-badge').evaluate(node=>getComputedStyle(node).opacity)).toBe('1');
 await expect.poll(()=>image.locator('img').evaluate(node=>getComputedStyle(node).opacity)).toBe('1');
 const trigger=slot.locator('.media-open');await expect(trigger).toBeFocused();const focusOutline=await trigger.evaluate(node=>({visible:node.matches(':focus-visible'),width:getComputedStyle(node).outlineWidth,color:getComputedStyle(node).outlineColor}));await trigger.evaluate(node=>(node as HTMLButtonElement).blur());await expect(trigger).not.toBeFocused();await expect(trigger).toHaveCSS('outline-style','none');await expect(trigger).toHaveCSS('border-top-width','0px');await expect(slot).toHaveCSS('outline-style','none');
 await page.screenshot({path:'../lab/evidence/private/web/media-poster-live-retina-'+info.project.name+'-clean.png'});await info.attach('live-key-still-resolution',{body:JSON.stringify({before,natural,focusOutline}),contentType:'application/json'});
});
