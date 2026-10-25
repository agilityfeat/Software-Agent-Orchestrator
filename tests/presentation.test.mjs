import test from 'node:test';
import assert from 'node:assert/strict';
import vm from 'node:vm';
import {Controller} from '../demo/controller.mjs';
import {BASE_SOURCE,EXPENSES,canAccept,digest} from '../demo/core.mjs';
import {SURFACES,canonicalAppearance,appearanceGoals,normalizeAppearancePatch,appearanceChecks,matchProfile,appearanceEvidenceValid,validateAppearanceState} from '../demo/presentation.mjs';

// Synthetic model/VM adapters validate contracts only, never Gemma or browser isolation.
const execute=async(source,inputs)=>inputs.map(input=>JSON.parse(vm.runInNewContext(source+';JSON.stringify(evaluateExpense('+JSON.stringify(input)+'))',Object.create(null),{timeout:100})));
const base=()=>({version:1,surfaces:{
  expenseApp:{background:'#f9fafc',color:'#1f293a',borderColor:'#dce2eb',borderRadius:12},
  receiptDialog:{background:'#172132',color:'#e0e6f0',borderColor:'#52617f',borderRadius:12,fontSize:16,padding:28},
  receiptButton:{background:'#ffffff',color:'#52695d',borderColor:'#d3e1d8',borderRadius:7,fontSize:9},
  dialogButton:{background:'#bba5ff',color:'#20153d',borderColor:'transparent',borderRadius:7,fontSize:12},
  cards:{background:'#ffffff',color:'#1f293a',borderColor:'#e1e7ef',borderRadius:7},
  headings:{color:'#1f293a',fontSize:26},
  tableRows:{color:'#1f293a',fontSize:12,paddingBlock:16},
}});
const goals=[{target:'headings',intent:'adjust'}];
const patch={changes:[{target:'headings',property:'fontSize',value:24}]};
async function ready({request='Make the app heading smaller.',planGoals=goals,code=patch,qa={summary:'Synthetic review of host checks',cases:[]}}={}){
  const roles=[],prompts=[];
  const model={cancel(){},async generate(role,prompt){
    roles.push(role);prompts.push({role,prompt});
    return JSON.stringify(role==='Planner'?{scope:'appearance',summary:'Adjust the requested presentation surface.',steps:['Adjust tokens.','Review executed checks.'],goals:planGoals}:role==='Coder'?code:qa);
  }};
  const controller=new Controller({model,execute,delay:0});
  await controller.plan({request,mode:'live',base:BASE_SOURCE,appearanceBase:base(),appearanceOverrides:{}});
  return {controller,roles,prompts};
}
test('canonical appearance sorts trusted tokens and normalizes colors',()=>{
  const value=base();value.surfaces.expenseApp.background='#F9FAFC';
  const reordered={surfaces:Object.fromEntries(Object.entries(value.surfaces).reverse()),version:1};
  assert.equal(canonicalAppearance(value),canonicalAppearance(reordered));
});
test('presentation patch rejects executable CSS, unknown keys, escapes and duplicate properties',()=>{
  for(const changes of [
    [{target:'headings',property:'color',value:'url(https://example.invalid/)'}],
    [{target:'headings',property:'innerHTML',value:'<script></script>'}],
    [{target:'policy',property:'amountCents',value:1}],
    [{target:'cards',property:'borderRadius',value:20}],
    [{target:'headings',property:'fontSize',value:100}],
    [{target:'headings',property:'fontSize',value:24},{target:'headings',property:'fontSize',value:23}],
  ])assert.throws(()=>normalizeAppearancePatch({changes},base(),goals));
  assert.throws(()=>normalizeAppearancePatch({changes:patch.changes,script:'ignored?'},base(),goals));
});
test('goals must be unique supported typed surfaces',()=>{
  assert.throws(()=>appearanceGoals([]));
  assert.throws(()=>appearanceGoals([{target:'headings',intent:'arbitrary'}]));
  assert.throws(()=>appearanceGoals([goals[0],goals[0]]));
  assert.throws(()=>appearanceGoals([{target:'document',intent:'adjust'}]));
});
test('schema validity and a passing model opinion cannot disguise a no-op or low contrast',()=>{
  const same=JSON.parse(canonicalAppearance(base()));
  assert.equal(appearanceChecks(same,base(),goals).find(row=>row.key==='appearance:changed').pass,false);
  const bad=normalizeAppearancePatch({changes:[{target:'headings',property:'color',value:'#f9fafc'}]},base(),goals);
  assert.equal(appearanceChecks(JSON.parse(bad),base(),goals).find(row=>row.key==='contrast:headings').pass,false);
});
test('match-app checks derive colors from the actual supplied app baseline',()=>{
  const custom=base();custom.surfaces.cards.background='#f0f6f2';custom.surfaces.expenseApp.color='#315e49';
  assert.equal(matchProfile('receiptDialog',custom).background,'#f0f6f2');
  assert.equal(matchProfile('receiptDialog',custom).color,'#315e49');
});
test('a distinct free-form appearance request reaches all model roles without requiring money',async()=>{
  const {controller:c,roles,prompts}=await ready();
  assert.equal(c.task.kind,'appearance');assert.equal(c.task.status,'Ready');assert.equal(c.task.candidate,'');
  assert.ok(prompts[0].prompt.includes('Make the app heading smaller.'));
  await c.run();
  assert.deepEqual(roles,['Planner','Coder','Tester']);
  assert.equal(c.task.status,'Review');assert.ok(canAccept(c.task));
  assert.equal(JSON.parse(c.task.candidate).surfaces.headings.fontSize,24);
  assert.equal(c.task.policySnapshot.source,BASE_SOURCE);
  assert.deepEqual(c.task.policySnapshot.outputs,await execute(BASE_SOURCE,EXPENSES));
  assert.equal(await c.accept(),c.task.candidate);assert.equal(c.task.status,'Done');
});
test('a second generic target uses its approved surface rather than a receipt default',async()=>{
  const {controller:c}=await ready({request:'Give the summary cards rounder corners.',planGoals:[{target:'cards',intent:'adjust'}],code:{changes:[{target:'cards',property:'borderRadius',value:20}]}});
  await c.run();assert.equal(c.task.status,'Review');
  assert.equal(JSON.parse(c.task.candidate).surfaces.cards.borderRadius,20);
  assert.deepEqual(JSON.parse(c.task.candidate).surfaces.receiptDialog,base().surfaces.receiptDialog);
});
test('no-op candidates exhaust bounded repair without reaching acceptance',async()=>{
  const {controller:c}=await ready({code:{changes:[{target:'headings',property:'fontSize',value:26}]}});
  await assert.rejects(()=>c.run(),/repair cycles/i);
  assert.equal(c.task.attempt,3);assert.equal(c.task.status,'Blocked');assert.equal(canAccept(c.task),false);
});
test('invalid Tester scope blocks instead of granting a fabricated verdict',async()=>{
  const {controller:c}=await ready({qa:{summary:'all pass',cases:[{target:'receiptDialog',check:'changed'}]}});
  await assert.rejects(()=>c.run(),/unapproved target|approved surface/);assert.equal(c.task.status,'Blocked');assert.equal(canAccept(c.task),false);
});
test('forged saved pass flags cannot override recomputed appearance checks',async()=>{
  const {controller:c}=await ready();await c.run();
  const bad=JSON.parse(c.task.candidate);bad.surfaces.headings.color='#f9fafc';
  c.task.candidate=canonicalAppearance(bad);c.task.hash=await digest(c.task.candidate);c.task.evidence.hash=c.task.hash;
  c.task.evidence.rows.forEach(row=>row.pass=true);
  assert.equal(appearanceEvidenceValid(c.task),false);assert.equal(canAccept(c.task),false);
  await assert.rejects(()=>c.accept());
});
test('post-test candidate edits reject even when valid style checks still pass',async()=>{
  const {controller:c}=await ready();await c.run();
  const changed=JSON.parse(c.task.candidate);changed.surfaces.headings.fontSize=23;
  c.task.candidate=canonicalAppearance(changed);
  await assert.rejects(()=>c.accept(),/changed|exact passing/i);assert.equal(c.task.status,'Review');
});
test('appearance acceptance independently rejects a changed policy source behind saved pass flags',async()=>{
  const {controller:c}=await ready();await c.run();
  c.task.policySnapshot.source=BASE_SOURCE+'\n// changed after testing\n';
  await assert.rejects(()=>c.accept(),/policy changed|nothing accepted/i);
  assert.notEqual(c.task.status,'Done');
});
for(const action of ['cancel','replace','candidate']){
  test('late appearance acceptance cannot survive '+action,async()=>{
    const {controller:c}=await ready();await c.run();
    let release,entered;const inExecute=new Promise(resolve=>entered=resolve);
    const original=c.execute;c.execute=async(source,inputs)=>{entered();await new Promise(resolve=>release=resolve);return original(source,inputs);};
    const accepting=c.accept();await inExecute;
    if(action==='cancel')c.cancel();
    if(action==='replace')c.task={...c.task,id:'replacement',status:'Ready'};
    if(action==='candidate'){const value=JSON.parse(c.task.candidate);value.surfaces.headings.fontSize=22;c.task.candidate=canonicalAppearance(value);}
    release();await assert.rejects(()=>accepting,/cancelled|changed|nothing accepted/i);
    assert.notEqual(c.task.status,'Done');
  });
}

test('observed quoted numeric model tokens canonicalize to bounded numbers without accepting units',()=>{
  for(const value of ['12','12.5',' 12 ']){
    const candidate=normalizeAppearancePatch({changes:[{target:'headings',property:'fontSize',value}]},base(),goals);
    assert.equal(typeof JSON.parse(candidate).surfaces.headings.fontSize,'number');
  }
  for(const value of ['12px','12%','1e1','NaN','Infinity','-12','+12','0x0c','012','12.999','url(x)','999999999999',NaN,Infinity,null,{},true]){
    assert.throws(()=>normalizeAppearancePatch({changes:[{target:'headings',property:'fontSize',value}]},base(),goals),String(value));
  }
});
test('matching receipt appearance explicitly reviews all three linked receipt surfaces',()=>{
  assert.deepEqual(appearanceGoals([{target:'receiptButton',intent:'match-app'}]).map(goal=>goal.target).sort(),['dialogButton','receiptButton','receiptDialog'].sort());
});
test('saved configuration and overrides must describe the same actual accepted state',()=>{
  const initial=base(),config=JSON.parse(normalizeAppearancePatch(patch,initial,goals));
  assert.equal(validateAppearanceState({base:initial,config,overrides:{headings:{fontSize:24}}}).config.surfaces.headings.fontSize,24);
  assert.throws(()=>validateAppearanceState({base:initial,config,overrides:{headings:{fontSize:23}}}),/do not match/);
  assert.throws(()=>validateAppearanceState({base:initial,config,overrides:{headings:{fontSize:'12px'}}}));
});
test('one synthetic-model formatting correction retains both raw QA responses and host checks',async()=>{
  const {controller:c,roles}=await ready();let qaCalls=0;
  const generate=c.model.generate.bind(c.model);
  c.model.generate=async(role,prompt)=>{
    if(role!=='Tester')return generate(role,prompt);
    roles.push(role);qaCalls++;
    return qaCalls===1?'{"summary":"Malformed first QA","cases":[{"target":"wrong","check":"pass"}]}':'{"summary":"Host checks reviewed","cases":[]}';
  };
  await c.run();
  assert.equal(qaCalls,2);assert.equal(c.task.qaResponses.length,2);
  assert.ok(c.task.qaResponses[0].rawResponse.includes('Malformed first QA'));
  assert.equal(c.task.status,'Review');assert.ok(canAccept(c.task));
  assert.ok(c.task.evidence.rows.every(row=>row.pass));
});
test('QA formatting retry never converts a failing no-op into a pass',async()=>{
  const {controller:c}=await ready({code:{changes:[{target:'headings',property:'fontSize',value:26}]}});
  let count=0;const generate=c.model.generate.bind(c.model);
  c.model.generate=async(role,prompt)=>role!=='Tester'?generate(role,prompt):(++count%2?'not JSON':'{"summary":"all pass","cases":[]}');
  await assert.rejects(()=>c.run(),/repair cycles/i);
  assert.equal(c.task.qaResponses.length,6);assert.equal(c.task.status,'Blocked');
  assert.ok(c.task.evidence.rows.some(row=>!row.pass));assert.equal(canAccept(c.task),false);
});
