import test from 'node:test';
import assert from 'node:assert/strict';
import vm from 'node:vm';
import {readFile} from 'node:fs/promises';
import {BASE_SOURCE,EXPENSES,policy,expected,criteria,requiredCases,score,digest,extractJSON,extractCode,validatePlan,extraCases,canAccept,fixtureSource,baselineOutputs,checkOutput} from '../demo/core.mjs';
import {Controller} from '../demo/controller.mjs';
// A test-only adapter for controller/contract tests. Not the production sandbox.
const execute=async(source,inputs)=>inputs.map(input=>JSON.parse(vm.runInNewContext(`${source};JSON.stringify(evaluateExpense(${JSON.stringify(input)}))`,Object.create(null),{timeout:100})));
const rule=policy('500');
const make=(options={})=>new Controller({model:{cancel(){}},execute,delay:0,...options});
const plan=c=>c.plan({request:'Add approval thresholds.',rule,mode:'guided'});
for(const [input,cents]of [['500',50000],['0.01',1],['500.01',50001],['999999.99',99999999]])test(`money parses ${input} as exact cents`,()=>assert.equal(policy(input).thresholdCents,cents));
for(const input of ['-1','0','1e3','NaN','Infinity','10.999','',' 500','1000000'])test(`invalid amount rejected: ${input}`,()=>assert.throws(()=>policy(input)));
test('receipt policy is optional',()=>assert.equal(rule.receiptCents,null));
test('receipt policy uses integer cents',()=>assert.equal(policy('500','75.10').receiptCents,7510));
const unsolicitedReceiptSource=fixtureSource(rule).replace("  return {status:'eligible'", "  if (expense.receipt === false) return {status:'receipt_required',reason:'Missing receipt',canApprove:false};\n  return {status:'eligible'");
test('default criteria explicitly prohibit an unrequested receipt rule',()=>assert.match(criteria(rule).join(' '),/No receipt requirement is enabled/));
test('required default cases reject unsolicited receipt requirements and protect displayed expenses',async()=>{
  const cases=requiredCases(rule),rows=score(cases,await execute(unsolicitedReceiptSource,cases.map(c=>c.input)),rule);
  assert.deepEqual(rows.filter(row=>!row.pass).map(row=>row.name),['No receipt rule · below threshold','No receipt rule · exact boundary']);
  const appCases=EXPENSES.map(input=>({name:input.merchant,input}));
  const appRows=score(appCases,await execute(unsolicitedReceiptSource,EXPENSES),rule);
  assert.deepEqual(appRows.filter(row=>!row.pass).map(row=>row.name),['Studio Supply','Design Tools']);
  const correct=await execute(fixtureSource(rule),EXPENSES);
  assert.equal(correct[2].status,'eligible');assert.equal(correct[5].status,'eligible');assert.equal(correct[4].status,'paid');
});
test('a live candidate adding an unrequested receipt rule cannot reach acceptance',async()=>{
  const c=make({model:{cancel(){},async generate(role,prompt){
    if(role==='Planner')return JSON.stringify({summary:'Synthetic regression',steps:['Implement']});
    if(role==='Coder'){assert.match(prompt,/No receipt requirement is enabled/);return unsolicitedReceiptSource;}
    return JSON.stringify({summary:'Review actual checks',cases:[]});
  }}});
  await c.plan({request:'Add approval threshold only',rule,mode:'live'});
  await assert.rejects(()=>c.run(),/Two repair/);
  assert.equal(c.task.status,'Blocked');assert.equal(canAccept(c.task),false);
  await assert.rejects(()=>c.accept());
});

test('all boundary and paid tests execute and pass for correct source',async()=>{const cases=requiredCases(rule);const rows=score(cases,await execute(fixtureSource(rule),cases.map(c=>c.input)),rule);assert.equal(rows.length,9);assert.ok(rows.every(r=>r.pass));});
test('wrong >= comparison is caught by exact-boundary test',async()=>{const cases=requiredCases(rule);const rows=score(cases,await execute(fixtureSource(rule,true),cases.map(c=>c.input)),rule);assert.equal(rows.filter(r=>!r.pass).length,2);assert.match(rows.find(r=>!r.pass).name,/Exact boundary/);});
test('receipt rule includes boundary, missing and historical tests',async()=>{const r=policy('500','75'),cases=requiredCases(r);assert.equal(cases.length,8);assert.ok(score(cases,await execute(fixtureSource(r),cases.map(c=>c.input)),r).every(row=>row.pass));});
test('receipt precedence is explicit, paid is immutable',()=>{const r=policy('500','75');assert.equal(expected({amountCents:90000,receipt:false,status:'submitted'},r),'receipt_required');assert.equal(expected({amountCents:90000,receipt:false,status:'paid'},r),'paid');});
test('baseline renderer matches executable baseline',async()=>assert.deepEqual(await execute(BASE_SOURCE,EXPENSES),baselineOutputs()));
test('nonempty reason and action boolean are mandatory',()=>{assert.ok(!checkOutput({status:'eligible',reason:'',canApprove:true}));assert.ok(!checkOutput({status:'eligible',reason:'ok',canApprove:'true'}));assert.ok(!checkOutput({status:'eligible',reason:'x'.repeat(301),canApprove:true}));});
test('fake status or wrong canApprove cannot pass',()=>{const cases=requiredCases(rule);assert.ok(score(cases,cases.map(()=>({status:'eligible',reason:'ok',canApprove:false})),rule).some(r=>!r.pass));assert.throws(()=>score(cases,[],rule));});
test('JSON fenced responses parse; commentary does not',()=>{assert.deepEqual(extractJSON('```json\n{"ok":true}\n```'),{ok:true});assert.throws(()=>extractJSON('Here you go: {"ok":true}'));});
test('source output must contain the contract and stay bounded',()=>{assert.equal(extractCode('```js\n'+BASE_SOURCE+'\n```'),BASE_SOURCE);assert.throws(()=>extractCode('hello'));assert.throws(()=>extractCode(BASE_SOURCE+'x'.repeat(16000)));});
test('planner schema rejects missing or excessive steps',()=>{assert.throws(()=>validatePlan({summary:'x',steps:[]}));assert.throws(()=>validatePlan({summary:'x',steps:[1]}));assert.equal(validatePlan({summary:'x',steps:['a']}).steps.length,1);});
test('QA can only supply valid data, never replace assertions',()=>{assert.throws(()=>extraCases({summary:'x',cases:[{amountCents:-1,status:'paid',receipt:true}]}));assert.throws(()=>extraCases({summary:'x',cases:Array(7).fill({})}));const cases=extraCases({summary:'x',cases:[{amountCents:10,status:'submitted',receipt:true,expected:'paid',code:'arbitrary'}]});assert.equal(cases[0].input.code,undefined);assert.equal(cases[0].input.expected,undefined);});
test('guided example genuinely fails then repairs and requires acceptance',async()=>{const c=make();await plan(c);assert.equal(c.task.status,'Ready');assert.equal(c.task.candidate,'');await c.run();assert.equal(c.task.status,'Review');assert.equal(c.task.attempt,2);assert.equal(c.task.versions.length,2);assert.ok(c.task.versions[0].evidence.rows.some(r=>!r.pass));assert.ok(canAccept(c.task));assert.equal(await c.accept(),fixtureSource(rule));assert.equal(c.task.status,'Done');});
test('cannot accept before tests or with changed source',async()=>{const c=make();await plan(c);await assert.rejects(()=>c.accept());await c.run();c.task.candidate+='\n// edit';await assert.rejects(()=>c.accept(),/changed/);});
test('stale passing evidence without current no-receipt cases cannot be accepted',async()=>{const c=make();await plan(c);await c.run();c.task.evidence.rows=c.task.evidence.rows.filter(row=>row.input.receipt);assert.equal(c.task.evidence.rows.length,5);assert.equal(canAccept(c.task),false);await assert.rejects(()=>c.accept());});
test('wrong evidence hash and recorded runs cannot be accepted',async()=>{const c=make();await plan(c);await c.run();c.task.evidence.hash='stale';assert.equal(canAccept(c.task),false);c.task.evidence.hash=c.task.hash;c.task.mode='recorded';assert.equal(canAccept(c.task),false);});
test('persistent failures stop after two repairs',async()=>{const c=make({execute:async(_,inputs)=>inputs.map(()=>({status:'paid',reason:'Wrong',canApprove:false}))});await plan(c);await assert.rejects(()=>c.run(),/Two repair/);assert.equal(c.task.attempt,3);assert.equal(c.task.versions.length,3);assert.equal(c.task.status,'Blocked');assert.equal(canAccept(c.task),false);});
test('candidate syntax errors are evidence and get bounded repairs',async()=>{const c=make({execute:async()=>{throw Error('Syntax failure');}});await plan(c);await assert.rejects(()=>c.run(),/Two repair/);assert.ok(c.task.evidence.rows.every(r=>!r.pass));});
test('live roles use the shared model sequentially, not fixture agents',async()=>{const roles=[];const model={cancel(){},async generate(role,prompt){roles.push(role);if(role==='Tester'){assert.match(prompt,/ALREADY EXECUTED the following 9 protected test cases/);assert.match(prompt,/Never copy the protected rows into cases/);assert.match(prompt,/"cases":\[\]/);}return role==='Planner'?JSON.stringify({summary:'Live test adapter',steps:['Implement']}):role==='Coder'?fixtureSource(rule):JSON.stringify({summary:'Checks reviewed',cases:[{amountCents:2,status:'submitted',receipt:true}]});}};const c=make({model});await c.plan({request:'Test',rule,mode:'live'});await c.run();assert.deepEqual(roles,['Planner','Coder','Tester']);assert.equal(c.task.evidence.rows.length,10);assert.equal(c.task.attempt,1);});
test('invalid QA output blocks rather than inventing a verdict',async()=>{const c=make({model:{cancel(){},async generate(role){return role==='Planner'?'{"summary":"x","steps":["x"]}':role==='Coder'?fixtureSource(rule):'{"summary":"all pass"}';}}});await c.plan({request:'Test',rule,mode:'live'});await assert.rejects(()=>c.run(),/QA cases must/);assert.equal(c.task.status,'Blocked');assert.equal(canAccept(c.task),false);});
test('QA validation identifies each schema failure without returning model text',()=>{
  assert.throws(()=>extraCases(null),/QA summary must be a string/);
  assert.throws(()=>extraCases({summary:{untrusted:'content'},cases:[]}),/QA summary must be a string/);
  assert.throws(()=>extraCases({summary:'Review',cases:{}}),/QA cases must be an array/);
  assert.throws(()=>extraCases({summary:'Review',cases:Array(9).fill({})}),/QA returned 9 additional cases; the maximum is 6/);
  assert.throws(()=>extraCases({summary:'Review',cases:[null]}),/QA case 1 must contain/);
  const input={amountCents:1,status:'submitted',receipt:false};
  assert.equal(extraCases({summary:'Review',cases:Array(6).fill(input)}).length,6);
});
test('QA repeating the nine protected rows blocks instead of truncating or inventing a pass',async()=>{
  const c=make({model:{cancel(){},async generate(role){
    return role==='Planner'?JSON.stringify({summary:'Synthetic regression',steps:['Implement']}):
      role==='Coder'?fixtureSource(rule):JSON.stringify({summary:'Protected results repeated',cases:requiredCases(rule).map(test=>test.input)});
  }}});
  await c.plan({request:'Test',rule,mode:'live'});
  await assert.rejects(()=>c.run(),/QA returned 9 additional cases; the maximum is 6/);
  assert.equal(c.task.status,'Blocked');assert.equal(c.task.evidence,null);assert.equal(canAccept(c.task),false);
});
test('cancellation fences late planner completion',async()=>{let release;const c=make({model:{cancel(){},generate(){return new Promise(resolve=>{release=resolve;});}}});const promise=c.plan({request:'x',rule,mode:'live'});c.cancel();release('{"summary":"late","steps":["x"]}');await assert.rejects(()=>promise,/cancelled/);assert.equal(c.task.status,'Blocked');assert.equal(c.task.plan,null);});
test('duplicate starts are rejected',async()=>{const c=make();await plan(c);const pending=c.run();await assert.rejects(()=>c.run(),/Approve/);await pending;});
test('SHA-256 binds evidence to source',async()=>{assert.equal((await digest(BASE_SOURCE)).length,64);assert.notEqual(await digest(BASE_SOURCE),await digest(BASE_SOURCE+' '));});
test('deployment uploads only demo, with no PR deployment trigger',async()=>{const yml=await readFile(new URL('../.github/workflows/demo-pages.yml',import.meta.url),'utf8');assert.match(yml,/path: demo/);assert.doesNotMatch(yml,/pull_request/);assert.match(yml,/pages: write/);assert.match(yml,/refs\/heads\/main/);});
test('model SDK and WASM versions are pinned together',async()=>{const source=await readFile(new URL('../demo/model-worker.mjs',import.meta.url),'utf8');assert.match(source,/@litert-lm\/core@0\.18\.0/);assert.match(source,/\$\{SDK\}\/wasm\//);assert.match(source,/enable_thinking:false/);});
test('UI uses relative assets and never evaluates generated source in the DOM',async()=>{const html=await readFile(new URL('../demo/index.html',import.meta.url),'utf8'),app=await readFile(new URL('../demo/app.mjs',import.meta.url),'utf8');assert.match(html,/src="\.\/app.mjs"/);assert.doesNotMatch(app,/innerHTML|new Function|eval\(/);});
