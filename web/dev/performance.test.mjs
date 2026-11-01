import assert from 'node:assert/strict';
import {traceWork,resultFrames} from '../tests/list-performance.ts';
import test from 'node:test';
test('paint costs are attributed once to the correct main-thread frame',()=>{
const events=[];
for(let index=0;index<=100;index++)events.push({name:'list-frame-'+index,ts:index*10000,pid:1,tid:1,ph:'I'});
events.push(...[
 {name:'FunctionCall',ts:0,dur:500,pid:1,tid:1,ph:'X'},
 {name:'Layout',ts:100,dur:300,pid:1,tid:1,ph:'X'},
 {name:'PrePaint',ts:500,dur:200,pid:1,tid:1,ph:'X'},
 {name:'Paint',ts:700,dur:1000,pid:1,tid:1,ph:'X'},
 {name:'Layerize',ts:1700,dur:300,pid:1,tid:1,ph:'X'},
 {name:'Paint',ts:700,dur:9000,pid:1,tid:2,ph:'X'},
 {name:'Paint',ts:9900,dur:300,pid:1,tid:1,ph:'X'},
]);
const result=traceWork(JSON.stringify({traceEvents:events}));
assert.equal(result.frames[0],2.1);assert.equal(result.frames[1],.2);assert.equal(result.frame_work.samples,100);
});
test('result frames include rendered work and exclude the already measured input dispatch',()=>{
const events=[];
for(let index=0;index<=48;index++)events.push({name:'AnimationFrame',ts:index*10000,pid:1,tid:1,ph:'e'},{name:'AnimationFrame',ts:index*10000+500,pid:1,tid:1,ph:'b'});
for(let index=0;index<24;index++){
 const at=index*20000;
 events.push(...[
  {name:'list-input-event-'+index,ts:at+1100,pid:1,tid:1,ph:'I'},
  {name:'list-input-frame-start-'+index,ts:at+2500,pid:1,tid:1,ph:'I'},
  {name:'list-input-result-'+index,ts:at+3000,pid:1,tid:1,ph:'I'},
  {name:'list-input-frame-end-'+index,ts:at+15000,pid:1,tid:1,ph:'I'},
  {name:'EventDispatch',ts:at+1000,dur:1000,pid:1,tid:1,ph:'X'},
  {name:'FunctionCall',ts:at+5000,dur:2000,pid:1,tid:1,ph:'X'},
  {name:'Layout',ts:at+6000,dur:500,pid:1,tid:1,ph:'X'},
  {name:'Paint',ts:at+8000,dur:500,pid:1,tid:1,ph:'X'},
 ]);
}
assert.deepEqual(resultFrames(JSON.stringify({traceEvents:events})).frames,Array(24).fill(2.5));
assert.throws(()=>resultFrames(JSON.stringify({traceEvents:events.filter(event=>event.name!=='list-input-result-23')})),/result marks missing/);
});

test('native frame boundaries include post-render paint and exclude the preceding paint',()=>{
 const events=[];
 for(let index=0;index<=100;index++)events.push(
  {name:'list-frame-'+index,ts:index*10000+1000,pid:1,tid:1,ph:'I'},
  {name:'AnimationFrame',ts:index*10000+3000,pid:1,tid:1,ph:'e'},
  {name:'Paint',ts:index*10000+3100,dur:600,pid:1,tid:1,ph:'X'},
  {name:'Layerize',ts:index*10000+3700,dur:300,pid:1,tid:1,ph:'X'},
  {name:'AnimationFrame',ts:index*10000+4100,pid:1,tid:1,ph:'b'},
 );
 for(let index=0;index<24;index++){
  const at=index*20000;
  events.push(
   {name:'list-input-event-'+index,ts:at+10100,pid:1,tid:1,ph:'I'},
   {name:'EventDispatch',ts:at+10000,dur:1000,pid:1,tid:1,ph:'X'},
   {name:'list-input-frame-start-'+index,ts:at+11500,pid:1,tid:1,ph:'I'},
   {name:'list-input-result-'+index,ts:at+12000,pid:1,tid:1,ph:'I'},
   {name:'list-input-frame-end-'+index,ts:at+21000,pid:1,tid:1,ph:'I'},
   {name:'FunctionCall',ts:at+11500,dur:500,pid:1,tid:1,ph:'X'},
   {name:'Paint',ts:at+13200,dur:200,pid:1,tid:2,ph:'X'},
  );
 }
 const raw=JSON.stringify({traceEvents:events});
 assert.deepEqual(resultFrames(raw).frames,Array(24).fill(1.4));
 const scroll=traceWork(raw,'scroll',true);
 assert.equal(scroll.frames.length,101);
 assert.equal(scroll.frames[0],.9);
 assert.equal(scroll.frames[1],2.4);
 assert.throws(()=>resultFrames(JSON.stringify({traceEvents:events.filter(event=>event.ph!=='b')})),/result frame missing/);
});
