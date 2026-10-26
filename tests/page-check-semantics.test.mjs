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

test('disabled assertions are typed booleans and invalid schema names identify the exact check',()=>{
 for(const expected of [true,false])assert.equal(validateChecks([{name:'Approval state',assertions:[{selector:'#approve',property:'disabled',operator:'equals',expected}]}])[0].assertions[0].expected,expected);
 for(const expected of ['true',1,null])assert.throws(()=>validateChecks([{name:'Approval state',assertions:[{selector:'#approve',property:'disabled',operator:'equals',expected}]}]),/Approval state.*assertion 1.*Boolean checks/);
 for(const [field,value,pattern] of [['property','unknown',/property "unknown".*disabled/],['operator','not-includes',/operator "not-includes".*equals/]])assert.throws(()=>validateChecks([{name:'Approval state',assertions:[{selector:'#approve',property:'text',operator:'equals',expected:'Review',[field]:value}]}]),error=>/Approval state.*assertion 1/.test(error.message)&&pattern.test(error.message));
});
