import {test,expect} from '@playwright/test';
import {mkdir,readFile,writeFile} from 'node:fs/promises';
import {gzipSync} from 'node:zlib';
import {bootstrap,control} from './session';
type Chunk={file:string;isEntry?:boolean;imports?:string[];dynamicImports?:string[];css?:string[]};
test('WEB-MSG-VIEWER-PRECACHE precache artifact contains the shell static closure and excludes dynamic-only chunks',async()=>{
 const manifest:Record<string,Chunk>=JSON.parse(await readFile('dist/.vite/manifest.json','utf8'));
 const html=await readFile('dist/index.html','utf8'),entry=/<script[^>]+src="([^"]+)"/u.exec(html)![1];
 const root=Object.keys(manifest).find(key=>'/'+manifest[key].file===entry);expect(root).toBeDefined();expect(manifest[root!].isEntry).toBe(true);
 const visited=new Set<string>(),queue=[root!],required=new Set<string>();
 while(queue.length){const key=queue.shift()!;if(visited.has(key))continue;visited.add(key);const chunk=manifest[key];required.add('/'+chunk.file);for(const css of chunk.css??[])required.add('/'+css);queue.push(...chunk.imports??[]);}
 for(const match of html.matchAll(/<link\b[^>]*\brel="stylesheet"[^>]*\bhref="([^"]+)"/gu))required.add(match[1]);
 const worker=await readFile('dist/sw.js','utf8'),precache:string[]=JSON.parse(/const PRECACHE=(\[[^;]+\]);/u.exec(worker)![1]);
 expect(precache).toContain(entry);for(const asset of required)expect(precache).toContain(asset);
 const dynamicOnly=Object.entries(manifest).filter(([key,chunk])=>!visited.has(key)&&chunk.file.endsWith('.js')).map(([,chunk])=>'/'+chunk.file);expect(dynamicOnly.length).toBeGreaterThan(0);
 expect(precache.filter(asset=>dynamicOnly.includes(asset))).toEqual([]);
 expect(precache.filter(asset=>/\.(js|css)$/u.test(asset)).sort()).toEqual([...required].sort());
});
test('WEB-MSG-VIEWER-PRECACHE trusted startup leaves the viewer unloaded until hover and caches its first use',async({page,context,request},info)=>{
 await bootstrap(context,request);const ids=await control(request,'/demo-fixtures'),requests:string[]=[];
 context.on('request',value=>{const path=new URL(value.url()).pathname;if(/\/assets\/ViewerSession-[^/]+\.js$/u.test(path))requests.push(path);});
 const manifest:Record<string,Chunk>=JSON.parse(await readFile('dist/.vite/manifest.json','utf8')),viewer=Object.values(manifest).find(chunk=>/ViewerSession-[^/]+\.js$/u.test(chunk.file));expect(viewer).toBeDefined();
 await page.goto('/__fixtures/media?chat='+encodeURIComponent(ids['message-chat']));
 await page.evaluate(async()=>{const registration=await navigator.serviceWorker.ready;await new Promise<void>((resolve,reject)=>{const channel=new MessageChannel(),timer=setTimeout(()=>reject(new Error('Trusted precache did not acknowledge')),15000);channel.port1.onmessage=event=>{clearTimeout(timer);event.data.ok?resolve():reject(new Error('Trusted precache refused'));};registration.active!.postMessage({t:'trusted'},[channel.port2]);});});
 await expect.poll(()=>page.evaluate(()=>Boolean(navigator.serviceWorker.controller))).toBe(true);
 const cached=()=>page.evaluate(async()=>{const paths:string[]=[];for(const key of await caches.keys())if(key.startsWith('ab.shell.'))for(const request of await(await caches.open(key)).keys())paths.push(new URL(request.url).pathname);return paths;});
 expect(requests).toEqual([]);expect(await cached()).toContain('/'+manifest['index.html'].file);expect(await cached()).not.toContain('/'+viewer!.file);
 const source=page.locator(`[data-message="${ids['demo-message-00-000032']}"] .media-open`);await source.hover();await expect.poll(()=>requests.length).toBeGreaterThan(0);await expect(page.getByRole('dialog',{name:'Media viewer'})).toHaveCount(0);
 await expect.poll(cached).toContain('/'+viewer!.file);await source.click();await expect(page.getByRole('dialog',{name:'Media viewer'})).toBeVisible();
 const html=await readFile('dist/index.html','utf8'),entry=/<script[^>]+src="([^"]+)"/u.exec(html)![1],bytes=await readFile('dist'+entry);expect(gzipSync(bytes).byteLength).toBeLessThanOrEqual(60000);
 await mkdir('../lab/evidence/web/metrics',{recursive:true});await writeFile(`../lab/evidence/web/metrics/WEB-MSG-VIEWER-precache-${info.project.name}.json`,JSON.stringify({entry_chunk:entry,entry_bytes:bytes.byteLength,entry_gzip_bytes:gzipSync(bytes).byteLength,entry_gzip9_bytes:gzipSync(bytes,{level:9}).byteLength,entry_budget_gzip_bytes:60000,trusted_startup_viewer_requests:0,viewer_runtime_cached:true},null,2)+'\n');
});
