import {test,expect} from '@playwright/test';
import {prepareSession,control} from './session';
type Fixtures=Record<string,string>;

test('WEB-MSG-READ-THROTTLE transcript recovers after a read burst is throttled',async({page,request})=>{
    await prepareSession(page);const ids:Fixtures=await control(request,'/demo-fixtures');let rateLimited=false;
    page.on('websocket',socket=>{if(new URL(socket.url()).searchParams.has('read-pressure'))return;socket.on('framereceived',event=>{const frame=JSON.parse(String(event.payload));if(frame.t==='error'&&frame.code==='rate_limited')rateLimited=true;});});
    await page.goto('/messages');await expect(page.locator('.conversation-row').first()).toBeVisible();
    await page.evaluate(()=>new Promise<void>((resolve,reject)=>{
        const socket=new WebSocket(new URL('/web/v1/live?read-pressure=1',location.href).href.replace(/^http/,'ws'),'web.1');let timer:ReturnType<typeof setInterval>|undefined;
        (window as any).__stopReadPressure=()=>{clearInterval(timer);socket.close();};
        socket.onopen=()=>socket.send(JSON.stringify({t:'hello',v:1}));
        socket.onerror=()=>reject(new Error('Read pressure socket failed'));
        socket.onmessage=event=>{const frame=JSON.parse(event.data);if(frame.t==='welcome')timer=setInterval(()=>{if(socket.readyState===WebSocket.OPEN)for(let index=0;index<16;index++)socket.send(JSON.stringify({t:'ping'}));},50);if(frame.t==='error'&&frame.code==='rate_limited')resolve();};
    }));
    try{
        await expect.poll(async()=>(await page.request.get(new URL('/web/v1/session',page.url()).href)).status()).toBe(429);
        await page.locator('.conversation-row[data-chat="'+ids['demo-chat-01']+'"]').click();
        await expect.poll(()=>rateLimited).toBe(true);
    }finally{await page.evaluate(()=>(window as any).__stopReadPressure());}
    await expect(page.locator('.messages-transcript')).toHaveAttribute('aria-busy','false');
    await expect(page.locator('.bubble.sent.service-green').first()).toBeVisible();
});
