import {test,expect} from '@playwright/test';
import {mkdir,writeFile} from 'node:fs/promises';
import {bootstrap,control} from './session';

test('WEB-MSG-SEND LIFECYCLE an actual Composer click paints in its first frame, acknowledges and delivers once',async({page,context,request})=>{
    await bootstrap(context,request);const ids=await control(request,'/demo-fixtures');
    await control(request,'/message-fault?name=receipt-before-outcome');
    const sent:any[]=[],received:any[]=[];
    page.on('websocket',socket=>{socket.on('framesent',event=>sent.push(JSON.parse(String(event.payload))));socket.on('framereceived',event=>received.push(JSON.parse(String(event.payload))));});
    await page.addInitScript(()=>{(window as any).__sendFrames=[];window.addEventListener('click',event=>{if(!(event.target as Element).closest('[aria-label="Send message"]'))return;requestAnimationFrame(()=>{(window as any).__sendFrames.push(document.querySelectorAll('[data-local-echo]').length);});},{capture:true});});
    await page.goto('/messages/'+encodeURIComponent(ids['message-chat']));await expect(page.locator('.messages-transcript')).toHaveAttribute('aria-busy','false');
    const text='[ab-test canonical] text lifecycle';await page.getByRole('textbox',{name:'Message',exact:true}).fill(text);await page.getByRole('button',{name:'Send message',exact:true}).click();
    await expect.poll(()=>page.evaluate(()=>(window as any).__sendFrames)).toEqual([1]);
    await expect.poll(()=>sent.filter(frame=>frame.t==='cmd').length).toBe(1);const command=sent.find(frame=>frame.t==='cmd');
    await expect.poll(()=>received.some(frame=>frame.t==='ack'&&frame.id===command.id)).toBe(true);
    await expect.poll(()=>received.some(frame=>frame.t==='cmd_status'&&frame.id===command.id&&frame.command?.observed_record_ids?.length)).toBe(true);
    const recordID=received.find(frame=>frame.t==='cmd_status'&&frame.id===command.id&&frame.command?.observed_record_ids?.length).command.observed_record_ids[0];
    const row=page.locator('[data-message="'+recordID+'"]');await expect(row).toContainText(text);await expect(row.locator('.command-status')).toHaveText('Delivered');await expect(row).toHaveCount(1);await expect(page.locator('.message-row.outgoing').filter({hasText:text})).toHaveCount(1);await expect(page.locator('.message-row:not(.outgoing)').filter({hasText:text})).toHaveCount(1);await expect(page.locator('[data-local-echo]')).toHaveCount(0);
    expect(sent.filter(frame=>frame.t==='cmd')).toHaveLength(1);await control(request,'/message-fault?name=');
});

test('WEB-MSG-FAIL-RESEND TEXT Not Delivered offers one trusted Try Again for the exact failed row',async({page,context,request})=>{
    await bootstrap(context,request);const ids=await control(request,'/demo-fixtures'),commands:any[]=[],statuses:any[]=[];
    page.on('websocket',socket=>{socket.on('framesent',event=>{const frame=JSON.parse(String(event.payload));if(frame.t==='cmd')commands.push(frame);});socket.on('framereceived',event=>{const frame=JSON.parse(String(event.payload));if(frame.t==='cmd_status')statuses.push(frame);});});
    await control(request,'/message-fault?name=delivery-failed');
    try{
        await page.goto('/messages/'+encodeURIComponent(ids['message-chat']));await expect(page.locator('.messages-transcript')).toHaveAttribute('aria-busy','false');
        const text='[ab-test canonical] retry this text';await page.getByRole('textbox',{name:'Message',exact:true}).fill(text);await page.getByRole('button',{name:'Send message',exact:true}).click();
        await expect.poll(()=>statuses.some(frame=>frame.command?.disposition==='delivery_failed')).toBe(true);
        const original=commands[0],failed=statuses.find(frame=>frame.id===original.id&&frame.command?.disposition==='delivery_failed').command;
        const recordID=failed.parts?.[0]?.record_id??failed.observed_record_ids[0],row=page.locator('[data-message="'+recordID+'"]');
        await expect(row).toContainText('Not Delivered');await expect(row).toHaveCount(1);await expect(page.locator('[data-local-echo]')).toHaveCount(0);
        const retry=row.getByRole('button',{name:'Try Again',exact:true});await expect(retry).toBeEnabled();
        await retry.evaluate(button=>button.dispatchEvent(new MouseEvent('click',{bubbles:true})));expect(commands).toHaveLength(1);
        await control(request,'/message-fault?name=receipt-before-outcome');await retry.click();await expect.poll(()=>commands.length).toBe(2);
        const next=commands[1];expect(next.id).not.toBe(original.id);expect(next.op).toBe('messages.send');expect(next.args).toEqual({chat_id:ids['message-chat'],text,resend_of:recordID});
        await expect.poll(()=>statuses.some(frame=>frame.id===next.id&&frame.command?.observed_record_ids?.length)).toBe(true);
        const nextID=statuses.find(frame=>frame.id===next.id&&frame.command?.observed_record_ids?.length).command.observed_record_ids[0];
        await expect(page.locator('[data-message="'+nextID+'"] .command-status')).toHaveText('Delivered');await expect(row.getByRole('button',{name:'Try Again',exact:true})).toHaveCount(0);
        await expect(page.locator('[data-message="'+nextID+'"]')).toHaveCount(1);await expect(page.locator('[data-local-echo]')).toHaveCount(0);await expect(page.locator('.message-row.outgoing').filter({hasText:text})).toHaveCount(2);await expect(page.locator('.message-row:not(.outgoing)').filter({hasText:text})).toHaveCount(1);expect(commands).toHaveLength(2);
    }finally{await control(request,'/message-fault?name=');}
});

test('WEB-MSG-A11Y STRUCTURE named controls, active descendants and keyboard traversal expose the transcript',async({page,context,request})=>{
    await bootstrap(context,request);const ids=await control(request,'/demo-fixtures');
    await page.goto('/messages/'+encodeURIComponent(ids['message-chat'])+'/'+encodeURIComponent(ids['demo-message-00-000058']));await expect(page.locator('.messages-transcript')).toHaveAttribute('aria-busy','false');
    const transcript=page.getByRole('listbox',{name:'Messages transcript',exact:true}),composer=page.getByRole('textbox',{name:'Message',exact:true});
    await expect(transcript).toBeVisible();await expect(composer).toBeVisible();
    const snapshot=await transcript.ariaSnapshot();expect(snapshot).toContain('listbox "Messages transcript"');expect(snapshot).toContain('option');
    await transcript.focus();await page.keyboard.press('ArrowDown');await expect(transcript).toBeFocused();
    const active=await transcript.getAttribute('aria-activedescendant');expect(active).toBeTruthy();await expect(page.locator('[id="'+active+'"]')).toHaveAttribute('role','option');await expect(page.locator('[id="'+active+'"]')).toHaveAttribute('aria-selected','true');
    await composer.focus();await composer.fill('[ab-test canonical] accessible draft');await page.keyboard.press('Tab');await expect(page.getByRole('button',{name:'Send message',exact:true})).toBeFocused();await page.keyboard.press('Shift+Tab');await expect(composer).toBeFocused();
    expect(await composer.ariaSnapshot()).toContain('textbox "Message"');
});

test('WEB-MSG-LIVE LATENCY twenty synthetic inbound messages paint with receive motion within 500 ms p95',async({page,context,request},info)=>{
    await bootstrap(context,request);const ids=await control(request,'/demo-fixtures');
    await page.addInitScript(()=>{
        const state=window as any;state.__arrivals=[];const seen=new Set<string>();let armed=false;
        state.__armArrivals=()=>{for(const row of document.querySelectorAll<HTMLElement>('[data-message]'))seen.add(row.dataset.message!);armed=true;};
        new MutationObserver(()=>{if(!armed)return;for(const row of document.querySelectorAll<HTMLElement>('.message-row:not(.outgoing)[data-message]')){const id=row.dataset.message!;if(seen.has(id))continue;if(!row.querySelector('[data-motion="receive"]'))continue;seen.add(id);requestAnimationFrame(()=>{const box=row.getBoundingClientRect(),viewport=row.closest('.virtual-viewport')!.getBoundingClientRect();state.__arrivals.push({id,at:performance.timeOrigin+performance.now(),visible:box.bottom>viewport.top&&box.top<viewport.bottom});});}}).observe(document,{subtree:true,childList:true,attributes:true,attributeFilter:['data-motion']});
    });
    await page.goto('/messages/'+encodeURIComponent(ids['message-chat']));await expect(page.locator('.messages-transcript')).toHaveAttribute('aria-busy','false');
    const viewport=page.getByRole('listbox',{name:'Messages transcript'});await viewport.focus();await page.keyboard.press('Meta+ArrowDown');await expect(page.locator('.transcript')).toHaveCSS('transform','none');await page.evaluate(async()=>{await new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)));(window as any).__armArrivals();});
    const samples:number[]=[];
    for(let index=0;index<20;index++){
        const result=await control(request,'/incoming');await expect.poll(()=>page.evaluate(()=>(window as any).__arrivals.length)).toBe(index+1);
        const arrival=await page.evaluate(index=>(window as any).__arrivals[index],index);expect(arrival.visible).toBe(true);samples.push(arrival.at-result.started_at_ms);
        await expect(page.locator('[data-message="'+arrival.id+'"] .bubble-slot').first()).toHaveCSS('transform','none');
    }
    const sorted=[...samples].sort((a,b)=>a-b),metrics={samples:20,p95_ms:sorted[18],max_ms:sorted[19],endpoint:'sim capture start to next animation frame containing visible receiving transcript row'};
    await mkdir('../lab/evidence/web/metrics',{recursive:true});await writeFile(`../lab/evidence/web/metrics/WEB-MSG-LIVE-${info.project.name}.json`,JSON.stringify(metrics,null,2)+'\n');
    expect(Math.min(...samples)).toBeGreaterThanOrEqual(0);expect(metrics.p95_ms).toBeLessThanOrEqual(500);
});
