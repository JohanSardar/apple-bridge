import { test, expect } from '@playwright/test';
import { bootstrap, control } from './session';
test('WEB-MSG-OPTIMISTIC first-frame echo swaps only on exact captured identity and never redispatches',async({page,context,request})=>{
    await bootstrap(context,request);
    const ids=await control(request,'/demo-fixtures');
    const commands:string[]=[],statuses:any[]=[],captured:any[]=[];
    let holdStatus=true,holdCaptured=true,welcomes=0;
    const releaseStatus:(()=>void)[]=[],releaseCaptured:(()=>void)[]=[];
    await page.routeWebSocket('**/web/v1/live',socket=>{
        const server=socket.connectToServer(),threads=new Set<string>();
        socket.onMessage(data=>{
            const frame=JSON.parse(String(data));
            if(frame.t==='cmd')commands.push(frame.id);
            if(frame.t==='sub'&&frame.view==='thread')threads.add(frame.id);
            server.send(data);
        });
        server.onMessage(data=>{
            const frame=JSON.parse(String(data));
            if(frame.t==='welcome')welcomes++;
            if(frame.t==='cmd_status'){
                statuses.push(frame);
                if(holdStatus){releaseStatus.push(()=>socket.send(data));return;}
            }
            if(frame.t==='delta'&&threads.has(frame.sub)){
                captured.push(frame);
                if(holdCaptured){releaseCaptured.push(()=>socket.send(data));return;}
            }
            socket.send(data);
        });
    });
    await page.addInitScript(()=>{
        (window as any).__composeFirstFrames=[];
        window.addEventListener('click',event=>{
            if(!(event.target as Element).closest('[data-compose-send]'))return;
            requestAnimationFrame(()=>{(window as any).__composeFirstFrames.push([...document.querySelectorAll('[data-local-echo]')].map(row=>row.getAttribute('data-record-id')));});
        },{capture:true});
    });
    await page.goto('/__fixtures/commands?chat='+encodeURIComponent(ids['message-chat']));
    const send=page.getByRole('button',{name:'Send fixture message',exact:true});
    await expect(send).toBeEnabled();
    await page.getByRole('textbox',{name:'Fixture message',exact:true}).fill('[ab-test compose] first frame');
    await send.click();
    await expect.poll(()=>commands.length).toBe(1);
    const commandID=commands[0];
    expect(commandID).toMatch(/^[0-9A-HJKMNP-TV-Z]{26}$/);
    await expect.poll(()=>page.evaluate(()=>(window as any).__composeFirstFrames.length)).toBe(1);
    expect(await page.evaluate(()=>(window as any).__composeFirstFrames[0])).toEqual(['command:'+commandID]);
    await expect(page.locator('[data-local-echo]')).toHaveCount(1);
    const echoTime=await page.locator('[data-local-echo]').getAttribute('data-command-sent-at');
    expect(echoTime).toMatch(/^\d{4}-\d{2}-\d{2}T/);
    expect(JSON.parse((await page.locator('[data-local-echo]').getAttribute('data-parts'))!)).toEqual([{index:0,kind:'text',text:'[ab-test compose] first frame'}]);
    await expect.poll(()=>captured.length).toBeGreaterThan(0);
    await expect.poll(()=>statuses.some(frame=>frame.command?.observed_record_ids?.length)).toBe(true);
    holdCaptured=false;
    releaseCaptured.splice(0).forEach(release=>release());
    await page.evaluate(()=>new Promise(requestAnimationFrame));
    await expect(page.locator('[data-local-echo]')).toHaveCount(1);
    const final=statuses.find(frame=>frame.command?.observed_record_ids?.length);
    const recordID=final.command.observed_record_ids[0];
    await expect(page.locator('[data-record-id="'+recordID+'"]')).toHaveCount(0);
    holdStatus=false;
    releaseStatus.splice(0).forEach(release=>release());
    await expect(page.locator('[data-local-echo]')).toHaveCount(0);
    await expect(page.locator('[data-record-id="'+recordID+'"][data-command-id="'+commandID+'"]')).toHaveCount(1);
    await expect(page.locator('[data-record-id="'+recordID+'"]')).toHaveAttribute('data-command-sent-at',echoTime!);
    const raw=captured.flatMap(frame=>frame.changes.flatMap((change:any)=>change.row?[change.row]:change.record?[change.record]:change.data?.items??[])).find(item=>item.id===recordID);
    expect(await page.locator('[data-record-id="'+recordID+'"]').getAttribute('data-native-sent-at')).toBe(raw.data.sent_at);
    await expect(page.getByLabel('Captured metadata untouched')).toHaveText('true');
    await page.getByRole('button',{name:'Reconnect command fixture'}).click();
    await expect.poll(()=>welcomes).toBe(2);
    await expect(page.getByLabel('Fixture connection')).toHaveText('connected');
    await expect(page.locator('[data-record-id="'+recordID+'"][data-command-id="'+commandID+'"]')).toHaveCount(1);
    expect(commands).toEqual([commandID]);
});
test('WEB-MSG-OPTIMISTIC lost response preserves the command ID and reconciles through reads only',async({page,context,request})=>{
    await bootstrap(context,request);
    const ids=await control(request,'/demo-fixtures');
    const commands:string[]=[],lookups:string[]=[];
    let generation=0,recordID='',releaseWelcome:(()=>void)|undefined;
    const seenRecords=new Set<string>();
    await page.routeWebSocket('**/web/v1/live',socket=>{
        const server=socket.connectToServer(),connection=++generation;
        socket.onMessage(data=>{
            const frame=JSON.parse(String(data));
            if(frame.t==='cmd')commands.push(frame.id);
            if(frame.t==='sub'&&frame.args?.collection==='system.command')lookups.push(frame.args.id);
            server.send(data);
        });
        server.onMessage(data=>{
            const frame=JSON.parse(String(data));
            if(connection===1&&(frame.t==='ack'||frame.t==='cmd_status')){
                if(frame.command?.observed_record_ids?.length){recordID=frame.command.observed_record_ids[0];if(seenRecords.has(recordID))void socket.close({code:4000,reason:'fixture_lost_response'});}
                return;
            }
            if(connection===2&&frame.t==='welcome'){releaseWelcome=()=>socket.send(data);return;}
            if(connection===1&&frame.t==='delta')for(const change of frame.changes){for(const item of change.row?[change.row]:change.data?.items??[])seenRecords.add(item.id);}
            socket.send(data);
            if(connection===1&&recordID&&seenRecords.has(recordID))void socket.close({code:4000,reason:'fixture_lost_response'});
        });
    });
    await page.goto('/__fixtures/commands?chat='+encodeURIComponent(ids['message-chat']));
    await expect(page.getByRole('button',{name:'Send fixture message',exact:true})).toBeEnabled();
    await page.getByRole('textbox',{name:'Fixture message',exact:true}).fill('[ab-test compose] lost response');
    await page.getByRole('button',{name:'Send fixture message',exact:true}).click();
    await expect.poll(()=>commands.length).toBe(1);
    const commandID=commands[0];
    await expect(page.getByLabel('Fixture command status')).toHaveText('Not sure if delivered');
    await expect(page.getByLabel('Fixture command ID')).toHaveText(commandID);
    await expect(page.getByLabel('Fixture ambiguity origin')).toHaveText('local transport');
    await expect.poll(()=>Boolean(releaseWelcome)).toBe(true);
    expect(recordID).not.toBe('');
    await expect(page.getByLabel('Projection timeout count')).toHaveText('1');
    await expect(page.locator('[data-local-echo]')).toHaveCount(0);
    await expect(page.locator('[data-record-id="'+recordID+'"]')).toHaveCount(1);
    await expect(page.getByLabel('Fixture command status')).toHaveText('Not sure if delivered');
    releaseWelcome!();
    await expect.poll(()=>lookups).toContain(commandID);
    await expect(page.locator('[data-record-id="'+recordID+'"][data-command-id="'+commandID+'"]')).toHaveCount(1);
    await expect(page.locator('[data-local-echo]')).toHaveCount(0);
    expect(commands).toEqual([commandID]);
});
