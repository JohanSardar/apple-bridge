import {test,expect} from '@playwright/test';
import {prepareSession,control} from './session';
test('WEB-MSG-FONTS Apple transcript uses native glyph fallback without downloading Inter',async({page,request})=>{
    await prepareSession(page);const ids:Record<string,string>=await control(request,'/demo-fixtures'),fonts:string[]=[];
    page.on('request',request=>{const path=new URL(request.url()).pathname;if(/InterVariable.*\.woff2$/u.test(path))fonts.push(path);});
    await page.goto('/messages');await expect(page.locator('.app-shell')).toBeVisible();const apple=await page.evaluate(()=>/Mac|iPhone|iPad/u.test(navigator.platform));if(apple)expect(fonts).toEqual([]);
    await page.goto('/messages/'+encodeURIComponent(ids['message-chat'])+'/'+encodeURIComponent(ids['demo-message-00-000058']));await expect(page.locator('.messages-transcript')).toHaveAttribute('aria-busy','false');
    await page.evaluate(async()=>{await document.fonts.ready;await new Promise<void>(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>resolve())));});
    const evidence=await page.evaluate(()=>({platform:navigator.platform,family:getComputedStyle(document.querySelector('.message-text')!).fontFamily,faces:Array.from(document.fonts,face=>({family:face.family,status:face.status}))}));
    await test.info().attach('font-selection',{body:JSON.stringify({...evidence,requests:fonts}),contentType:'application/json'});
    if(apple){expect(fonts).toEqual([]);expect(evidence.family).not.toContain('InterVariable');}else expect(evidence.family).toContain('InterVariable');
});

test('WEB-MSG-FONTS reuses native text measurements when declared web fonts are unused',async({page})=>{
    await page.addInitScript(()=>{
        const measure=CanvasRenderingContext2D.prototype.measureText,counts=new Map<string,number>();(window as any).__nativeMeasureCounts=counts;
        CanvasRenderingContext2D.prototype.measureText=function(value){const key=this.font+'|'+value;counts.set(key,(counts.get(key)??0)+1);return measure.call(this,value);};
        document.addEventListener('DOMContentLoaded',()=>{document.documentElement.style.fontFamily='system-ui';document.fonts.add(new FontFace('UnusedBridgeFont','local(Arial)'));});
    });
    await prepareSession(page);await page.goto('/messages');await page.locator('.conversation-row').first().click();await expect(page.locator('.messages-transcript')).toHaveAttribute('aria-busy','false');
    await page.evaluate(async()=>{await document.fonts.ready;await new Promise<void>(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>resolve())));});
    const counts=await page.evaluate(()=>Object.fromEntries([...((window as any).__nativeMeasureCounts as Map<string,number>)].filter(([key])=>key.endsWith('| '))));expect(Object.keys(counts).length).toBeGreaterThan(0);
    await page.goBack();await page.locator('.conversation-row').first().click();await expect(page.locator('.messages-transcript')).toHaveAttribute('aria-busy','false');
    const repeated=await page.evaluate(keys=>Object.fromEntries(keys.map(key=>[key,(window as any).__nativeMeasureCounts.get(key)])),Object.keys(counts));expect(repeated).toEqual(counts);
});
