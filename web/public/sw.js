const SHELL='ab.shell.__VERSION__';
const THUMBS='ab.thumbs';
const MAX_THUMB_BYTES=200_000_000;
const PRECACHE="__PRECACHE__";
const trusted=new Set();
let generation=0,queue=Promise.resolve();
function enqueue(action){const work=queue.catch(()=>{}).then(action);queue=work.then(()=>{}).catch(()=>{});return work;}
self.addEventListener('install',event=>{event.waitUntil(self.skipWaiting());});
self.addEventListener('activate',event=>{event.waitUntil(self.clients.claim());});
async function wipe(){
  generation++;
  trusted.clear();
  await enqueue(async()=>{for(const key of await caches.keys())if(key.startsWith('ab.'))await caches.delete(key);});
}
self.addEventListener('message',event=>{
  if(event.data?.t==='wipe'){event.waitUntil(wipe().then(()=>event.ports[0]?.postMessage({ok:true})));return;}
  if(event.data?.t!=='trusted'||!event.source?.id)return;
  const version=generation;
  event.waitUntil((async()=>{
    const response=await fetch('/web/v1/session',{cache:'no-store',credentials:'same-origin',signal:AbortSignal.timeout(5000)});
    const session=response.ok?await response.json():null;
    if(!session||session.kind!=='trusted'||session.cache!=='persistent'){await wipe();event.ports[0]?.postMessage({ok:false});return;}
    if(version!==generation){event.ports[0]?.postMessage({ok:false});return;}
    trusted.add(event.source.id);
    for(const url of PRECACHE){
      if(version!==generation)break;
      const asset=await fetch(url,{cache:'reload',credentials:'same-origin',signal:AbortSignal.timeout(5000)});
      if(asset.ok)await enqueue(async()=>{if(version===generation){const cache=await caches.open(SHELL);await cache.put(url,asset);}});
    }
    await enqueue(async()=>{if(version===generation)for(const key of await caches.keys())if(key.startsWith('ab.shell.')&&key!==SHELL)await caches.delete(key);});
    event.ports[0]?.postMessage({ok:true});
  })());
});
async function thumbnail(request,response,version){
  if(version!==generation)return;
  const bytes=await response.arrayBuffer();
  if(bytes.byteLength>MAX_THUMB_BYTES||version!==generation)return;
  const headers=new Headers(response.headers);
  headers.set('X-AB-Bytes',String(bytes.byteLength));
  headers.set('X-AB-Used',String(Date.now()));
  const cache=await caches.open(THUMBS);
  await cache.put(request,new Response(bytes,{status:response.status,headers}));
  const entries=[];
  let total=0;
  for(const key of await cache.keys()){
    const value=await cache.match(key);
    const size=Number(value.headers.get('X-AB-Bytes')||0);
    total+=size;
    entries.push({key,size,at:Number(value.headers.get('X-AB-Used')||0)});
  }
  entries.sort((a,b)=>a.at-b.at);
  for(const entry of entries){if(total<=MAX_THUMB_BYTES)break;await cache.delete(entry.key);total-=entry.size;}
}
self.addEventListener('fetch',event=>{
  const request=event.request,url=new URL(request.url);
  if(request.method!=='GET'||url.origin!==self.location.origin||request.headers.has('Range'))return;
  if(url.pathname.startsWith('/web/v1/')&&(request.mode==='navigate'||!url.pathname.startsWith('/web/v1/blob/')))return;
  const client=event.clientId||event.resultingClientId;
  const version=generation;
  if(request.mode==='navigate'){
    const wasTrusted=trusted.has(event.clientId);
    event.respondWith((async()=>{
      let allowed=false;
      try{
        const response=await fetch('/web/v1/session',{cache:'no-store',credentials:'same-origin',signal:AbortSignal.timeout(5000)});
        const session=response.ok?await response.json():null;
        allowed=session?.kind==='trusted'&&session.cache==='persistent';
        if(!allowed){await wipe();return fetch(request);}
      }catch{allowed=wasTrusted;}
      if(!allowed||version!==generation)return fetch(request);
      trusted.add(client);
      const cached=await enqueue(async()=>{if(version!==generation)return;return (await caches.open(SHELL)).match('/');});
      return cached??fetch(request);
    })());
    return;
  }
  if(!trusted.has(client))return;
  const thumb=url.pathname.startsWith('/web/v1/blob/')&&url.searchParams.get('variant')==='thumb';
  const shell=url.pathname.startsWith('/assets/')||url.pathname==='/icon.svg'||url.pathname==='/icons.svg'||url.pathname==='/manifest.webmanifest';
  if(!thumb&&!shell)return;
  event.respondWith((async()=>{
    const cached=await enqueue(async()=>{
      if(version!==generation||!trusted.has(client))return;
      const cache=await caches.open(thumb?THUMBS:SHELL);
      const value=await cache.match(request);
      if(value&&thumb)await thumbnail(request,value.clone(),version);
      return value;
    });
    if(cached)return cached;
    const response=await fetch(request);
    if(response.ok&&thumb){const copy=response.clone();event.waitUntil(enqueue(()=>thumbnail(request,copy,version)));}
    if(response.ok&&shell){const copy=response.clone();event.waitUntil(enqueue(async()=>{if(version===generation&&trusted.has(client))await(await caches.open(SHELL)).put(request,copy);}));}
    return response;
  })());
});
