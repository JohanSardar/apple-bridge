import { test, expect } from '@playwright/test';
import { bootstrap, control } from './session';
import { mkdir, writeFile } from 'node:fs/promises';
test('WEB-MSG-CHATS-STORE canceled subscriptions leave other live views intact',async({page,context,request},info)=>{
    await bootstrap(context,request);await control(request,'/list-fixture');await page.goto('/__fixtures/live');
    await expect.poll(async()=>Number(await page.getByLabel('Live count',{exact:true}).textContent())).toBeGreaterThanOrEqual(540);
    const frames=await page.evaluate(()=>new Promise<any[]>((resolve,reject)=>{
        const seen:any[]=[],socket=new WebSocket(new URL('/web/v1/live',location.href).href.replace(/^http/,'ws'),'web.1');
        const timer=setTimeout(()=>{socket.close();reject(new Error('Cancellation probe did not complete'));},5000);
        socket.onopen=()=>socket.send(JSON.stringify({t:'hello',v:1}));
        socket.onmessage=event=>{const frame=JSON.parse(event.data);seen.push(frame);if(frame.t==='welcome')socket.send(JSON.stringify({t:'sub',id:'retained',view:'chats',args:{filter:'all',limit:200}}));if(frame.t==='snap'&&frame.sub==='retained'){for(let index=0;index<4;index++){const id='canceled-'+index;socket.send(JSON.stringify({t:'sub',id,view:'chats',args:{filter:'known',limit:1}}));socket.send(JSON.stringify({t:'unsub',id}));}setTimeout(()=>{clearTimeout(timer);(window as any).__cancelProbe={socket,seen};resolve(seen);},500);}};
    }));
    await mkdir('../lab/evidence/private/web/reconnect',{recursive:true});await writeFile('../lab/evidence/private/web/reconnect/cancellation-'+info.project.name+'.json',JSON.stringify(frames,null,2));
    expect(frames.filter(frame=>frame.t==='error')).toEqual([]);
    await control(request,'/list-late-incoming');
    await expect.poll(()=>page.evaluate(()=>(window as any).__cancelProbe.seen.some((frame:any)=>frame.t==='delta'&&frame.sub==='retained'))).toBe(true);
    expect(await page.evaluate(()=>(window as any).__cancelProbe.socket.readyState)).toBe(1);
    await page.evaluate(()=>(window as any).__cancelProbe.socket.send(JSON.stringify({t:'sub',id:'invalid-active',view:'unsupported',args:{}})));
    await expect.poll(()=>page.evaluate(()=>(window as any).__cancelProbe.seen.some((frame:any)=>frame.t==='error'&&frame.code==='unsupported'))).toBe(true);
    await page.evaluate(()=>(window as any).__cancelProbe.socket.close());
});
