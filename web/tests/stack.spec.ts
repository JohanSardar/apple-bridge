import {test,expect,type APIRequestContext,type BrowserContext} from '@playwright/test';
import {control} from './session';
import {mediaSession} from './media-session';
import {mkdir,readFile,writeFile} from 'node:fs/promises';
import {gzipSync} from 'node:zlib';
async function setup(context:BrowserContext,request:APIRequestContext){await mediaSession(context,request);return control(request,'/demo-fixtures');}
function path(ids:Record<string,string>){return '/__fixtures/media?chat='+encodeURIComponent(ids['message-chat'])+[42,43,44,45].map(index=>'&stack='+encodeURIComponent(ids[`demo-message-00-${String(index).padStart(6,'0')}`])).join('');}
test('WEB-MSG-MEDIA-STACK fixed bounds survive decode, phone swiping and desktop grid opening into the exact cell',async({page,context,request},info)=>{
 const ids=await setup(context,request);let release!:()=>void;const gate=new Promise<void>(resolve=>{release=resolve;}),variants:string[]=[];
 await page.route('**/web/v1/blob/**',async route=>{variants.push(new URL(route.request().url()).searchParams.get('variant')??'');await gate;await route.continue();});
 await page.goto(path(ids),{waitUntil:'domcontentloaded'});const stack=page.getByRole('group',{name:'Photos'});await expect(stack).toBeVisible();
 const cells=stack.locator('.media-stack-cell');await expect(cells).toHaveCount(4);await page.evaluate(()=>document.fonts.ready);const before=await stack.boundingBox();
 expect(before!.width).toBe(info.project.name.endsWith('-phone')?260:360);expect(before!.height).toBe(info.project.name.endsWith('-phone')?320:280);expect(await stack.getAttribute('data-media-origin')).toBeNull();
 await expect(stack.locator('canvas').first()).toHaveAttribute('data-thumbhash','decoded');
 const clsSupported=await page.evaluate(()=>{const supported=PerformanceObserver.supportedEntryTypes.includes('layout-shift');(window as any).__stackShift=0;if(supported){const observer=new PerformanceObserver(list=>{for(const entry of list.getEntries())(window as any).__stackShift+=(entry as any).value;});observer.observe({type:'layout-shift'});(window as any).__stackObserver=observer;}return supported;});
 release();await expect.poll(()=>stack.locator('[data-media-state=ready]').count()).toBe(4);expect(await stack.boundingBox()).toEqual(before);expect(variants.every(value=>value==='thumb')).toBe(true);
 await page.evaluate(()=>new Promise(requestAnimationFrame));const cls=await page.evaluate(()=>{(window as any).__stackObserver?.disconnect();return (window as any).__stackShift;});if(clsSupported)expect(cls).toBe(0);
 let selected=2;
 if(info.project.name.endsWith('-phone')){
  selected=1;const x=before!.x+before!.width*.7,y=before!.y+before!.height/2;
  await page.mouse.move(x,y);await page.mouse.down();await page.mouse.move(x-before!.width*.55,y,{steps:12});await page.mouse.up();await expect(stack).toHaveAttribute('data-stack-index','1');await expect(stack).toHaveAttribute('data-stack-motion','settled');await expect(page.getByRole('dialog',{name:'Media viewer'})).toHaveCount(0);
 }else{
  const boxes=await cells.evaluateAll(elements=>elements.map(element=>{const r=element.getBoundingClientRect();return {x:r.x,y:r.y,width:r.width,height:r.height};}));
  expect(boxes[1].x).toBeGreaterThan(boxes[0].x+boxes[0].width);expect(boxes[2].y).toBeGreaterThan(boxes[0].y+boxes[0].height);expect(boxes[0].y).toBe(boxes[1].y);expect(boxes[2].x).toBe(boxes[0].x);
 }
 expect(await stack.boundingBox()).toEqual(before);await page.screenshot({path:`../lab/evidence/private/web/media-stack-${info.project.name}.png`});
 const cell=cells.nth(selected),button=cell.locator('.media-open'),attachment=await cell.getAttribute('data-media-origin');expect(attachment).toBe(await cell.locator('.media-image').getAttribute('data-media-id'));
 const source=await cell.boundingBox();await button.click();const viewer=page.getByRole('dialog',{name:'Media viewer'});await expect(viewer).toHaveAttribute('data-media-message',ids[`demo-message-00-${String(42+selected).padStart(6,'0')}`]);
 await expect(viewer.locator('.media-viewer-stage .media-image')).toHaveAttribute('data-media-id',attachment!);await expect(cell).toHaveCSS('visibility','hidden');await expect(cells.nth(selected===0?1:0)).toHaveCSS('visibility','visible');expect(await cell.boundingBox()).toEqual(source);
 await page.keyboard.press('Escape');await expect(viewer).toBeHidden();await expect(button).toBeFocused();await expect(cell).toHaveCSS('visibility','visible');expect(await stack.boundingBox()).toEqual(before);expect(variants).toContain('display');
 const html=await readFile('dist/index.html','utf8'),entry=/<script[^>]+src="([^"]+)"/u.exec(html)![1],bytes=await readFile('dist'+entry);expect(bytes.toString()).not.toContain('media-stack-cell');expect(gzipSync(bytes).byteLength).toBeLessThanOrEqual(60000);
 await mkdir('../lab/evidence/web/metrics',{recursive:true});await writeFile(`../lab/evidence/web/metrics/WEB-MSG-MEDIA-stack-${info.project.name}.json`,JSON.stringify({reserved_before:before,reserved_after:await stack.boundingBox(),decode_geometry_shift_px:0,cls_supported:clsSupported,cls:clsSupported?cls:null,entry_chunk:entry,entry_bytes:bytes.byteLength,entry_gzip_bytes:gzipSync(bytes).byteLength,entry_gzip9_bytes:gzipSync(bytes,{level:9}).byteLength,entry_budget_gzip_bytes:60000,stack_code_in_entry:false},null,2)+'\n');
});
test('WEB-MSG-MEDIA-STACK Reduce Motion and keyboard navigation settle without changing reserved geometry',async({page,context,request},info)=>{
 const ids=await setup(context,request);await page.emulateMedia({reducedMotion:'reduce'});await page.goto(path(ids));const stack=page.getByRole('group',{name:'Photos'});await expect(stack).toBeVisible();
 const cells=stack.locator('.media-stack-cell'),before=await stack.boundingBox(),first=await cells.first().boundingBox();await cells.first().locator('.media-open').focus();await page.keyboard.press('ArrowRight');await page.evaluate(()=>new Promise(requestAnimationFrame));
 await expect(stack).toHaveAttribute('data-stack-index','1');await expect(stack).toHaveAttribute('data-stack-motion','settled');await expect(cells.nth(1).locator('.media-open')).toBeFocused();expect(await stack.boundingBox()).toEqual(before);
 if(info.project.name.endsWith('-phone'))expect(await cells.nth(1).boundingBox()).toEqual(first);
 await page.keyboard.press('Home');await expect(stack).toHaveAttribute('data-stack-index','0');await page.keyboard.press('ArrowLeft');await expect(stack).toHaveAttribute('data-stack-index','0');await page.keyboard.press('End');await expect(stack).toHaveAttribute('data-stack-index','3');await page.keyboard.press('ArrowRight');await expect(stack).toHaveAttribute('data-stack-index','3');
});
