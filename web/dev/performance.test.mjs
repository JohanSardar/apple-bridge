import assert from 'node:assert/strict';
import {traceWork} from '../tests/list-performance.ts';
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
