import {test,expect} from '@playwright/test';
import {control,prepareSession} from './session';

test('WEB-RM-THREAD-ORIGINATOR-BROWSER filters the root and exact replies on web.1',async({page,request})=>{
 const ids=await control(request,'/demo-fixtures');
 await prepareSession(page);
 await page.goto('/messages');
 const pages=await page.evaluate(async({chat,root})=>{
  const socket=new WebSocket(`${location.protocol==='https:'?'wss:':'ws:'}//${location.host}/web/v1/live`,'web.1');
  type Frame={t:string;sub?:string;data?:{items:{id:string;data:{sent_at:string}}[];before?:string;after?:string;older:number;newer:number}};
  const waiting=new Map<string,(frame:Frame)=>void>();
  let welcome:()=>void=()=>{};
  const ready=new Promise<void>(resolve=>welcome=resolve);
  socket.onopen=()=>socket.send(JSON.stringify({t:'hello',v:1}));
  socket.onmessage=event=>{const frame=JSON.parse(event.data) as Frame;if(frame.t==='welcome')welcome();else if(frame.sub)waiting.get(frame.sub)?.(frame);else if(frame.t==='error')for(const resolve of waiting.values())resolve(frame);};
  const snapshot=(id:string,args:Record<string,unknown>)=>new Promise<Frame>((resolve,reject)=>{
   const timer=setTimeout(()=>reject(new Error('filtered snapshot timeout')),20000);
   waiting.set(id,frame=>{if(frame.t==='snap'||frame.t==='error'){clearTimeout(timer);waiting.delete(id);resolve(frame);}});
   socket.send(JSON.stringify({t:'sub',id,view:'thread',args:{chat_id:chat,thread_originator_id:root,limit:100,...args}}));
  });
  try{
   await ready;
   return[await snapshot('filtered',{anchor:'oldest'})];
  }finally{socket.close();}
 },{chat:ids['message-chat'],root:ids['demo-message-00-000006']});
 expect(pages).toHaveLength(1);
 const frame=pages[0];expect(frame.t).toBe('snap');
 expect(frame.data?.items.map(item=>item.id)).toEqual([6,7,8,9,10].map(index=>ids['demo-message-00-'+String(index).padStart(6,'0')]));
 expect(frame.data?.before).toBeUndefined();expect(frame.data?.after).toBeUndefined();
 expect(frame.data?.older).toBe(0);expect(frame.data?.newer).toBe(0);
});
