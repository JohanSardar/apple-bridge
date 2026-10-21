import { test, expect } from '@playwright/test';
import { bootstrap, control } from './session';
test('WEB-SOURCES-STATUS live proofs gate actions and reconnect never reuses stale proof',async({page,context,request,browser})=>{
    expect((await page.request.get('/web/v1/sources')).status()).toBe(401);
    await bootstrap(context,request);
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
    await control(request,'/capability-proven');
    await expect(react).toBeVisible();
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
