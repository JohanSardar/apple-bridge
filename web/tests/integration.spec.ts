import { test, expect, type Page, type APIRequestContext } from '@playwright/test';
import { mkdir, writeFile } from 'node:fs/promises';
import { control, bootstrap } from './session';
async function virtualKey(page:Page){const cdp=await page.context().newCDPSession(page);await cdp.send('WebAuthn.enable');const {authenticatorId}=await cdp.send('WebAuthn.addVirtualAuthenticator',{options:{protocol:'ctap2',transport:'internal',hasResidentKey:true,hasUserVerification:true,isUserVerified:true,automaticPresenceSimulation:true}});return {cdp,authenticatorId};}
async function enroll(page:Page,request:APIRequestContext){const key=await virtualKey(page);const {url}=await control(request,'/enroll');await page.goto(url);await page.getByRole('button',{name:'Continue',exact:true}).click();await expect(page.getByText('No Messages',{exact:true})).toBeVisible();return {key,url};}
test('WEB-SERVE-EMBED binary serves the app, routes, hashed assets and security headers',async({page,request})=>{
  const response=await request.get('/messages/deep-link');expect(response.status()).toBe(200);expect(response.headers()['cache-control']).toBe('no-store');expect(response.headers()['content-security-policy']).toContain("script-src 'self'");
  const html=await response.text();const asset=/src="(\/assets\/[^"]+\.js)"/.exec(html)![1];const script=await request.get(asset);expect(script.status()).toBe(200);expect(script.headers()['cache-control']).toBe('public, max-age=31536000, immutable');
  for(const path of ['/sw.js','/manifest.webmanifest'])expect((await request.get(path)).headers()['cache-control']).toBe('no-cache');
  await page.goto('/messages/deep-link');await expect(page.getByRole('heading',{name:'Sign in with a passkey'})).toBeVisible();
});
test('WEB-AUTH-ENROLL browser passkey enrollment, token single use and expiry',async({page,request})=>{
  const {url}=await enroll(page,request);expect(page.url()).not.toContain('#');
  const session=await page.request.get('/web/v1/session').then(response=>response.json());expect(session.kind).toBe('trusted');expect(session.cache).toBe('persistent');
  const token=new URL(url).hash.slice(1);const reused=await page.request.post('/web/v1/enroll/options',{headers:{Origin:new URL(url).origin},data:{token}});expect(reused.status()).toBe(401);
  const expired=await control(request,'/enroll?expired=1');const rejected=await page.request.post('/web/v1/enroll/options',{headers:{Origin:new URL(url).origin},data:{token:new URL(expired.url).hash.slice(1)}});expect(rejected.status()).toBe(401);
});
test('WEB-AUTH-LOGIN discoverable browser passkey sign-in chooses trusted or public cache',async({page,request,context})=>{
  await enroll(page,request);
  for(const kind of ['trusted','public']){
    await page.goto('/settings/devices');await page.getByRole('button',{name:'Sign out',exact:true}).click();await expect(page.getByRole('heading',{name:'Sign in with a passkey'})).toBeVisible();
    await page.getByRole('radio',{name:kind==='trusted'?'Yes, trust this computer':'No, this is a public computer',exact:false}).check();
    await page.getByRole('button',{name:'Sign in with a passkey',exact:true}).click();await expect(page.getByText('No Messages',{exact:true})).toBeVisible();
    const session=await page.request.get('/web/v1/session').then(response=>response.json());expect(session.kind).toBe(kind);expect(session.cache).toBe(kind==='trusted'?'persistent':'memory');
    const cookie=(await context.cookies()).find(cookie=>cookie.name==='ab_session_local')!;expect(cookie.httpOnly&&!cookie.secure&&cookie.sameSite==='Strict').toBe(true);
    if(kind==='public'){
      expect(cookie.expires).toBe(-1);
      await expect.poll(()=>page.evaluate(async()=>({db:await indexedDB.databases(),cache:await caches.keys(),workers:(await navigator.serviceWorker.getRegistrations()).length,local:Object.keys(localStorage),session:Object.keys(sessionStorage)}))).toEqual({db:[],cache:[],workers:0,local:[],session:[]});
    }else expect(cookie.expires*1000).toBeGreaterThan(Date.now()+29*86400000);
  }
});
test('WEB-AUTH-REVOKE another session closes within one second, wipes cache, and web lock revokes all',async({page,request,context,browser},info)=>{
  const first=await bootstrap(context,request);
  await page.goto('/__fixtures/live');await expect(page.getByRole('status', {name:'Connection'})).toHaveText('connected');
  await expect.poll(()=>page.evaluate(async()=> (await indexedDB.databases()).length)).toBeGreaterThan(0);
  const other=await browser.newContext({baseURL:process.env.AB_WEB_URL});await bootstrap(other,request);await expect(page.getByRole('alert')).toContainText('A new device signed in.');const controller=await other.newPage();await controller.goto('/settings/devices');
  const opened=page.waitForEvent('websocket');
  await page.getByRole('button',{name:'Reconnect live view'}).click();
  const socket=await opened;
  const closed=new Promise<number>(resolve=>socket.on('close',()=>resolve(Date.now())));
  await expect(page.getByRole('status', {name:'Connection'})).toHaveText('connected');
  const start=Date.now();const response=await controller.request.delete('/web/v1/devices/'+encodeURIComponent(first.device_id),{headers:{Origin:process.env.AB_WEB_URL!}});expect(response.ok()).toBe(true);
  await expect(page.getByRole('heading',{name:'Sign in with a passkey'})).toBeVisible();const close=await Promise.race([closed,new Promise<number>((_,reject)=>setTimeout(()=>reject(new Error('Socket did not close')),1000))]);expect(close-start).toBeGreaterThanOrEqual(0);expect(close-start).toBeLessThanOrEqual(1000);
  await expect.poll(()=>page.evaluate(async()=>({db:await indexedDB.databases(),cache:await caches.keys()}))).toEqual({db:[],cache:[]});
  await control(request,'/lock');await expect(controller.getByRole('heading',{name:'Sign in with a passkey'})).toBeVisible();await other.close();
  await mkdir('../lab/evidence/web/metrics',{recursive:true});await writeFile(`../lab/evidence/web/metrics/WEB-AUTH-REVOKE-${info.project.name}.json`,JSON.stringify({samples:1,p50_ms:close-start,p95_ms:close-start,max_ms:close-start})+'\n');
});
test('WEB-LIVE-DELTA sim capture is visible in the browser within 500 ms p95',async({page,context,request},info)=>{
  await bootstrap(context,request);await page.goto('/__fixtures/live');await expect(page.getByRole('status', {name:'Connection'})).toHaveText('connected');await expect(page.getByLabel('Latest message',{exact:true})).not.toHaveText('');
  const samples:number[]=[];
  for(let i=0;i<20;i++){const before=Number(await page.getByLabel('Unread count',{exact:true}).textContent());const result=await control(request,'/incoming');await expect(page.getByLabel('Unread count',{exact:true})).toHaveText(String(before+1));const observed=Number(await page.getByLabel('View observed at',{exact:true}).textContent());samples.push(observed-result.started_at_ms);}
  const sorted=[...samples].sort((a,b)=>a-b);const metrics={samples:samples.length,p50_ms:sorted[9],p95_ms:sorted[18],max_ms:sorted[19],endpoint:'sim capture start to browser live-view render'};
  expect(metrics.p95_ms).toBeLessThanOrEqual(500);expect(Math.min(...samples)).toBeGreaterThanOrEqual(0);
  await mkdir('../lab/evidence/web/metrics',{recursive:true});await writeFile(`../lab/evidence/web/metrics/WEB-LIVE-DELTA-${info.project.name}.json`,JSON.stringify(metrics,null,2)+'\n');
});
test('WEB-LIVE-RESUME resubscribes with its cursor and converges after an actual socket close',async({page,context,request})=>{
  await bootstrap(context,request);const frames:any[]=[];page.on('websocket',socket=>socket.on('framesent',event=>{try{frames.push(JSON.parse(String(event.payload)));}catch{}}));
  await page.goto('/__fixtures/live');await expect(page.getByRole('status', {name:'Connection'})).toHaveText('connected');await expect(page.getByLabel('Latest message',{exact:true})).not.toHaveText('');const before=Number(await page.getByLabel('Unread count',{exact:true}).textContent());
  await page.getByRole('button',{name:'Reconnect live view'}).click();await control(request,'/incoming');await expect(page.getByRole('status', {name:'Connection'})).toHaveText('connected');await expect(page.getByLabel('Unread count',{exact:true})).toHaveText(String(before+1));expect(frames.some(frame=>frame.t==='sub'&&frame.cursor)).toBe(true);
  const previous=await page.getByLabel('Latest message',{exact:true}).textContent();await control(request,'/incoming');await expect(page.getByLabel('Unread count',{exact:true})).toHaveText(String(before+2));await expect(page.getByLabel('Latest message',{exact:true})).not.toHaveText(previous!);
});
