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
