import test from 'node:test';
import assert from 'node:assert/strict';
import {validateChecks} from '../demo/page-checks.mjs';
const check=assertion=>[{name:'Column label',assertions:[{selector:'thead th',property:'text',operator:'equals',expected:'Date',...assertion}]}];
test('node quantifiers preserve deliberate any, all and first data',()=>{
  for(const flag of [{},{all:true},{all:false},{first:true},{first:false},{all:true,first:false},{all:false,first:true}])assert.deepEqual(validateChecks(check(flag)),[{...check(flag)[0],steps:[]}]);
});
test('node quantifiers reject conflicting or untyped selectors',()=>{
  for(const flag of [{all:true,first:true},{first:'true'},{first:1},{first:null}])assert.throws(()=>validateChecks(check(flag)));
});
