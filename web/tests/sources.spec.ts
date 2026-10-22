import { test, expect, type BrowserContext } from '@playwright/test';
import { bootstrap, control } from './session';
let trusted: Parameters<BrowserContext['addCookies']>[0] = [];
test.beforeAll(async ({request}) => { trusted = [(await control(request,'/session')).cookie]; });
test('WEB-SOURCES-STATUS transient discovery failure retries on next connected transition with backoff',async({page,context})=>{
    await context.addCookies(trusted);
    await page.addInitScript(()=>{
        (window as any).__sourceAttempts=[];
        const fetch=window.fetch.bind(window);
        window.fetch=(...args)=>{
            if(typeof args[0]==='string'&&new URL(args[0],location.href).pathname==='/web/v1/sources')
                (window as any).__sourceAttempts.push(performance.now());
            return fetch(...args);
        };
    });
    const attempts=()=>page.evaluate(()=> (window as any).__sourceAttempts as number[]);
    let injected=false,firstServerSequence=0;
    const sourceResponses:{status:number;worker:boolean;sequence:number;cache:string}[]=[];
    page.on('response',response=>{if(new URL(response.url()).pathname==='/web/v1/sources')sourceResponses.push({status:response.status(),worker:response.fromServiceWorker(),sequence:Number(response.headers()['x-bridge-source-sequence']),cache:response.headers()['cache-control']});});
    await page.route('**/web/v1/sources',async route=>{
        if(!injected){injected=true;const response=await route.fetch();expect(response.status()).toBe(200);expect(response.headers()['cache-control']).toBe('no-store');firstServerSequence=Number(response.headers()['x-bridge-source-sequence']);await route.fulfill({response,status:503,json:{error:{code:'unavailable'}}});}
        else await route.continue();
    });
    await page.goto('/__fixtures/sources');
    await expect.poll(async()=>(await attempts()).length).toBeGreaterThanOrEqual(1);
    await expect.poll(async()=>await page.getByRole('button',{name:'React',exact:true}).isVisible()||await page.getByLabel('Reaction proof').textContent()==='Couldn’t check available actions').toBe(true);
    await page.getByRole('button',{name:'Reconnect source proofs'}).click();
    await expect.poll(async()=>(await attempts()).length).toBe(2);
    const times=await attempts();
    expect(times[1]-times[0]).toBeGreaterThanOrEqual(250);
    expect(firstServerSequence).toBeGreaterThan(0);
    await expect.poll(()=>sourceResponses.some(response=>response.status===200&&response.sequence>firstServerSequence)).toBe(true);
    expect(sourceResponses.every(response=>response.cache==='no-store'&&!response.worker)).toBe(true);
    await expect(page.getByRole('button',{name:'React',exact:true})).toBeVisible();
});
test('WEB-SOURCES-STATUS live proofs gate actions and reconnect never reuses stale proof',async({page,context,request,browser})=>{
    expect((await page.request.get('/web/v1/sources')).status()).toBe(401);
    await context.addCookies(trusted);
    let hold=false,welcomes=0,commands=0,connections=0;
    const released:(()=>void)[]=[];
    await page.routeWebSocket('**/web/v1/live',socket=>{
        const server=socket.connectToServer(),sources=new Set<string>(),generation=++connections;
        socket.onMessage(data=>{
            const frame=JSON.parse(String(data));
            if(frame.t==='cmd')commands++;
            if(frame.t==='sub'&&frame.args?.collection==='system.source')sources.add(frame.id);
            server.send(data);
        });
        server.onMessage(data=>{
            const frame=JSON.parse(String(data));
            if(frame.t==='welcome')welcomes++;
            if(hold&&sources.has(frame.sub))released.push(()=>{if(generation===connections)socket.send(data);});
            else socket.send(data);
        });
    });
    await page.goto('/__fixtures/sources');
    const react=page.getByRole('button',{name:'React',exact:true});
    await expect(react).toBeVisible();
    await control(request,'/capability-unproven');
    await expect(react).toHaveCount(0);
    await expect(page.getByLabel('Reaction proof')).toContainText('capability_unproven');
    await control(request,'/capability-proven-empty-reason');
    await expect(react).toBeVisible();
    await expect(page.getByLabel('Reaction proof')).toHaveText('');
    hold=true;
    await control(request,'/capability-unproven');
    await page.getByRole('button',{name:'Reconnect source proofs'}).click();
    await expect.poll(()=>welcomes).toBe(2);
    await expect(page.getByLabel('Source connection')).toHaveText('connected');
    await expect(react).toHaveCount(0);
    await expect(page.getByLabel('Reaction proof')).toHaveText('Waiting for capability proof');
    hold=false;
    released.splice(0).forEach(send=>send());
    await expect(page.getByLabel('Reaction proof')).toContainText('capability_unproven');
    await expect(react).toHaveCount(0);
    expect(commands).toBe(0);
    const publicContext=await browser.newContext({baseURL:process.env.AB_WEB_URL});
    try{
        await bootstrap(publicContext,request,'public');
        const publicPage=await publicContext.newPage();
        await publicPage.goto('/__fixtures/sources');
        await expect(publicPage.getByLabel('Reaction proof')).toContainText('capability_unproven');
        await expect(publicPage.getByRole('button',{name:'React',exact:true})).toHaveCount(0);
        await control(request,'/capability-proven');
        await expect.poll(async()=>{
            const result=await (await publicPage.request.get('/web/v1/sources')).json();
            return result.records.find((row:any)=>row.data.source==='messages')?.data.coverage.operations.find((proof:any)=>proof.operation==='messages.react')?.proven;
        }).toBe(true);
        await expect(publicPage.getByRole('button',{name:'React',exact:true})).toBeVisible();
        expect(await publicPage.evaluate(()=>localStorage.length)).toBe(0);
        expect(await publicPage.evaluate(async()=> (await indexedDB.databases()).length)).toBe(0);
    }finally{await publicContext.close();}
});
test('WEB-SOURCES-STATUS JSON proofs bypass the service worker while trusted thumbnails remain cached',async({page,context,request})=>{
    await context.addCookies(trusted);
    const ids=await control(request,'/demo-fixtures');
    await page.goto('/__fixtures/media?chat='+encodeURIComponent(ids['message-chat']));
    await expect(page.locator('[data-media-state="ready"]').first()).toBeVisible();
    await expect.poll(()=>page.evaluate(()=>Boolean(navigator.serviceWorker.controller))).toBe(true);
    expect(await page.evaluate(async()=>{
        const registration=await navigator.serviceWorker.ready;
        return new Promise<boolean>(resolve=>{const channel=new MessageChannel();channel.port1.onmessage=event=>resolve(event.data.ok===true);registration.active!.postMessage({t:'trusted'},[channel.port2]);});
    })).toBe(true);
    const thumb=await page.locator('[data-media-id] img').first().getAttribute('src');
    expect(thumb).toContain('variant=thumb');
    const thumbURL=new URL(thumb!,process.env.AB_WEB_URL).href;
    const thumbResponse=page.waitForResponse(response=>response.url()===thumbURL);
    expect(await page.evaluate(url=>fetch(url,{cache:'reload'}).then(response=>response.ok),thumbURL)).toBe(true);
    expect((await thumbResponse).fromServiceWorker()).toBe(true);
    await expect.poll(()=>page.evaluate(async url=>Boolean(await(await caches.open('ab.thumbs')).match(url)),thumbURL),{timeout:2000}).toBe(true);
    const sourceResponse=page.waitForResponse(response=>new URL(response.url()).pathname==='/web/v1/sources');
    expect(await page.evaluate(()=>fetch('/web/v1/sources',{cache:'no-store'}).then(response=>response.ok))).toBe(true);
    const fresh=await sourceResponse;
    expect(fresh.fromServiceWorker()).toBe(false);
    expect(fresh.headers()['cache-control']).toBe('no-store');
    expect(Number(fresh.headers()['x-bridge-source-sequence'])).toBeGreaterThan(0);
    const navigation=await page.goto('/web/v1/sources');
    expect(navigation!.fromServiceWorker()).toBe(false);
    expect(navigation!.headers()['content-type']).toContain('application/json');
    expect(navigation!.headers()['cache-control']).toBe('no-store');
});
test.describe('late capability discovery',()=>{
    test.use({serviceWorkers:'block'});
test('WEB-SOURCES-STATUS LATE-FAILURE repeated late discovery failures heal without reconnect or command dispatch',async({page,context},testInfo)=>{
    await context.addCookies(trusted);
    let welcomes=0,commands=0,requests=0;const sequences:number[]=[];
    await page.addInitScript(()=>{
        (window as any).__lateSourceStarts=[];const fetch=window.fetch.bind(window);
        window.fetch=(...args)=>{if(typeof args[0]==='string'&&new URL(args[0],location.href).pathname==='/web/v1/sources')(window as any).__lateSourceStarts.push(performance.now());return fetch(...args);};
    });
    await page.routeWebSocket('**/web/v1/live',socket=>{
        const server=socket.connectToServer();socket.onMessage(data=>{if(JSON.parse(String(data)).t==='cmd')commands++;server.send(data);});
        server.onMessage(data=>{if(JSON.parse(String(data)).t==='welcome')welcomes++;socket.send(data);});
    });
    await page.route('**/web/v1/sources',async route=>{
        const index=++requests,response=await route.fetch();expect(response.status()).toBe(200);expect(response.headers()['cache-control']).toBe('no-store');sequences.push(Number(response.headers()['x-bridge-source-sequence']));
        if(index<=2){await expect(page.getByLabel('Source connection')).toHaveText('connected');expect(welcomes).toBe(1);await route.fulfill({response,status:503,json:{error:{code:'unavailable'}}});}
        else await route.fulfill({response});
    });
    await page.goto('/__fixtures/sources');
    await expect(page.getByLabel('Source connection')).toHaveText('connected');
    await expect(page.getByLabel('Reaction proof')).toHaveText('Couldn’t check available actions');
    await expect(page.getByRole('button',{name:'React',exact:true})).toBeVisible();
    expect(requests).toBe(3);expect(welcomes).toBe(1);expect(commands).toBe(0);
    expect(sequences).toHaveLength(3);expect(sequences[1]).toBeGreaterThan(sequences[0]);expect(sequences[2]).toBeGreaterThan(sequences[1]);
    const starts=await page.evaluate(()=>(window as any).__lateSourceStarts as number[]);
    expect(starts[1]-starts[0]).toBeGreaterThanOrEqual(1000);expect(starts[2]-starts[1]).toBeGreaterThanOrEqual(2000);
    await testInfo.attach('source-retry.json',{body:JSON.stringify({requests,welcomes,commands,server_sequences:sequences,delays_ms:[starts[1]-starts[0],starts[2]-starts[1]]}),contentType:'application/json'});
});
});
