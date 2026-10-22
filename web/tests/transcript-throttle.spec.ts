import {test,expect} from '@playwright/test';
import {prepareSession,control} from './session';
type Fixtures=Record<string,string>;


test('WEB-MSG-READ-THROTTLE transcript recovers after a read burst is throttled',async({page,request})=>{
    await prepareSession(page);const ids:Fixtures=await control(request,'/demo-fixtures');let rateLimited=false;
    page.on('websocket',socket=>socket.on('framereceived',event=>{const frame=JSON.parse(String(event.payload));if(frame.t==='error'&&frame.code==='rate_limited')rateLimited=true;}));
    await page.goto('/messages');await expect(page.locator('.conversation-row').first()).toBeVisible();
    let throttled=false;
    const endpoint=new URL('/web/v1/session',page.url()).href;
    const pressure=(async()=>{
        const end=Date.now()+1800;
        while(Date.now()<end){const responses=await Promise.all(Array.from({length:32},()=>page.request.get(endpoint)));throttled||=responses.some(response=>response.status()===429);await new Promise(resolve=>setTimeout(resolve,10));}
    })();
    await expect.poll(()=>throttled).toBe(true);
    await page.locator('.conversation-row[data-chat="'+ids['demo-chat-01']+'"]').click();
    await pressure;await expect.poll(()=>rateLimited).toBe(true);
    await expect(page.locator('.messages-transcript')).toHaveAttribute('aria-busy','false');
    await expect(page.locator('.bubble.sent.service-green').first()).toBeVisible();
});
