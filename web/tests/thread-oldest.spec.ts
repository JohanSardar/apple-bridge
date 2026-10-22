import {test,expect} from '@playwright/test';
import {control,prepareSession} from './session';

test('WEB-RM-THREAD-OLDEST oldest anchor pages forward through the huge archive',async({page,request})=>{
 const ids=await control(request,'/demo-fixtures');
 await prepareSession(page);
 await page.goto('/messages');
 const pages=await page.evaluate(async(chat)=>{
  const socket=new WebSocket(`${location.protocol==='https:'?'wss:':'ws:'}//${location.host}/web/v1/live`,'web.1');
  type Frame={t:string;sub?:string;data?:{items:{id:string;data:{sent_at:string}}[];before?:string;after?:string;older:number;newer:number}};
  const waiting=new Map<string,(frame:Frame)=>void>();
  let welcome:()=>void=()=>{};
  const ready=new Promise<void>(resolve=>welcome=resolve);
  socket.onopen=()=>socket.send(JSON.stringify({t:'hello',v:1}));
  socket.onmessage=event=>{const frame=JSON.parse(event.data) as Frame;if(frame.t==='welcome')welcome();else if(frame.sub)waiting.get(frame.sub)?.(frame);else if(frame.t==='error')for(const resolve of waiting.values())resolve(frame);};
  const snapshot=(id:string,args:Record<string,unknown>)=>new Promise<Frame>((resolve,reject)=>{
   const timer=setTimeout(()=>reject(new Error('oldest snapshot timeout')),20000);
   waiting.set(id,frame=>{if(frame.t==='snap'||frame.t==='error'){clearTimeout(timer);waiting.delete(id);resolve(frame);}});
   socket.send(JSON.stringify({t:'sub',id,view:'thread',args:{chat_id:chat,limit:100,...args}}));
  });
  try{
   await ready;
   const first=await snapshot('oldest',{anchor:'oldest'});
   if(!first.data)return[first];
   const second=await snapshot('forward',{after:first.data.after});
   if(!second.data)return[first,second];
   const back=await snapshot('back',{before:second.data.before});
   return[first,second,back];
  }finally{socket.close();}
 },ids['demo-chat-39']);
 expect(pages).toHaveLength(3);
 for(const frame of pages){expect(frame.t).toBe('snap');expect(frame.data?.items).toHaveLength(100);}
 const[first,second,back]=pages.map(frame=>frame.data!);
 expect(first.before).toBeUndefined();expect(first.after).toBeTruthy();
 expect(first.older).toBe(0);expect(first.newer).toBe(99900);
 expect(second.older).toBe(100);expect(second.newer).toBe(99800);
 expect(back.items.map(item=>item.id)).toEqual(first.items.map(item=>item.id));
 expect(new Set([...first.items,...second.items].map(item=>item.id)).size).toBe(200);
 const earliest=Date.parse('2026-10-20T16:00:00Z')-730*86400000;
 for(const[index,item]of[...first.items,...second.items].entries())expect(Date.parse(item.data.sent_at)).toBe(earliest+index*600000);
});
