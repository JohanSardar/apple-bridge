const SHELL='ab.shell.__VERSION__';
const THUMBS='ab.thumbs';
const PRECACHE="__PRECACHE__";
const trusted=new Set();
let generation=0,queue=Promise.resolve();
self.addEventListener('install',event=>{event.waitUntil(self.skipWaiting());});
self.addEventListener('activate',event=>{event.waitUntil(self.clients.claim());});
async function wipe(){generation++;trusted.clear();await queue.catch(()=>{});for(const key of await caches.keys())if(key.startsWith('ab.'))await caches.delete(key);}
self.addEventListener('message',event=>{
  if(event.data?.t==='wipe'){event.waitUntil(wipe());return;}
  if(event.data?.t!=='trusted'||!event.source?.id)return;
  const version=generation;
  event.waitUntil((async()=>{
    const response=await fetch('/web/v1/session',{cache:'no-store',credentials:'same-origin'});
    const session=response.ok?await response.json():null;
    if(!session||session.kind!=='trusted'||session.cache!=='persistent'){await wipe();event.ports[0]?.postMessage({ok:false});return;}
    trusted.add(event.source.id);
    queue=queue.catch(()=>{}).then(async()=>{if(version!==generation)return;const cache=await caches.open(SHELL);for(const url of PRECACHE){if(version!==generation)return;const response=await fetch(url,{cache:'reload',credentials:'same-origin'});if(response.ok)await cache.put(url,response);}for(const key of await caches.keys())if(key.startsWith('ab.shell.')&&key!==SHELL)await caches.delete(key);});
    await queue;
    event.ports[0]?.postMessage({ok:true});
  })());
});
async function thumbnail(request,response,version){
  const bytes=await response.arrayBuffer();
  if(bytes.byteLength>200*1024*1024||version!==generation)return;
  const headers=new Headers(response.headers);headers.set('X-AB-Bytes',String(bytes.byteLength));headers.set('X-AB-Used',String(Date.now()));
  const cache=await caches.open(THUMBS);
  await cache.put(request,new Response(bytes,{status:response.status,headers}));
  const entries=[];
  let total=0;
  for(const key of await cache.keys()){const value=await cache.match(key);const size=Number(value.headers.get('X-AB-Bytes')||0);total+=size;entries.push({key,size,at:Number(value.headers.get('X-AB-Used')||0)});}
  entries.sort((a,b)=>a.at-b.at);
  for(const entry of entries){if(total<=200*1024*1024)break;await cache.delete(entry.key);total-=entry.size;}
}
self.addEventListener('fetch',event=>{
  const request=event.request,url=new URL(request.url);
  if(request.method!=='GET'||url.origin!==self.location.origin||request.headers.has('Range'))return;
  const client=event.clientId||event.resultingClientId;
  if(request.mode==='navigate'){
    const wasTrusted=trusted.has(event.clientId);
    event.respondWith((async()=>{
      let allowed=false;
      try{const response=await fetch('/web/v1/session',{cache:'no-store',credentials:'same-origin'});const session=response.ok?await response.json():null;allowed=session?.kind==='trusted'&&session.cache==='persistent';if(!allowed){await wipe();return fetch(request);}}catch{allowed=wasTrusted;}
      if(!allowed)return fetch(request);
      trusted.add(client);
      const cache=await caches.open(SHELL);
      return await cache.match('/')??fetch(request);
    })());
    return;
  }
  if(!trusted.has(client))return;
  const version=generation;
  const thumb=url.pathname.startsWith('/web/v1/blob/')&&url.searchParams.get('variant')==='thumb';
  const shell=url.pathname.startsWith('/assets/')||url.pathname==='/icon.svg'||url.pathname==='/manifest.webmanifest';
  if(!thumb&&!shell)return;
  event.respondWith((async()=>{
    const cache=await caches.open(thumb?THUMBS:SHELL);
    const cached=await cache.match(request);
    if(cached){
      if(thumb){queue=queue.catch(()=>{}).then(()=>thumbnail(request,cached.clone(),version));event.waitUntil(queue);}
      return cached;
    }
    const response=await fetch(request);
    if(response.ok&&thumb){queue=queue.catch(()=>{}).then(()=>thumbnail(request,response.clone(),version));event.waitUntil(queue);}
    return response;
  })());
});
