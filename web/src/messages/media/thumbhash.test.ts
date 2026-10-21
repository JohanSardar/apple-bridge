import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { decodeThumbHash } from './thumbhash.ts';
test('ThumbHash decoder rejects malformed and truncated input',()=>{
    for(const value of ['', '!', 'YWJj', 'AAAAAAA=', 'A'.repeat(140)])assert.equal(decodeThumbHash(value),undefined);
});
test('ThumbHash decoder consumes the Swift encoder fixtures with bounded geometry',()=>{
    const fixtures=JSON.parse(readFileSync('lab/fixtures/media/thumbhash.json','utf8')) as {hash:string;width:number;height:number}[];
    for(const fixture of fixtures){const decoded=decodeThumbHash(fixture.hash)!;assert.ok(decoded);assert.equal(decoded.rgba.length,decoded.width*decoded.height*4);assert.equal(Math.max(decoded.width,decoded.height),32);assert.ok(Math.abs(decoded.width/decoded.height-fixture.width/fixture.height)<.25);}
});
test('zero AC scales yield a constant image, independently of packed AC values',()=>{
    const hash=new Uint8Array(32);hash.set([32,8,2,7,0]);hash.fill(255,5);
    const decoded=decodeThumbHash(Buffer.from(hash).toString('base64'))!;
    assert.ok(decoded);assert.equal(decoded.rgba[3],255);
    for(let index=0;index<decoded.rgba.length;index++)assert.equal(decoded.rgba[index],decoded.rgba[index%4]);
    for(const value of decoded.rgba.slice(0,3))assert.ok(value>120&&value<140);
});
