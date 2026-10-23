import {test,expect,type APIRequestContext,type BrowserContext} from '@playwright/test';
import {control} from './session';
import {mkdir,readFile,writeFile} from 'node:fs/promises';
import {gzipSync} from 'node:zlib';
test.use({locale:'en-US',timezoneId:'America/Chicago'});
let session:Promise<{cookie:Parameters<BrowserContext['addCookies']>[0][number]}>|undefined;
async function setup(context:BrowserContext,request:APIRequestContext){const value=await(session??=control(request,'/session?kind=public'));await context.addCookies([value.cookie]);return control(request,'/demo-fixtures');}
test('WEB-MSG-VIEWER META collection source pages beyond the transcript window with interleaved photos and playable video',async({page,context,request},info)=>{
 const ids=await setup(context,request),subscriptions:Record<string,unknown>[]=[],viewerChunks:string[]=[];
 await page.addInitScript(()=>{window.requestIdleCallback=callback=>{(window as any).__viewerIdle=()=>callback({didTimeout:false,timeRemaining:()=>50});return 1;};window.cancelIdleCallback=()=>{delete (window as any).__viewerIdle;};});
 await page.addInitScript(()=>{const show=HTMLDialogElement.prototype.showModal,send=WebSocket.prototype.send;(window as any).__collectionAfterPaint=[];HTMLDialogElement.prototype.showModal=function(){const result=show.call(this);if(this.classList.contains('media-viewer'))requestAnimationFrame(()=>(window as any).__viewerFrame=true);return result;};WebSocket.prototype.send=function(data){if(typeof data==='string'){const frame=JSON.parse(data);if(frame.t==='sub'&&frame.view==='collection'&&frame.args.kind==='media')(window as any).__collectionAfterPaint.push(Boolean((window as any).__viewerFrame));}return send.call(this,data);};});
 await page.addInitScript(id=>{const show=HTMLDialogElement.prototype.showModal;HTMLDialogElement.prototype.showModal=function(){if(this.classList.contains('media-viewer'))(window as any).__sourceHiddenBeforeModal=[...document.querySelectorAll<HTMLElement>('[data-message="'+id+'"],[data-message="'+id+'"] *')].some(element=>element.style.visibility==='hidden');return show.call(this);};},ids['demo-message-00-000032']);
 page.on('request',request=>{if(/\/assets\/ViewerSession-[^/]+\.js$/u.test(new URL(request.url()).pathname))viewerChunks.push(request.url());});
 await page.addInitScript(()=>Object.defineProperty(navigator,'share',{value:async(payload:ShareData)=>{Object.defineProperty(window,'viewerShared',{value:payload,configurable:true});}}));
 page.on('websocket',socket=>socket.on('framesent',({payload})=>{try{const frame=JSON.parse(String(payload));if(frame.t==='sub')subscriptions.push(frame);}catch{}}));
 const variants:string[]=[];page.on('request',request=>{const url=new URL(request.url());if(url.pathname.startsWith('/web/v1/blob/'))variants.push(url.searchParams.get('variant')??'');});
 await page.goto('/__fixtures/media?chat='+encodeURIComponent(ids['message-chat'])+'&window=1&anchor='+encodeURIComponent(ids['demo-message-00-000032'])+'&pageSize=2');
 const source=page.locator(`[data-message="${ids['demo-message-00-000032']}"]`);await source.scrollIntoViewIfNeeded();const before=await source.boundingBox();
 expect(viewerChunks).toHaveLength(0);await source.locator('.media-open').hover();await expect.poll(()=>viewerChunks.length).toBeGreaterThan(0);await expect(page.getByRole('dialog',{name:'Media viewer'})).toHaveCount(0);
 expect(subscriptions.filter(value=>value.view==='collection')).toHaveLength(0);
 await source.locator('.media-open').click();const viewer=page.getByRole('dialog',{name:'Media viewer'});await expect(viewer).toBeVisible();await expect(viewer.locator('.media-viewer-stage')).toBeFocused();
 expect(await page.evaluate(()=>(window as any).__sourceHiddenBeforeModal)).toBe(true);
 await expect(viewer.locator('.media-viewer-preview')).toHaveCount(0);await page.evaluate(()=>(window as any).__viewerIdle());await expect(viewer.locator('.media-viewer-preview')).toHaveCount(2);
 const decoded=await viewer.locator('.media-viewer-stage .media-image>img').elementHandle();
 await expect(viewer).toHaveAttribute('data-media-loading','false');await expect(viewer).toHaveAttribute('data-media-count','13');expect(await decoded!.evaluate(element=>element.isConnected)).toBe(true);
 await expect(viewer.locator('.media-viewer-heading time')).toHaveText('Dec 31, 2024 at 6:11 PM');
 expect(subscriptions.some(value=>value.view==='collection'&&(value.args as Record<string,unknown>).kind==='media')).toBe(true);
 expect(subscriptions.filter(value=>value.view==='collection').length).toBeGreaterThan(1);
 expect(await page.evaluate(()=>(window as any).__collectionAfterPaint.every(Boolean))).toBe(true);
 await expect(page.locator('.media-fixture-slot')).toHaveCount(1);
 await expect(viewer.locator('.media-viewer-stage .media-image')).toHaveAttribute('data-media-state','ready');expect(variants).toContain('display');
 await page.mouse.move(24,24);await viewer.getByRole('button',{name:'Share media'}).click();expect(await page.evaluate(()=>Boolean((window as unknown as {viewerShared?:ShareData}).viewerShared?.url))).toBe(true);
 await page.screenshot({path:`../lab/evidence/private/web/media-viewer-${info.project.name}.png`});
 await viewer.locator('.media-viewer-stage').focus();for(let index=0;index<6;index++)await page.keyboard.press('ArrowRight');
 await expect(viewer).toHaveAttribute('data-media-message',ids['demo-message-00-000038']);
 const video=viewer.locator('video');await expect(video).toBeVisible();
 await expect.poll(()=>video.evaluate((value:HTMLVideoElement)=>value.currentTime)).toBeGreaterThan(.05);
 expect(variants).toContain('video');
 await page.mouse.move(24,24);await video.evaluate((value:HTMLVideoElement)=>value.pause());await viewer.getByRole('button',{name:'Play video',exact:true}).click();await viewer.locator('.media-viewer-stage').focus();await page.keyboard.press(' ');await expect(video).toHaveJSProperty('paused',true);
 const scrub=viewer.getByRole('slider',{name:'Video position'});await scrub.focus();await page.keyboard.press('Home');await page.keyboard.press('ArrowRight');await expect.poll(()=>video.evaluate((value:HTMLVideoElement)=>value.currentTime)).toBeGreaterThan(0);
 await expect(viewer.getByRole('link',{name:'Download original'})).toHaveAttribute('href',/variant=original/u);
 await page.mouse.move(24,24);const download=page.waitForEvent('download');await viewer.getByRole('link',{name:'Download original'}).click();await download;expect(variants).toContain('original');
 await page.mouse.move(24,24);await viewer.getByRole('button',{name:'Show in Chat'}).click();await expect(viewer).toBeHidden();
 await expect(page.locator('.media-fixture')).toHaveAttribute('data-shown-message',ids['demo-message-00-000038']);
 expect(await source.boundingBox()).toEqual(before);
 const html=await readFile('dist/index.html','utf8'),entry=/<script[^>]+src="([^"]+)"/u.exec(html)![1],bytes=await readFile('dist'+entry),gzip=gzipSync(bytes).byteLength;
 expect(gzip).toBeLessThanOrEqual(60000);expect(bytes.toString()).not.toContain('Media viewer');expect(bytes.toString()).not.toContain('Play video');
 await mkdir('../lab/evidence/web/metrics',{recursive:true});await writeFile(`../lab/evidence/web/metrics/WEB-MSG-VIEWER-${info.project.name}.json`,JSON.stringify({entry_chunk:entry,entry_bytes:bytes.byteLength,entry_gzip_bytes:gzip,entry_gzip9_bytes:gzipSync(bytes,{level:9}).byteLength,entry_budget_gzip_bytes:60000,viewer_chunks:viewerChunks.map(value=>new URL(value).pathname),viewer_requested_at_startup:false,viewer_prefetched_on_hover:true},null,2)+'\n');
});
test('WEB-MSG-VIEWER zoom and swipe gestures track input then dismiss into the reserved source',async({page,context,request})=>{
 const ids=await setup(context,request);await page.goto('/__fixtures/media?chat='+encodeURIComponent(ids['message-chat']));
 const source=page.locator(`[data-message="${ids['demo-message-00-000033']}"]`);await source.scrollIntoViewIfNeeded();const before=await source.boundingBox();await source.locator('.media-open').click();
 const viewer=page.getByRole('dialog',{name:'Media viewer'});await expect(viewer).toHaveAttribute('data-media-loading','false');
 await expect.poll(()=>viewer.locator('.media-viewer-stage').evaluate(element=>new DOMMatrixReadOnly(getComputedStyle(element).transform).a)).toBeCloseTo(1,3);
 const bounds=await viewer.locator('.media-viewer-stage').boundingBox(),x=bounds!.x+bounds!.width/2,y=bounds!.y+bounds!.height/2;
 await page.mouse.dblclick(x,y);await expect(viewer).toHaveAttribute('data-media-zoom','2.5');
 await page.mouse.wheel(0,-100);await expect.poll(async()=>Number(await viewer.getAttribute('data-media-zoom'))).toBeGreaterThan(2.5);
 await page.mouse.dblclick(x,y);await expect(viewer).toHaveAttribute('data-media-zoom','1');
 await viewer.evaluate((element,{x,y})=>{
  const send=(type:string,id:number,clientX:number)=>element.dispatchEvent(new PointerEvent(type,{bubbles:true,pointerId:id,pointerType:'touch',clientX,clientY:y}));
  send('pointerdown',91,x-50);send('pointerdown',92,x+50);send('pointermove',92,x+150);send('pointerup',91,x-50);send('pointerup',92,x+150);
 },{x,y});
 await expect.poll(async()=>Number(await viewer.getAttribute('data-media-zoom'))).toBeGreaterThan(1);
 await page.mouse.dblclick(x,y);await expect(viewer).toHaveAttribute('data-media-zoom','1');
 await page.mouse.move(x,y);await page.mouse.down();await page.mouse.move(x-Math.max(120,page.viewportSize()!.width*.25),y,{steps:12});await page.mouse.up();
 await expect(viewer).toHaveAttribute('data-media-message',ids['demo-message-00-000034']);
 await page.mouse.move(x,y);await page.mouse.down();await page.mouse.move(x,y+180,{steps:12});await page.mouse.up();await expect(viewer).toBeHidden();
 expect(await source.boundingBox()).toEqual(before);
});
test('WEB-MSG-VIEWER Reduce Motion uses a short fade with stable geometry and Escape dismissal',async({page,context,request})=>{
 const ids=await setup(context,request);await page.emulateMedia({reducedMotion:'reduce'});await page.goto('/__fixtures/media?chat='+encodeURIComponent(ids['message-chat']));
 const source=page.locator(`[data-message="${ids['demo-message-00-000036']}"]`);await source.scrollIntoViewIfNeeded();const before=await source.boundingBox();await source.locator('.media-open').click();
 const viewer=page.getByRole('dialog',{name:'Media viewer'});await expect(viewer).toBeVisible();
 expect(await viewer.evaluate(element=>element.getAnimations({subtree:true}).every(animation=>Number(animation.effect?.getTiming().duration)<=150))).toBe(true);
 await page.keyboard.press('Escape');await expect(viewer).toBeHidden();expect(await source.boundingBox()).toEqual(before);
});
test('WEB-MSG-VIEWER DISMISS themed backdrop and interactive dismissal preserve the source silhouette',async({page,context,request},info)=>{
 const ids=await setup(context,request);await page.goto('/__fixtures/media?chat='+encodeURIComponent(ids['message-chat']));
 const source=page.locator(`[data-message="${ids['demo-message-00-000033']}"]`);await source.locator('.media-open').click();
 const viewer=page.getByRole('dialog',{name:'Media viewer'}),stage=viewer.locator('.media-viewer-stage');
 await expect.poll(()=>stage.evaluate(element=>new DOMMatrixReadOnly(getComputedStyle(element).transform).a)).toBeCloseTo(1,3);
 await expect.soft(viewer.locator('.media-viewer-backdrop')).toHaveCSS('background-color',info.project.name.includes('-dark-')?'rgb(0, 0, 0)':'rgb(255, 255, 255)');
 const bounds=await stage.boundingBox(),x=bounds!.x+bounds!.width/2,y=bounds!.y+bounds!.height/2;
 await page.mouse.move(x,y);await page.mouse.down();await page.mouse.move(x,y+70,{steps:12});
 expect.soft(await stage.evaluate(element=>new DOMMatrixReadOnly(getComputedStyle(element).transform).a)).toBeLessThan(.99);
 const progress=Math.min(1,70/(page.viewportSize()!.height*.3));
 const radius=await stage.evaluate(element=>Number(/round ([\d.]+)px/u.exec(getComputedStyle(element).clipPath)?.[1]??0));expect.soft(radius).toBeGreaterThan(0);
 expect.soft(await viewer.locator('.media-viewer-backdrop').evaluate(element=>Number(getComputedStyle(element).opacity))).toBeCloseTo(1-progress*.7,2);
 expect.soft(await viewer.locator('.media-viewer-toolbar').evaluate(element=>Number(getComputedStyle(element).opacity))).toBeLessThan(.05);
 await expect.soft(source).toHaveCSS('visibility','hidden');
 if(info.project.name.endsWith('-phone'))await expect.soft(viewer.locator('.media-viewer-next')).toBeHidden();
 await page.mouse.move(x,y,{steps:12});await page.mouse.up();await expect(viewer).toBeVisible();await expect.poll(()=>viewer.locator('.media-viewer-toolbar').evaluate(element=>Number(getComputedStyle(element).opacity))).toBeGreaterThan(.95);
 await viewer.evaluate((element,{x,y})=>{for(const [type,dy] of [['pointerdown',0],['pointermove',70],['pointermove',0],['pointerup',0]] as const)element.dispatchEvent(new PointerEvent(type,{bubbles:true,pointerId:101,pointerType:'touch',clientX:x,clientY:y+dy}));},{x,y});await expect(viewer).toHaveAttribute('data-chrome-visible','true');
 await page.keyboard.press('Escape');await expect(viewer).toBeHidden();await expect.soft(source.locator('.media-open')).toBeFocused();
});
test('WEB-MSG-VIEWER Apple chrome uses sprite actions, sender metadata and idle or edge interaction',async({page,context,request},info)=>{
 const ids=await setup(context,request);await page.goto('/__fixtures/media?chat='+encodeURIComponent(ids['message-chat']));
 const source=page.locator(`[data-message="${ids['demo-message-00-000032']}"]`);await source.scrollIntoViewIfNeeded();await source.locator('.media-open').click();
 const viewer=page.getByRole('dialog',{name:'Media viewer'}),toolbar=viewer.locator('.media-viewer-toolbar'),stage=viewer.locator('.media-viewer-stage');
 for(const [label,symbol] of [['Close media viewer','close'],['Share media','share'],['Download original','download'],['Show in Chat','messages']]){
  const action=viewer.getByRole(label==='Download original'?'link':'button',{name:label,exact:true});
  await expect(action.locator('svg use')).toHaveAttribute('href','/icons.svg#'+symbol);await expect(action).toHaveAttribute('title',/\S/u);await expect(action).toHaveText('');
 }
 await expect(viewer.locator('.media-viewer-heading strong')).toContainText('Cobalt02');
 await expect(viewer.locator('.media-viewer-heading time')).toContainText(/\d{1,2}:\d{2}/u);await expect(toolbar).not.toContainText(/\d+ of \d+/u);
 const bar=await toolbar.boundingBox();expect(bar!.x).toBe(0);expect(bar!.width).toBe(page.viewportSize()!.width);
 await expect.poll(()=>stage.evaluate(element=>new DOMMatrixReadOnly(getComputedStyle(element).transform).a)).toBeCloseTo(1,3);
 const bounds=await stage.boundingBox(),x=bounds!.x+bounds!.width/2,y=bounds!.y+bounds!.height/2;
 await page.mouse.move(x,y);await expect.poll(()=>toolbar.evaluate(element=>Number(getComputedStyle(element).opacity)),{timeout:4000}).toBeLessThan(.05);
 await page.mouse.move(x+12,y);await expect.poll(()=>toolbar.evaluate(element=>Number(getComputedStyle(element).opacity))).toBeGreaterThan(.95);
 await stage.click({position:{x:bounds!.width/2,y:bounds!.height/2}});await expect.poll(()=>toolbar.evaluate(element=>Number(getComputedStyle(element).opacity))).toBeLessThan(.05);
 await stage.click({position:{x:bounds!.width/2,y:bounds!.height/2}});await expect.poll(()=>toolbar.evaluate(element=>Number(getComputedStyle(element).opacity))).toBeGreaterThan(.95);
 const next=viewer.locator('.media-viewer-next');await expect(next.locator('svg use')).toHaveAttribute('href','/icons.svg#chevron');
 if(info.project.name.endsWith('-desktop')){await page.mouse.move(x,y);await expect.poll(()=>next.evaluate(element=>Number(getComputedStyle(element).opacity))).toBeLessThan(.05);await page.mouse.move(page.viewportSize()!.width-24,y);await expect.poll(()=>next.evaluate(element=>Number(getComputedStyle(element).opacity))).toBeGreaterThan(.95);}
 await viewer.locator('.media-viewer-stage').focus();await page.keyboard.press('ArrowRight');await expect(viewer.locator('.media-viewer-heading strong')).toHaveText('You');
 await page.keyboard.press('ArrowLeft');await expect(viewer.locator('.media-viewer-heading strong')).toContainText('Cobalt02');await expect.poll(()=>stage.evaluate(element=>new DOMMatrixReadOnly(getComputedStyle(element).transform).m41)).toBeCloseTo(0,3);await expect(stage.locator('.media-image')).toHaveAttribute('data-media-state','ready');await expect(stage.locator('.media-image>img')).toHaveCSS('opacity','1');await page.mouse.move(x+24,y);await expect.poll(()=>toolbar.evaluate(element=>Number(getComputedStyle(element).opacity))).toBeGreaterThan(.95);await page.screenshot({path:`../lab/evidence/private/web/media-viewer-chrome-${info.project.name}.png`});
 await page.mouse.move(x,y);await page.mouse.down();await page.mouse.move(x,y+120,{steps:12});await page.screenshot({path:`../lab/evidence/private/web/media-viewer-dismiss-${info.project.name}.png`});await page.mouse.up();await expect(viewer).toBeHidden();
});
test('WEB-MSG-VIEWER press prefetches code without opening or downloading display media',async({page,context,request},info)=>{
 const ids=await setup(context,request),chunks:string[]=[],variants:string[]=[];
 page.on('request',request=>{const url=new URL(request.url());if(/\/assets\/ViewerSession-[^/]+\.js$/u.test(url.pathname))chunks.push(url.pathname);if(url.pathname.startsWith('/web/v1/blob/'))variants.push(url.searchParams.get('variant')??'');});
 await page.goto('/__fixtures/media?chat='+encodeURIComponent(ids['message-chat']));const source=page.locator(`[data-message="${ids['demo-message-00-000032']}"] .media-open`);await expect(source).toBeAttached();expect(chunks).toHaveLength(0);
 await source.dispatchEvent('pointerdown',{pointerId:91,pointerType:'touch',isPrimary:true});await expect.poll(()=>chunks.length).toBeGreaterThan(0);await expect(page.getByRole('dialog',{name:'Media viewer'})).toHaveCount(0);expect(variants).not.toContain('display');expect(variants).not.toContain('video');
 const path=`../lab/evidence/web/metrics/WEB-MSG-VIEWER-${info.project.name}.json`,metrics=JSON.parse(await readFile(path,'utf8'));await writeFile(path,JSON.stringify({...metrics,viewer_prefetched_on_press:true},null,2)+'\n');
});

test('WEB-MSG-VIEWER opening a transcript photo trims background overscan and restores it on close',async({page,context,request})=>{
 const ids=await setup(context,request),id=ids['demo-message-00-000032'];await page.goto('/messages/'+encodeURIComponent(ids['message-chat'])+'/'+encodeURIComponent(id));
 await expect(page.locator('.messages-transcript')).toHaveAttribute('aria-busy','false');await expect(page.locator('main.transcript')).toHaveCSS('transform','none');
 await expect.poll(async()=>Number(await page.locator('.messages-transcript .virtual-row').first().getAttribute('aria-setsize'))).toBeGreaterThan(100);await page.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));
 const rows=page.locator('.messages-transcript .virtual-row'),before=await rows.count(),source=page.locator('[data-message="'+id+'"] .media-open'),box=await source.boundingBox();
 await source.click();const viewer=page.getByRole('dialog',{name:'Media viewer',exact:true});await expect(viewer).toBeVisible();await expect.poll(()=>rows.count()).toBeLessThan(before);
 await page.keyboard.press('Escape');await expect(viewer).not.toBeVisible();await expect(rows).toHaveCount(before);expect(await source.boundingBox()).toEqual(box);
});

test('WEB-MSG-VIEWER early opening preserves source geometry when the held first prefetch publishes',async({page,context,request})=>{
 const ids=await setup(context,request),id=ids['demo-message-00-000032'],held:(()=>void)[]=[];let holding=true;const release=()=>{holding=false;held.splice(0).forEach(send=>send());};
 await page.routeWebSocket('**/web/v1/live',socket=>{const server=socket.connectToServer(),prefetch=new Set<string>();socket.onMessage(data=>{const frame=JSON.parse(String(data));if(frame.t==='sub'&&frame.view==='thread'&&(frame.args.before||frame.args.after))prefetch.add(frame.id);server.send(data);});server.onMessage(data=>{const frame=JSON.parse(String(data));if(holding&&prefetch.has(frame.sub))held.push(()=>socket.send(data));else socket.send(data);});});
 try{
  await page.goto('/messages/'+encodeURIComponent(ids['message-chat'])+'/'+encodeURIComponent(id));await expect.poll(()=>held.length).toBeGreaterThan(0);await expect(page.locator('.messages-transcript')).toHaveAttribute('aria-busy','false');await expect(page.locator('main.transcript')).toHaveCSS('transform','none');
  const source=page.locator('[data-message="'+id+'"] .media-open'),box=await source.boundingBox();await source.click();const viewer=page.getByRole('dialog',{name:'Media viewer',exact:true});await expect(viewer).toBeVisible();release();
  await expect.poll(async()=>Number(await page.locator('.messages-transcript .virtual-row').first().getAttribute('aria-setsize'))).toBeGreaterThan(100);await page.keyboard.press('Escape');await expect(viewer).not.toBeVisible();expect(await source.boundingBox()).toEqual(box);
 }finally{release();}
});
