// Generic contract regressions; synthetic model responses are NON_ACCEPTANCE for inference.
import test from 'node:test';
import assert from 'node:assert/strict';
import {modelChecks,validatePlan,CHECK_SEMANTICS} from '../demo/page-contract.mjs';
import {Controller} from '../demo/page-controller.mjs';
const state=(expected,extra={})=>({name:'Declared state',selector:'#feature',property:'visible',operator:'equals',expected,...extra});
const plan=checks=>({summary:'Add an interactive feature.',steps:['Build the requested interaction.'],checks});
test('contradictory single observations need explicit scenario actions before approval',()=>{
 for(const selector of ['#feature','body','html',':root'])assert.throws(()=>modelChecks([state(false,{selector}),state(true,{selector})]),/contradict.*explicit click\/actions/);
 for(const flag of [{first:true},{all:true}])assert.throws(()=>modelChecks([state(false,{selector:'.items',...flag}),state(true,{selector:'.items',...flag})]),/contradict/);
 assert.throws(()=>modelChecks([state(1,{selector:'.items',property:'count'}),state(2,{selector:'.items',property:'count'})]),/contradict/);
});
test('different actions, properties, targets and heterogeneous any-node values remain supported',()=>{
 for(const checks of [
  [state(false),state(true,{click:'#toggle'})],
  [state(false),state(true,{actions:[{action:'click',selector:'#toggle'}]})],
  [state(false),state(true,{selector:'#other'})],
  [state(false),state(true,{property:'checked'})],
  [state(false,{selector:'.items'}),state(true,{selector:'.items'})],
  [state('Alpha',{selector:'.items',property:'text'}),state('Beta',{selector:'.items',property:'text'})],
  [state(false,{selector:'.items',first:true}),state(true,{selector:'.items',all:true})],
  [state(false),state(false)],
 ])assert.equal(modelChecks(checks).length,2);
 assert.equal(validatePlan(plan([state(true)])).checkSemantics,CHECK_SEMANTICS);
 assert.equal(CHECK_SEMANTICS,'any-node-isolated-v2');
});
test('Planner receives one explicit action correction and invalid checks never reach the browser',async()=>{
 let generations=0,executions=0,correction='';
 const controller=new Controller({model:{cancel(){},async generate(role,prompt){generations++;correction=prompt;return JSON.stringify(plan(generations===1?[state(false),state(true)]:[state(false),state(true,{click:'#toggle'})]));}},execute:async()=>{executions++;return {rows:[],errors:[]};}});
 await controller.plan({request:'Add an interactive feature.',mode:'live',base:{html:'<main>Initial app</main>',css:'',js:''}});
 assert.equal(generations,2);assert.equal(executions,1);assert.match(correction,/Supply explicit click\/actions/);assert.equal(controller.task.status,'Ready');assert.equal(controller.task.plan.checkSemantics,CHECK_SEMANTICS);
});

test('unsupported assertion correction names the rejected field and never executes invalid checks',async()=>{
 let generations=0,executions=0,correction='';
 const controller=new Controller({model:{cancel(){},async generate(role,prompt){generations++;correction=prompt;return JSON.stringify(plan([state(true,{property:generations===1?'enabled':'disabled'})]));}},execute:async()=>{executions++;return {rows:[],errors:[]};}});
 await controller.plan({request:'Change an approval rule.',mode:'live',base:{html:'<button id="feature">Approve</button>',css:'',js:''}});
 assert.equal(generations,2);assert.equal(executions,1);assert.match(correction,/Declared state.*assertion 1.*property "enabled"/);assert.match(correction,/Use disabled\/equals\/true or false/);assert.equal(controller.task.status,'Ready');assert.equal(controller.task.plan.checks[0].assertions[0].property,'disabled');
});

test('change plans cannot use checks that already pass, including when a trailing runtime row fails',async()=>{
 for(const runtimePass of [true,false]){
  let generations=0,prompt='';const c=new Controller({model:{cancel(){},async generate(role,value){generations++;prompt=value;return JSON.stringify(plan([state(true)]));}},execute:async()=>({rows:[{name:'Declared state',pass:true,actual:['Existing label']},{name:'Page runs without browser errors',pass:runtimePass}],errors:runtimePass?[]:['Existing runtime error'],observations:{text:'Existing rendered label',html:'<main>Existing rendered label</main>'}})});
  await assert.rejects(c.plan({request:'Change the displayed rule.',mode:'live',base:{html:'<main>Existing label</main>',css:'',js:''}}),/already pass on the accepted app/);
  assert.equal(generations,2);assert.equal(c.task.status,'Blocked');assert.match(prompt,/Existing rendered label/);assert.match(prompt,/Do not switch a behavior-changing request/);
 }
});
test('one changed outcome can fail alongside passing preservation checks before approval',async()=>{
 let calls=0;const c=new Controller({model:{cancel(){},async generate(){calls++;return JSON.stringify(plan([state(true),state(false,{selector:'#preserved'})]));}},execute:async()=>({rows:[{name:'Declared state',pass:false},{name:'Declared state',pass:true},{name:'Page runs without browser errors',pass:true}],errors:[]})});
 await c.plan({request:'Show the requested feature and preserve another panel.',mode:'live',base:{html:'<main>Initial app</main>',css:'',js:''}});assert.equal(calls,1);assert.equal(c.task.plan.verification,'change');assert.equal(c.task.status,'Ready');
});
test('explicit preserve plans support behavior-preserving refactors with all baseline checks passing',async()=>{
 const c=new Controller({model:{cancel(){},async generate(){return JSON.stringify({...plan([state(true)]),verification:'preserve'});}},execute:async()=>({rows:[{name:'Declared state',pass:true},{name:'Page runs without browser errors',pass:true}],errors:[]})});
 await c.plan({request:'Refactor the render helper without changing appearance or behavior.',mode:'live',base:{html:'<main>Initial app</main>',css:'',js:''}});assert.equal(c.task.plan.verification,'preserve');assert.equal(c.task.status,'Ready');
 for(const verification of ['anything',null,false])assert.throws(()=>validatePlan({...plan([state(true)]),verification}),/verification must be change or preserve/);
});

test('Planner gets one reasoning correction without echoing rejected JSON and blocks repeated invalid checks',async()=>{
 for(const repairSucceeds of [true,false]){
  const calls=[];let executions=0;const c=new Controller({model:{cancel(){},async generate(role,prompt,options){calls.push({role,prompt,options});return JSON.stringify({...plan([state(true,{property:calls.length===1||!repairSucceeds?'enabled':'disabled'})]),note:'REJECTED_RAW_SENTINEL'});}},execute:async()=>{executions++;return {rows:[],errors:[]}}});
  const pending=c.plan({request:'Change the requested control behavior.',mode:'live',base:{html:'<button id="feature">Existing control</button>',css:'',js:''}});
  if(repairSucceeds)await pending;else await assert.rejects(pending,/property "enabled"/);
  assert.deepEqual(calls.map(call=>call.options),[{repair:false},{repair:true}]);assert.ok(calls.every(call=>call.role==='Planner'));assert.match(calls[1].prompt,/Generate a fresh corrected complete plan/);assert.match(calls[1].prompt,/property "enabled"/);assert.doesNotMatch(calls[1].prompt,/REJECTED_RAW_SENTINEL/);
  assert.equal(executions,repairSucceeds?1:0);assert.equal(c.task.status,repairSucceeds?'Ready':'Blocked');assert.equal(c.task.plannerResponses.length,2);assert.match(c.task.plannerResponses[0],/REJECTED_RAW_SENTINEL/);
 }
});
