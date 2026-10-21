import {test,expect} from '@playwright/test';
import {layout,type LayoutPrefs} from '../src/messages/transcript/layout';
import type {ThreadItem} from '../src/data/types';
const prefs:LayoutPrefs={phone:true,viewportHeight:844,group:false,now:new Date('2026-10-20T16:00:00Z'),scale:1,measure:value=>value.length*7,expanded:new Set()};
function item(mime:string,width:number,height:number,live=false,sticker=false):ThreadItem{return{id:'message',source:'messages',collection:'messages.message',version:1,data:{sent_at:'2026-10-20T16:00:00Z',parts:[{index:0,kind:'attachment',attachment_id:'asset'}]},attachments:[{id:'asset',source:'messages',collection:'messages.attachment',version:1,data:{mime_type:mime,is_sticker:sticker,media:{width,height,live}}}],reactions:[],reply_count:0};}
function size(value:ThreadItem,options=prefs,width=390){const bubble=layout([value],new Map(),options,width)[0].bubbles[0];return[bubble.width,bubble.height];}
test('video layout math floors video and Live dimensions without upscaling ordinary images or stickers',()=>{
 for(const mime of ['video/mp4','video/quicktime'])expect(size(item(mime,16,16))).toEqual([120,120]);
 expect(size(item('image/jpeg',16,16,true))).toEqual([120,120]);
 expect(size(item('image/jpeg',16,16))).toEqual([16,16]);
 expect(size(item('image/png',16,16,false,true))).toEqual([16,16]);
 expect(size(item('video/mp4',0,0))).toEqual([260,120]);
 expect(size(item('image/jpeg',0,0))).toEqual([260,88]);
});
test('video layout math caps both minimum dimensions and preserves larger natural media sizing',()=>{
 expect(size(item('video/mp4',16,16),{...prefs,viewportHeight:100},100)).toEqual([60,60]);
 expect(size(item('image/jpeg',16,16,true),{...prefs,viewportHeight:100},100)).toEqual([60,60]);
 expect(size(item('video/mp4',320,180))).toEqual([273,153.5625]);
 expect(size(item('video/mp4',180,320))).toEqual([180,320]);
 expect(size(item('video/mp4',1920,200))).toEqual([273,120]);
 expect(size(item('video/mp4',1920,200),{...prefs,phone:false},900)).toEqual([480,120]);
});
