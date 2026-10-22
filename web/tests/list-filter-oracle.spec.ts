import { expect } from '@playwright/test';
import { test } from './list-session';
import { control } from './session';
const filters=['all','known','unknown','unread','deleted','spam'] as const;
const ids=(rows:any[])=>rows.map(row=>row.chat.id).sort();
test('WEB-MSG-FILTERS ORACLE stable transport equals server filters with fourth linked participant and hidden Contacts',async({page,context,request})=>{
    await control(request,'/list-filters');
    const sent:any[]=[],received:any[]=[];
    page.on('websocket',socket=>{socket.on('framesent',frame=>{try{sent.push(JSON.parse(String(frame.payload)));}catch{}});socket.on('framereceived',frame=>{try{received.push(JSON.parse(String(frame.payload)));}catch{}});});
    for(const contacts of ['visible','hidden']){
        await page.goto('about:blank');await control(request,'/list-filter-group');
        const oracle=await control(request,'/list-filter-oracle?contacts='+contacts);
        const group=oracle.all.find((row:any)=>row.title==='Fourth linked participant');
        expect(group.chat.data.participants).toHaveLength(4);expect(group.avatar_handles).toHaveLength(3);expect(group.avatar_handles.every((handle:any)=>handle.contact_id===null)).toBe(true);
        expect(group.known_sender).toBe(true);expect(ids(oracle.known)).toContain(group.chat.id);expect(ids(oracle.unknown)).not.toContain(group.chat.id);
        const session=await control(request,'/session?contacts='+contacts);await context.clearCookies();await context.addCookies([session.cookie]);sent.length=0;received.length=0;
        await page.goto('/__fixtures/live');const rows=page.locator('.fixture>p[data-chat]'),select=page.getByLabel('Chat filter',{exact:true});
        const rendered=()=>rows.evaluateAll(elements=>elements.map(element=>(element as HTMLElement).dataset.chat!).sort());
        await expect.poll(rendered).toEqual(ids(oracle.all));
        await expect.poll(()=>sent.filter(frame=>frame.t==='sub'&&frame.view==='chats'&&!frame.args?.after).map(frame=>frame.args.filter).sort()).toEqual(['all','deleted','spam']);
        const baseline=sent.filter(frame=>frame.t==='sub'&&frame.view==='chats').map(frame=>frame.id).sort();
        for(const filter of filters){await select.selectOption(filter);await expect.poll(rendered).toEqual(ids(oracle[filter]));}
        expect(sent.filter(frame=>frame.t==='sub'&&frame.view==='chats').map(frame=>frame.id).sort()).toEqual(baseline);
        expect(sent.filter(frame=>frame.t==='unsub'&&baseline.includes(frame.id))).toEqual([]);
        if(contacts==='hidden'){
            const summaries=received.filter(frame=>frame.t==='snap').flatMap(frame=>frame.data?.chats??[]);expect(summaries.length).toBeGreaterThan(500);expect(summaries.every(row=>row.avatar_handles.every((handle:any)=>handle.contact_id===null))).toBe(true);
            const response=await page.request.post('/web/v1/handles',{headers:{Origin:process.env.AB_WEB_URL!},data:{ids:group.chat.data.participants}});expect(response.ok()).toBe(true);const handles=(await response.json()).handles;
            expect(handles).toHaveLength(4);expect(handles.every((handle:any)=>!handle.contact_id&&!handle.avatar&&!handle.given_name&&!handle.family_name&&handle.display_name===handle.address)).toBe(true);
        }
        for(const [known,unread] of [[false,true],[false,false],[true,true]]){
            await control(request,`/list-filter-group?known=${Number(known)}&unread=${Number(unread)}`);
            const next=await control(request,'/list-filter-oracle?contacts='+contacts);
            for(const filter of ['known','unknown','unread'] as const){await select.selectOption(filter);await expect.poll(rendered).toEqual(ids(next[filter]));}
            expect(sent.filter(frame=>frame.t==='sub'&&frame.view==='chats').map(frame=>frame.id).sort()).toEqual(baseline);
        }
    }
});
