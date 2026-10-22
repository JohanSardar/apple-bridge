import { test, expect } from '@playwright/test';
import { header } from '../src/messages/format';
import { createTranscriptLayout, emojiOnly, layout, transcriptSpacing, textWidthWithSlack, wrapText, type LayoutPrefs } from '../src/messages/transcript/layout';
import type { ThreadItem } from '../src/data/types';
import type { Handles } from '../src/messages/store/types';
const now=new Date(2026,9,20,9,41);
const handles:Handles=new Map([['alex',{id:'alex',display_name:'Alex Morgan',initials:'AM'}]]);
const prefs:LayoutPrefs={phone:false,group:true,now,scale:1,measure:value=>Array.from(value).length*7,expanded:new Set(),locale:'en-US'};
test('layout math leaves subpixel shaping slack before allocating or wrapping text',()=>{
    expect(textWidthWithSlack(136.83679)).toBe(138);
    expect(textWidthWithSlack(400)).toBe(402);
    expect(wrapText('word word',137,value=>value==='word'?68:value===' '?0:0).lines).toBe(1);
    expect(wrapText('word word',136,value=>value==='word'?68:value===' '?0:0).lines).toBe(2);
});
function message(id:string,offset:number,data:Record<string,unknown>={}):ThreadItem{return {id,source:'messages',collection:'messages.message',version:1,data:{sent_at:new Date(now.getTime()+offset).toISOString(),sender_handle_id:'alex',kind:'message',text:'Hello',parts:[{index:0,kind:'text',text:'Hello'}],...data},reactions:[],attachments:[],reply_count:0};}
test('layout math groups sender runs within one minute and places only the last tail/avatar',()=>{
    const rows=layout([message('one',0),message('two',60000),message('three',120001)],handles,prefs,900);
    expect(rows.map(row=>row.bubbles[0].tail)).toEqual([false,true,true]);
    expect(rows.map(row=>row.senderName)).toEqual(['Alex Morgan',undefined,'Alex Morgan']);
    expect(rows.map(row=>row.avatar)).toEqual([undefined,'AM','AM']);
    expect(rows.map(row=>row.gap)).toEqual([9,3,9]);
    expect(rows[0].height).toBe(9+16+9+16+2+16+14);
    expect(rows[1].height).toBe(3+16+14);
});
test('layout math headers require a gap over one hour and include the year only when different',()=>{
    const rows=layout([message('one',0),message('two',3600000),message('three',7200001)],handles,prefs,900);
    expect(rows.map(row=>Boolean(row.header))).toEqual([true,false,true]);
    expect(rows[0].header).toBe('Today 9:41 AM');
    const yesterday=layout([message('one',-86400000)],handles,prefs,900)[0];expect(yesterday.header).toContain('Yesterday');
    const previousYear=layout([message('one',0,{sent_at:new Date(2025,9,20,9,41).toISOString()})],handles,prefs,900)[0];expect(previousYear.header).toContain('2025');
});
test('layout math caps opaque phone portraits at sixty percent of the viewport before bytes load',()=>{
    const item=message('photo',0,{parts:[{index:0,kind:'attachment',attachment_id:'asset'}]});
    item.attachments=[{id:'asset',source:'messages',collection:'messages.attachment',version:1,data:{mime_type:'image/jpeg',media:{width:400,height:1200}}}];
    for(const viewportHeight of [600,844,1000]){
        const options={...prefs,phone:true,viewportHeight},phone=layout([item],handles,options,390)[0].bubbles[0];
        expect(phone.width).toBe(273);expect(phone.height).toBeCloseTo(viewportHeight*.6,8);
        const pending={...item,attachments:item.attachments.map(value=>({...value,data:{...value.data,blob:{state:'pending'}}}))};
        expect(layout([pending],handles,options,390)[0].bubbles.map(value=>[value.width,value.height])).toEqual([[phone.width,phone.height]]);
    }
    const desktop=layout([item],handles,prefs,900)[0].bubbles[0];expect([desktop.width,desktop.height]).toEqual([120,360]);
});
test('layout math preserves alpha portrait aspect within the same phone height cap',()=>{
    const item=message('alpha',0,{parts:[{index:0,kind:'attachment',attachment_id:'asset'}]});
    item.attachments=[{id:'asset',source:'messages',collection:'messages.attachment',version:1,data:{mime_type:'image/png',is_sticker:true,media:{width:400,height:1200}}}];
    const bubble=layout([item],handles,{...prefs,phone:true,viewportHeight:844},390)[0].bubbles[0];
    expect(bubble.height).toBeCloseTo(844*.6,8);expect(bubble.width/bubble.height).toBeCloseTo(1/3,8);expect(bubble.floating).toBe(true);
});
test('layout math recognizes one to three grapheme emoji and never treats plain digits as emoji',()=>{
    for(const value of ['🌊','🌊☀️','👨‍👩‍👧‍👦','🇸🇪','1️⃣'])expect(emojiOnly(value)).toBe(true);
    for(const value of ['1','123','hello 🎉','🌊🌊🌊🌊','\uFE0F'])expect(emojiOnly(value)).toBe(false);
    const row=layout([message('emoji',0,{text:'🌊☀️',parts:[{index:0,kind:'text',text:'🌊☀️'}]})],handles,prefs,900)[0];expect(row.bubbles[0].emoji).toBe(true);expect(row.bubbles[0].tail).toBe(false);
});
test('layout math keeps per-part reactions, unsent wording, edit heights and one final receipt',()=>{
    const one=message('one',0,{is_from_me:true,delivered_at:now.toISOString()});
    const two=message('two',30000,{is_from_me:true,read_at:now.toISOString(),edit_history:[{text:'Before'}],parts:[{index:0,kind:'text',text:'After'},{index:1,kind:'text',text:'Also'}]});
    two.reactions=[{id:'like',source:'messages',collection:'messages.reaction',version:1,data:{kind:'like',part_index:1,sender_handle_id:'alex'}}];
    const closed=layout([one,two],handles,prefs,900),opened=layout([one,two],handles,{...prefs,expanded:new Set(['two'])},900);
    expect(closed.map(row=>row.receipt)).toEqual([undefined,'Read 9:41 AM']);expect(closed[1].bubbles.map(value=>value.reactions.length)).toEqual([0,1]);
    expect(opened[1].height-closed[1].height).toBe(16+14+4);
    expect(layout([message('unsent',0,{parts:[],text:'',retracted_parts:[0]})],handles,prefs,900)[0].system).toBe('Alex Morgan unsent a message');
    expect(layout([one,two],handles,{...prefs,atLatest:false},900).every(row=>!row.receipt)).toBe(true);
});
test('layout math keeps the receipt on the last partly unsent message',()=>{
    const first=message('one',0,{is_from_me:true,delivered_at:now.toISOString()});
    const last=message('two',30000,{is_from_me:true,read_at:now.toISOString(),parts:[{index:0,kind:'text',text:'Still here'},{index:1,kind:'text',text:'Removed'}],retracted_parts:[1]});
    const rows=layout([first,last],handles,prefs,900);
    expect(rows.map(row=>row.receipt)).toEqual([undefined,'Read 9:41 AM']);expect(rows[1].unsent).toBe('You unsent a message');
});
test('layout math suppresses delivery receipts on failed messages',()=>{
    const row=layout([message('failed',0,{is_from_me:true,error_code:1,delivered_at:now.toISOString()})],handles,prefs,900)[0];
    expect(row.failed).toBe(true);expect(row.receipt).toBeUndefined();
});
test('layout math reserves an edit label when previous versions are unavailable',()=>{
    const plain=layout([message('one',0)],handles,prefs,900)[0];
    const edited=layout([message('one',0,{edited_at:now.toISOString()})],handles,prefs,900)[0];
    expect(edited.edited).toBe(true);expect(edited.height-plain.height).toBe(22);
});
test('layout math reserves decoded audio, file, card and Genmoji dimensions',()=>{
    function attachment(mime:string,audio?:Record<string,unknown>){const item=message('media',0,{text:'',parts:[{index:0,kind:'attachment',attachment_id:'asset'}],audio});item.attachments=[{id:'asset',source:'messages',collection:'messages.attachment',version:1,data:{mime_type:mime}}];return item;}
    const size=(item:ThreadItem,phone=false)=>{const bubble=layout([item],handles,{...prefs,phone},phone?390:900)[0].bubbles[0];return[bubble.width,bubble.height];};
    expect(size(attachment('audio/mp4'))).toEqual([280,76]);
    expect(size(attachment('audio/mp4',{transcription:'Hello'}),true)).toEqual([273,120]);
    expect(size(attachment('application/pdf'))).toEqual([260,88]);
    expect(size(attachment('text/vcard'))).toEqual([260,88]);
    for(const kind of ['link','app'])for(const image of [false,true])expect(size(message('card',0,{text:'',parts:[],balloon:{kind,[kind]:{image_attachment_id:image?'image':undefined}}}))).toEqual([280,image?180:88]);
    expect(size(message('unknown',0,{text:'',parts:[],balloon:{kind:'poll'}}))).toEqual([260,88]);
    expect(size(message('genmoji',0,{text:'',parts:[{index:0,kind:'genmoji'}]}),true)).toEqual([22,22]);
});
test('layout math excludes hidden and sticker companions from standalone bubble geometry',()=>{
    const item=message('companions',0);
    item.attachments=[{id:'hidden',source:'messages',collection:'messages.attachment',version:1,data:{mime_type:'video/quicktime',hidden:true}},{id:'sticker',source:'messages',collection:'messages.attachment',version:1,data:{mime_type:'image/png',is_sticker:true}}];
    expect(layout([item],handles,prefs,900)[0].bubbles).toHaveLength(1);
});
test('transcript date math follows locale hour cycles and calendar-day header wording',()=>{
    const current=new Date(2026,9,20,9,41),local=(year:number,month:number,day:number,hour=9,minute=41)=>new Date(year,month,day,hour,minute).toISOString();
    expect(header(local(2026,9,20),current,'en-US')).toBe('Today 9:41 AM');
    expect(header(local(2026,9,19,18,1),current,'en-US')).toBe('Yesterday 6:01 PM');
    expect(header(local(2026,9,18,18,1),current,'en-US')).toBe('Sunday 6:01 PM');
    expect(header(local(2026,0,1,18,1),current,'en-US')).toBe('Thu, Jan 1 at 6:01 PM');
    expect(header(local(2025,11,31,18,1),current,'en-US')).toBe('Dec 31, 2025 at 6:01 PM');
    expect(header(local(2026,9,20,18,1),current,'sv-SE')).toBe('Today 18:01');
});

test('WEB-MSG-THREAD-CORE layout math gives exactly the final bubble a tail for every same-sender run regardless of ID ties',()=>{
    const permutations=(values:string[]):string[][]=>values.length?values.flatMap((value,index)=>permutations(values.filter((_,other)=>other!==index)).map(rest=>[value,...rest])):[[]];
    for(const mode of ['incoming','direction'])for(let mask=0;mask<16;mask++)for(const ids of permutations(['z','a','m','b'])){
        const items=ids.map((id,index)=>message(id,0,{is_from_me:mode==='direction'&&Boolean(mask&(1<<index)),sender_handle_id:mode==='incoming'&&Boolean(mask&(1<<index))?'beau':'alex',parts:[{index:0,kind:'text',text:'First'},{index:1,kind:'text',text:'Last'}]})).sort((a,b)=>a.id.localeCompare(b.id));
        const rows=layout(items,handles,prefs,900),runs:{sender:string;tails:boolean[]}[]=[];
        for(let index=0;index<items.length;index++){
            const item=items[index],sender=item.data.is_from_me?'self':String(item.data.sender_handle_id);
            if(runs.at(-1)?.sender!==sender)runs.push({sender,tails:[]});
            runs.at(-1)!.tails.push(...rows[index].bubbles.map(bubble=>bubble.tail));
        }
        for(const run of runs)expect(run.tails,mode+':'+mask+':'+ids.join(',')).toEqual(run.tails.map((_,index)=>index===run.tails.length-1));
    }
});

test('WEB-MSG-REACTIONS layout math reserves sixteen points above reaction-bearing bubbles',()=>{
    const item=message('reacted',0),plain=layout([item],handles,prefs,900)[0];
    item.reactions=[{id:'love',source:'messages',collection:'messages.reaction',version:1,data:{kind:'love',part_index:0}}];
    const reacted=layout([item],handles,prefs,900)[0];
    expect(transcriptSpacing.reactionTop).toBe(16);expect(reacted.height-plain.height).toBe(transcriptSpacing.reactionTop);
});

test('WEB-MSG-COMPOSITION layout math groups only compatible image runs and retains actual members',()=>{
    const photo=(id:string,index:number)=>{const item=message(id,index*10000,{text:'',parts:[{index:0,kind:'attachment',attachment_id:'asset-'+id}]});item.attachments=[{id:'asset-'+id,source:'messages',collection:'messages.attachment',version:1,data:{mime_type:'image/jpeg',media:{width:240,height:400}}}];return item;};
    const items=['a','b','c','d'].map(photo);
    for(const phone of [false,true]){
        const rows=layout(items,handles,{...prefs,phone,viewportHeight:844},phone?390:900);expect(rows).toHaveLength(1);expect(rows[0].members?.map(item=>item.id)).toEqual(['a','b','c','d']);expect(rows[0].bubbles[0].mediaAttachments?.map(item=>item.id)).toEqual(['asset-a','asset-b','asset-c','asset-d']);expect(rows[0].bubbles[0].tail).toBe(false);expect(rows[0].bubbles[0].width).toBe(phone?240:480);expect(rows[0].bubbles[0].height).toBe(360);
        const split=layout(items,handles,{...prefs,phone,selectedMessageID:'c'},phone?390:900);expect(split.map(row=>row.members?.map(item=>item.id))).toEqual([['a','b'],['c','d']]);
    }
    for(const mutate of [(item:ThreadItem)=>item.data.parts=[{index:1,kind:'attachment',attachment_id:item.attachments![0].id}],(item:ThreadItem)=>item.data.parts=[],(item:ThreadItem)=>item.reactions=[{id:'reaction',source:'messages',collection:'messages.reaction',version:1,data:{kind:'like'}}],(item:ThreadItem)=>item.data.edited_at=now.toISOString(),(item:ThreadItem)=>item.attachments![0].data.media={width:240,height:400,live:true},(item:ThreadItem)=>item.attachments![0].data.mime_type='video/mp4']){
        const isolated=items.map(item=>structuredClone(item));mutate(isolated[1]);const rows=layout(isolated,handles,prefs,900);expect(rows.find(row=>row.id==='b')?.members).toBeUndefined();
    }
});

test('layout math cached windows match fresh layouts after boundary, content and typography changes',()=>{
    const cached=createTranscriptLayout(),one=message('one',0,{is_from_me:true,read_at:now.toISOString()}),two=message('two',60000),three=message('three',120001);
    cached([one,two,three],handles,prefs,900,'font-1',{start:0,end:2});
    const initial=cached([one,two,three],handles,prefs,900,'font-1');
    expect(initial).toEqual(layout([one,two,three],handles,prefs,900));
    expect(cached([one,two,three],handles,prefs,900,'font-1')[1]).toBe(initial[1]);
    const windows=[[message('older',-60000),one,two,three],[one,two],[one,{...two,version:2,data:{...two.data,text:'Changed',parts:[{index:0,kind:'text',text:'Changed'}]}},three]];
    for(const items of windows)expect(cached(items,handles,prefs,900,'font-1')).toEqual(layout(items,handles,prefs,900));
    for(const change of [{...prefs,atLatest:false},{...prefs,scale:1.5},{...prefs,expanded:new Set(['one'])},{...prefs,now:new Date(now.getTime()+86400000)},{...prefs,measure:(value:string)=>value.length*12}])expect(cached([one,two,three],handles,change,390,'font-2')).toEqual(layout([one,two,three],handles,change,390));
    const renamed:Handles=new Map([['alex',{id:'alex',display_name:'Alex Example',initials:'AE'}]]);
    expect(cached([one,two,three],renamed,prefs,900,'font-1')).toEqual(layout([one,two,three],renamed,prefs,900));
});
