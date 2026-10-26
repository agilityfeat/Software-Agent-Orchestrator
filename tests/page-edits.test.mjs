import test from 'node:test';
import assert from 'node:assert/strict';
import {applyEdits} from '../demo/page-contract.mjs';
const base={html:'<main>Before</main>',css:'',js:"const items=[\n{id:'first',date:'2026-10-14'}\n];"};
test('a model no-op is diagnosed before its whitespace-mismatched find can obscure the actual problem',()=>{
 const flattened=base.js.replaceAll('\n','');
 assert.throws(()=>applyEdits(base,{edits:[{file:'js',find:flattened,replace:flattened}]}),/changes nothing; omit it/);
 assert.equal(base.html,'<main>Before</main>');
});
test('failed exact edits give bounded offending text and source context without partially applying earlier edits',()=>{
 const before=structuredClone(base);
 assert.throws(()=>applyEdits(base,{edits:[{file:'html',find:'Before',replace:'After'},{file:'js',find:base.js.replaceAll('\n',''),replace:'const items=[];'}]}),error=>error.message.includes('find text was not found')&&error.message.includes('const items=')&&error.message.includes('Nearby current source:')&&error.message.length<900);
 assert.deepEqual(base,before);
});
