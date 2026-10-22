import { test, expect, type BrowserContext } from '@playwright/test';
import { control } from './session';
let sessionCookies: Parameters<BrowserContext['addCookies']>[0] = [];
test.beforeAll(async ({request}) => { sessionCookies = [(await control(request, '/session')).cookie]; });
test.afterEach(async ({page}, info) => {
    if (page.isClosed()) return;
    const counts = await page.evaluate(() => ({ timeout_count: Number(document.querySelector('[aria-label="Projection timeout count"]')?.textContent ?? 0), missing_identity_count: Number(document.querySelector('[aria-label="Missing observed identity count"]')?.textContent ?? 0) }));
    console.log('compose_metric:' + JSON.stringify({test: info.title, status: info.status, ...counts}));
});
test('WEB-MSG-OPTIMISTIC first-frame echo swaps only on exact captured identity and never redispatches',async({page,context,request})=>{
    await context.addCookies(sessionCookies);
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
test('WEB-MSG-OPTIMISTIC AMBIGUITY lost response preserves the command ID and reconciles through reads only',async({page,context,request})=>{
    await context.addCookies(sessionCookies);
    const ids=await control(request,'/demo-fixtures');
    const commands:string[]=[],lookups:string[]=[],lookupConnections:number[]=[];
    let generation=0,recordID='',releaseWelcome:(()=>void)|undefined;
    const seenRecords=new Set<string>();
    await page.routeWebSocket('**/web/v1/live',socket=>{
        const server=socket.connectToServer(),connection=++generation;
        socket.onMessage(data=>{
            const frame=JSON.parse(String(data));
            if(frame.t==='cmd')commands.push(frame.id);
            if(frame.t==='sub'&&frame.args?.collection==='system.command'){lookups.push(frame.args.id);lookupConnections.push(connection);}
            server.send(data);
        });
        server.onMessage(data=>{
            const frame=JSON.parse(String(data));
            if(connection===1&&(frame.t==='ack'||frame.t==='cmd_status')){
                if(frame.command?.observed_record_ids?.length){recordID=frame.command.observed_record_ids[0];if(seenRecords.has(recordID))void socket.close({code:4000,reason:'fixture_lost_response'});}
                return;
            }
            if(connection===2&&frame.t==='welcome'){releaseWelcome=()=>socket.send(data);return;}
            if(connection===2&&frame.t==='snap'&&frame.data.records?.some((record:any)=>record.collection==='system.command')){void socket.close({code:4000,reason:'fixture_lost_status_read'});return;}
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
    await expect(page.getByLabel('Uncertain delivery')).toContainText('1 message may not have been delivered');
    await expect(page.locator('[data-local-echo]')).toHaveCount(0);
    await expect(page.locator('[data-record-id="'+recordID+'"]')).toHaveCount(1);
    await expect(page.getByLabel('Fixture command status')).toHaveText('Not sure if delivered');
    releaseWelcome!();
    await expect.poll(()=>lookups).toContain(commandID);
    await expect(page.locator('[data-record-id="'+recordID+'"][data-command-id="'+commandID+'"]')).toHaveCount(1);
    await expect(page.locator('[data-local-echo]')).toHaveCount(0);
    expect(commands).toEqual([commandID]);
    expect(lookupConnections.filter(connection=>connection===3)).toHaveLength(1);
    await expect(page.getByLabel('Uncertain delivery')).toHaveCount(0);
});
test('WEB-MSG-OPTIMISTIC native outgoing rows render in the first frame without a local echo',async({page,context,request})=>{
    await context.addCookies(sessionCookies);
    const ids=await control(request,'/demo-fixtures');
    await page.addInitScript(()=>{
        (window as any).__nativeFrames=[];
        const Native=window.WebSocket;
        window.WebSocket=class extends Native{
            constructor(url:string|URL,protocols?:string|string[]){super(url,protocols);this.addEventListener('message',event=>{
                const frame=JSON.parse(String(event.data));
                if(frame.t!=='delta')return;
                const outgoing=frame.changes.flatMap((change:any)=>change.row?[change.row]:change.data?.items??[]).filter((item:any)=>item.collection==='messages.message'&&item.data.is_from_me).map((item:any)=>item.id);
                if(outgoing.length)requestAnimationFrame(()=>(window as any).__nativeFrames.push({outgoing,rendered:[...document.querySelectorAll('[data-record-id]')].map(row=>row.getAttribute('data-record-id'))}));
            });}
        };
    });
    await page.goto('/__fixtures/commands?chat='+encodeURIComponent(ids['message-chat']));
    await expect(page.getByRole('button',{name:'Send fixture message',exact:true})).toBeEnabled();
    await expect(page.locator('[data-record-id]').first()).toBeVisible();
    const baseline=await page.locator('[data-record-id]').evaluateAll(rows=>rows.map(row=>row.getAttribute('data-record-id')));
    await control(request,'/incoming');
    await expect.poll(()=>page.evaluate(initial=>(window as any).__nativeFrames.filter((sample:any)=>sample.outgoing.some((id:string)=>!initial.includes(id))),baseline)).not.toEqual([]);
    const samples=await page.evaluate(initial=>(window as any).__nativeFrames.filter((sample:any)=>sample.outgoing.some((id:string)=>!initial.includes(id))),baseline);
    for(const id of samples[0].outgoing.filter((id:string)=>!baseline.includes(id)))expect(samples[0].rendered).toContain(id);
    await expect(page.locator('[data-local-echo]')).toHaveCount(0);
    await expect(page.getByLabel('Projection timeout count')).toHaveText('0');
    console.log('compose: timeout_count=0; local_echo=false; native_first_frame=true');
});
test('WEB-MSG-OPTIMISTIC unrelated outgoing rows release atomically at the ambiguity bound',async({page,context,request})=>{
    await context.addCookies(sessionCookies);
    const ids=await control(request,'/demo-fixtures');
    const captured:any[]=[],statuses:any[]=[],commands:string[]=[];
    const releaseCaptured:(()=>void)[]=[],releaseStatus:(()=>void)[]=[];
    let holdCaptured=true,holdStatus=true;
    await page.routeWebSocket('**/web/v1/live',socket=>{
        const server=socket.connectToServer(),threads=new Set<string>();
        socket.onMessage(data=>{const frame=JSON.parse(String(data));if(frame.t==='cmd')commands.push(frame.id);if(frame.t==='sub'&&frame.view==='thread')threads.add(frame.id);server.send(data);});
        server.onMessage(data=>{
            const frame=JSON.parse(String(data));
            if(frame.t==='cmd_status'){statuses.push(frame);if(holdStatus){releaseStatus.push(()=>socket.send(data));return;}}
            if(frame.t==='delta'&&threads.has(frame.sub)){captured.push(frame);if(holdCaptured){releaseCaptured.push(()=>socket.send(data));return;}}
            socket.send(data);
        });
    });
    await page.goto('/__fixtures/commands?chat='+encodeURIComponent(ids['message-chat']));
    await expect(page.getByRole('button',{name:'Send fixture message',exact:true})).toBeEnabled();
    const baseline=await page.locator('[data-record-id]').evaluateAll(rows=>rows.map(row=>row.getAttribute('data-record-id')));
    await page.getByRole('textbox',{name:'Fixture message',exact:true}).fill('[ab-test compose] ambiguity bound');
    await page.getByRole('button',{name:'Send fixture message',exact:true}).click();
    await expect.poll(()=>statuses.some(frame=>frame.command?.observed_record_ids?.length)).toBe(true);
    const recordID=statuses.find(frame=>frame.command?.observed_record_ids?.length).command.observed_record_ids[0];
    await control(request,'/incoming');
    const rows=()=>captured.flatMap(frame=>frame.changes.flatMap((change:any)=>change.row?[change.row]:change.data?.items??[])).filter(item=>item.collection==='messages.message'&&!baseline.includes(item.id));
    await expect.poll(()=>new Set(rows().map(item=>item.id)).size).toBeGreaterThanOrEqual(3);
    const incoming=rows().find(item=>!item.data.is_from_me).id,native=rows().find(item=>item.data.is_from_me&&item.id!==recordID).id;
    holdCaptured=false;releaseCaptured.splice(0).forEach(release=>release());
    const first=await page.evaluate(()=>new Promise<{ids:(string|null)[];echoes:number}>(resolve=>requestAnimationFrame(()=>resolve({ids:[...document.querySelectorAll('[data-record-id]')].map(row=>row.getAttribute('data-record-id')),echoes:document.querySelectorAll('[data-local-echo]').length}))));
    expect(first.ids).toContain(incoming);expect(first.ids).not.toContain(native);expect(first.ids).not.toContain(recordID);expect(first.echoes).toBe(1);
    await expect(page.getByLabel('Projection timeout count')).toHaveText('1');
    await expect(page.locator('[data-local-echo]')).toHaveCount(0);
    for(const id of [incoming,native,recordID])await expect(page.locator('[data-record-id="'+id+'"]')).toHaveCount(1);
    await expect(page.getByLabel('Fixture command status')).toHaveText('Sending…');
    holdStatus=false;releaseStatus.splice(0).forEach(release=>release());
    await expect(page.locator('[data-record-id="'+recordID+'"]')).toHaveAttribute('data-command-id',commands[0]);
    await expect(page.locator('[data-record-id="'+native+'"]')).toHaveAttribute('data-command-id','');
    await expect(page.locator('[data-local-echo]')).toHaveCount(0);
    expect(commands).toHaveLength(1);
    console.log('compose: timeout_count=1; forced_status_delay=true; unrelated_native_preserved=true');
});
test('WEB-MSG-OPTIMISTIC OBSERVED observed without record IDs remains Sent and uses one captured row',async({page,context,request})=>{
    await context.addCookies(sessionCookies);const ids=await control(request,'/demo-fixtures');let recordID='';const commands:string[]=[];
    await page.routeWebSocket('**/web/v1/live',socket=>{
        const server=socket.connectToServer();socket.onMessage(data=>{const frame=JSON.parse(String(data));if(frame.t==='cmd')commands.push(frame.id);server.send(data);});
        server.onMessage(data=>{const frame=JSON.parse(String(data));if(frame.t==='cmd_status'&&frame.command?.observed_record_ids?.length){recordID=frame.command.observed_record_ids[0];socket.send(JSON.stringify({...frame,command:{...frame.command,observed_record_ids:[],parts:frame.command.parts?.map((part:any)=>({...part,record_id:undefined}))}}));return;}socket.send(data);});
    });
    await page.goto('/__fixtures/commands?chat='+encodeURIComponent(ids['message-chat']));
    await expect(page.getByRole('button',{name:'Send fixture message',exact:true})).toBeEnabled();
    await page.getByRole('textbox',{name:'Fixture message',exact:true}).fill('[ab-test compose] observed without IDs');
    await page.getByRole('button',{name:'Send fixture message',exact:true}).click();
    await expect.poll(()=>recordID).not.toBe('');
    await expect(page.getByLabel('Fixture command status')).toHaveText('Sent');
    await expect(page.getByLabel('Projection timeout count')).toHaveText('1');
    await expect(page.getByLabel('Missing observed identity count')).toHaveText('1');
    await expect(page.locator('[data-local-echo]')).toHaveCount(0);
    await expect(page.locator('[data-record-id="'+recordID+'"]')).toHaveCount(1);
    await expect(page.getByText('Not sure if delivered',{exact:true})).toHaveCount(0);
    expect(commands).toHaveLength(1);console.log('compose: missing_observed_ids=1; timeout_count=1; disposition=observed');
});
test('WEB-MSG-OPTIMISTIC FAILURE a definitive no-effect part failure restores its suppressed bubble',async({page,context,request})=>{
    await context.addCookies(sessionCookies);const ids=await control(request,'/demo-fixtures');await control(request,'/message-fault?name=part-no-effect');
    const held:(()=>void)[]=[],captures:any[]=[],commands:string[]=[];let failed=false,holdStatus=true;
    await page.routeWebSocket('**/web/v1/live',socket=>{
        const server=socket.connectToServer();socket.onMessage(data=>{const frame=JSON.parse(String(data));if(frame.t==='cmd')commands.push(frame.id);server.send(data);});
        server.onMessage(data=>{const frame=JSON.parse(String(data));if(frame.t==='cmd_status'){if(frame.status==='delivery_failed')failed=true;if(holdStatus){held.push(()=>socket.send(data));return;}}if(frame.t==='delta')captures.push(frame);socket.send(data);});
    });
    try{
        await page.goto('/__fixtures/commands?chat='+encodeURIComponent(ids['message-chat']));
        await expect(page.getByRole('button',{name:'Send fixture message',exact:true})).toBeEnabled();
        await page.getByRole('textbox',{name:'Fixture message',exact:true}).fill('[ab-test compose] definitive failure');
        await page.getByLabel('Fixture attachments').setInputFiles([{name:'bridge-no-effect-part.txt',mimeType:'text/plain',buffer:Buffer.from('[ab-test compose] no-effect attachment')}]);
        await expect(page.getByLabel('Fixture ready attachments')).toHaveText('1');
        await page.getByRole('button',{name:'Send fixture message',exact:true}).click();await expect.poll(()=>failed).toBe(true);
        await control(request,'/incoming');await expect.poll(()=>captures.length).toBeGreaterThan(0);
        await expect(page.getByLabel('Projection timeout count')).toHaveText('1');await expect(page.locator('[data-local-echo]')).toHaveCount(0);
        holdStatus=false;held.splice(0).forEach(release=>release());
        await expect(page.getByLabel('Fixture command status')).toHaveText('Not Delivered');
        await expect(page.locator('[data-local-echo][data-send-state="failed"]')).toHaveCount(1);
        expect(commands).toHaveLength(1);
    }finally{await control(request,'/message-fault?name=');}
});
test('WEB-MSG-OPTIMISTIC MONOTONIC a stale same-cursor reconcile record cannot regress observed',async({page,context,request})=>{
    await context.addCookies(sessionCookies);const ids=await control(request,'/demo-fixtures');let stale:(()=>void)|undefined;const commands:string[]=[];
    await page.routeWebSocket('**/web/v1/live',socket=>{
        const server=socket.connectToServer();socket.onMessage(data=>{const frame=JSON.parse(String(data));if(frame.t==='cmd')commands.push(frame.id);server.send(data);});
        server.onMessage(data=>{const frame=JSON.parse(String(data));if(frame.t==='snap'&&frame.data.records?.some((record:any)=>record.collection==='system.command'))stale=()=>socket.send(JSON.stringify({...frame,data:{...frame.data,records:frame.data.records.map((record:any)=>({...record,data:{...record.data,state:'sent',disposition:''}}))}}));socket.send(data);});
    });
    await page.goto('/__fixtures/commands?chat='+encodeURIComponent(ids['message-chat']));
    await expect(page.getByRole('button',{name:'Send fixture message',exact:true})).toBeEnabled();
    await page.getByRole('textbox',{name:'Fixture message',exact:true}).fill('[ab-test compose] monotonic status');
    await page.getByRole('button',{name:'Send fixture message',exact:true}).click();await expect(page.getByLabel('Fixture command status')).toHaveText('Sent');
    await page.getByRole('button',{name:'Check fixture command',exact:true}).click();await expect.poll(()=>Boolean(stale)).toBe(true);stale!();
    await page.evaluate(()=>new Promise(requestAnimationFrame));await expect(page.getByLabel('Fixture command status')).toHaveText('Sent');expect(commands).toHaveLength(1);
});
test('WEB-MSG-OPTIMISTIC NO-CANDIDATE server indeterminate without a released candidate keeps an echo with Check and Copy',async({page,context,request})=>{
    await context.addCookies(sessionCookies);const ids=await control(request,'/demo-fixtures');await control(request,'/message-fault?name=ui-interference');let reads=0;const commands:string[]=[];
    await page.addInitScript(()=>{Object.defineProperty(navigator,'clipboard',{value:{writeText:async(value:string)=>{(window as any).__copied=value;}}});});
    await page.routeWebSocket('**/web/v1/live',socket=>{const server=socket.connectToServer();socket.onMessage(data=>{const frame=JSON.parse(String(data));if(frame.t==='cmd')commands.push(frame.id);if(frame.t==='sub'&&frame.args?.collection==='system.command')reads++;server.send(data);});server.onMessage(data=>socket.send(data));});
    try{
        await page.goto('/__fixtures/commands?chat='+encodeURIComponent(ids['message-chat']));await expect(page.getByRole('button',{name:'Send fixture message',exact:true})).toBeEnabled();
        await page.getByRole('textbox',{name:'Fixture message',exact:true}).fill('[ab-test compose] server indeterminate');await page.getByRole('button',{name:'Send fixture message',exact:true}).click();
        await expect(page.getByLabel('Fixture command status')).toHaveText('Not sure if delivered');await expect(page.locator('[data-local-echo][data-send-state="indeterminate"]')).toHaveCount(1);
        await expect(page.getByLabel('Fixture ambiguity origin')).toHaveText('indeterminate');await expect(page.getByLabel('Uncertain delivery')).toHaveCount(0);
        await page.getByRole('button',{name:'Check fixture command',exact:true}).click();await expect.poll(()=>reads).toBe(1);
        await expect(page.getByRole('button',{name:'Copy fixture message',exact:true})).toBeVisible();
        await page.getByRole('button',{name:'Copy fixture message',exact:true}).click();expect(await page.evaluate(()=>(window as any).__copied)).toBe('[ab-test compose] server indeterminate');expect(commands).toHaveLength(1);
    }finally{await control(request,'/message-fault?name=');}
});
test('WEB-MSG-OPTIMISTIC Check strengthens server ambiguity through read-only original-ID Resolve',async({page,context,request})=>{
    await context.addCookies(sessionCookies);
    const ids=await control(request,'/demo-fixtures');
    const sent:any[]=[],statuses:any[]=[];
    page.on('websocket',socket=>{
        socket.on('framesent',event=>{try{sent.push(JSON.parse(String(event.payload)));}catch{}});
        socket.on('framereceived',event=>{try{const frame=JSON.parse(String(event.payload));if(frame.t==='cmd_status')statuses.push(frame);}catch{}});
    });
    await page.goto('/__fixtures/commands?chat='+encodeURIComponent(ids['message-chat']));
    const send=page.getByRole('button',{name:'Send fixture message',exact:true});
    await expect(send).toBeEnabled();
    await control(request,'/message-fault?name=part-progress');
    await page.getByRole('textbox',{name:'Fixture message',exact:true}).fill('[ab-test compose] explicit read-only Check');
    await send.click();
    await expect.poll(()=>statuses.some(frame=>frame.command?.disposition==='indeterminate')).toBe(true);
    const commandID=sent.find(frame=>frame.t==='cmd').id;
    await expect(page.getByLabel('Fixture ambiguity origin')).toHaveText('indeterminate');
    await page.getByRole('button',{name:'Check fixture command',exact:true}).click();
    await expect.poll(()=>statuses.some(frame=>frame.id===commandID&&frame.command?.state==='observed')).toBe(true);
    await expect(page.getByLabel('Fixture command status')).toHaveText('Sent');
    expect(sent.filter(frame=>frame.t==='cmd').map(frame=>frame.id)).toEqual([commandID]);
    expect(sent.filter(frame=>frame.t==='resolve').map(frame=>frame.id)).toEqual([commandID]);
    await page.getByRole('button',{name:'Check fixture command',exact:true}).click();
    await expect.poll(()=>statuses.some(frame=>frame.id===commandID&&frame.throttled===true)).toBe(true);
    await expect(page.getByLabel('Fixture command status')).toHaveText('Sent');
    expect(sent.filter(frame=>frame.t==='resolve').map(frame=>frame.id)).toEqual([commandID,commandID]);
    const final=statuses.find(frame=>frame.id===commandID&&frame.command?.state==='observed');
    expect(final.command.parts).toHaveLength(1);
    expect(final.command.parts[0].record_id).toMatch(/^msg_/);
    await expect(page.locator('[data-record-id="'+final.command.parts[0].record_id+'"]')).toHaveCount(1);
    await expect(page.locator('[data-local-echo]')).toHaveCount(0);
    const effect=await request.post(process.env.AB_WEB_CONTROL!+'/command-effect',{headers:{'X-Bridge-Lab':process.env.AB_WEB_CONTROL_KEY!},data:{command_id:commandID}});
    expect(effect.ok()).toBe(true);
    expect((await effect.json()).count).toBe(1);
    expect(sent.filter(frame=>frame.t==='cmd').map(frame=>frame.id)).toEqual([commandID]);
});
test('WEB-MSG-OPTIMISTIC Check failures stay on their command notice and clear after recovery',async({page,context,request})=>{
    await context.addCookies(sessionCookies);
    const ids=await control(request,'/demo-fixtures');
    const commands:string[]=[],statuses:any[]=[];
    let failFirst=true;
    await page.routeWebSocket('**/web/v1/live',socket=>{
        const server=socket.connectToServer();
        socket.onMessage(data=>{
            const frame=JSON.parse(String(data));
            if(frame.t==='cmd')commands.push(frame.id);
            if(frame.t==='resolve'&&frame.id===commands[0]&&failFirst){socket.send(JSON.stringify({t:'error',id:frame.id,code:'unavailable',message:'RAW_CHECK_INTERNAL_SENTINEL'}));return;}
            server.send(data);
        });
        server.onMessage(data=>{const frame=JSON.parse(String(data));if(frame.t==='cmd_status')statuses.push(frame);socket.send(data);});
    });
    await page.goto('/__fixtures/commands?chat='+encodeURIComponent(ids['message-chat']));
    const send=page.getByRole('button',{name:'Send fixture message',exact:true});
    await expect(send).toBeEnabled();
    for(let i=0;i<2;i++){
        await control(request,'/message-fault?name=part-progress');
        await page.getByRole('textbox',{name:'Fixture message',exact:true}).fill('[ab-test compose] scoped Check '+i);
        await send.click();
        await expect.poll(()=>statuses.some(frame=>frame.id===commands[i]&&frame.command?.state==='indeterminate')).toBe(true);
    }
    const notice=page.getByLabel('Uncertain delivery');
    await expect(notice).toContainText('2 messages may not have been delivered');
    await notice.getByRole('button',{name:'Check',exact:true}).click();
    await expect(page.getByLabel('Fixture command status')).toHaveText('Sent');
    await expect(notice).toContainText("Couldn't check this message. Try again.");
    await expect(notice).toContainText('1 message may not have been delivered');
    await expect(page.getByText('RAW_CHECK_INTERNAL_SENTINEL',{exact:false})).toHaveCount(0);
    expect(commands).toHaveLength(2);
    failFirst=false;
    await notice.getByRole('button',{name:'Check',exact:true}).click();
    await expect(notice).toHaveCount(0);
    expect(commands).toHaveLength(2);
    for(const id of commands){
        const effect=await request.post(process.env.AB_WEB_CONTROL!+'/command-effect',{headers:{'X-Bridge-Lab':process.env.AB_WEB_CONTROL_KEY!},data:{command_id:id}});
        expect(effect.ok()).toBe(true);expect((await effect.json()).count).toBe(1);
    }
});
