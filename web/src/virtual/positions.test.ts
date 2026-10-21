import { test } from 'node:test';
import assert from 'node:assert/strict';
import { Positions } from './positions.ts';
test('100000 variable positions agree with independent prefix sums after measurement',()=>{const values=Array.from({length:100000},(_,i)=>20+i%97);const positions=new Positions(values);for(let i=0;i<values.length;i+=131){values[i]+=33;positions.measure(i,values[i]);}let sum=0;for(let i=0;i<values.length;i++){assert.equal(positions.offset(i),sum);assert.equal(positions.indexAt(sum),i);assert.equal(positions.indexAt(sum+values[i]-.01),i);sum+=values[i];}assert.equal(positions.total,sum);assert.equal(positions.indexAt(sum+100),values.length-1);});
