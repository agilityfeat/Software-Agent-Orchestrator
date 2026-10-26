// Independent contract regressions. NON_ACCEPTANCE for real Gemma/browser outcomes.
import test from 'node:test';
import assert from 'node:assert/strict';
import {safePattern,validateChecks} from '../demo/page-checks.mjs';
const assertion={selector:'#new-feature',property:'text',operator:'equals',expected:'Ready'};
const check={name:'A generic user-visible feature',steps:[],assertions:[assertion]};
test('browser check contract accepts arbitrary feature selectors and actual interactive actions',()=>{
 const input=[{name:'Any app form',steps:[{action:'input',selector:'[name="new-item"]',value:'Read a book'},{action:'key',selector:'[name="new-item"]',key:'Enter'},{action:'click',selector:'.items button'},{action:'select',selector:'#sort',value:'new'},{action:'wait',ms:40}],assertions:[{selector:'.items li',property:'count',operator:'equals',expected:0},{selector:'.notice',property:'visible',operator:'equals',expected:true},{selector:'#toggle',property:'checked',operator:'equals',expected:false}]}];
 assert.deepEqual(validateChecks(input),input);
});
test('computed styles are data-only checks for arbitrary CSS properties',()=>{
 const input=[{...check,assertions:[{selector:'.new-card',property:'style',attribute:'border-radius',operator:'equals',expected:'17px'}]}];
 assert.deepEqual(validateChecks(input),input);
});
test('missing or empty model checks cannot create trusted evidence',()=>{
 for(const value of [null,undefined,{},[],Array(7).fill(check),[{...check,assertions:[]}],[{...check,name:''}],[{...check,assertions:'pass'}]])assert.throws(()=>validateChecks(value));
});
test('browser check values and actions remain typed and bounded',()=>{
 for(const bad of [
 {...assertion,property:'eval',expected:'true'}, {...assertion,selector:''}, {...assertion,selector:'x'.repeat(301)},
 {...assertion,property:'count',expected:'1'}, {...assertion,property:'count',expected:-1}, {...assertion,property:'count',expected:10001},
 {...assertion,property:'visible',expected:'true'}, {...assertion,property:'visible',operator:'includes',expected:true},
 {...assertion,expected:{toString:'Ready'}}, {...assertion,expected:'x'.repeat(1001)}, {...assertion,all:'true'},
 {...assertion,property:'attribute',attribute:'onclick()'}, {...assertion,property:'style',attribute:'color;display'},
 ])assert.throws(()=>validateChecks([{...check,assertions:[bad]}]));
 for(const step of [{action:'eval',value:'true'},{action:'wait',ms:1001},{action:'wait',ms:-1},{action:'wait',ms:'10'},{action:'input',selector:'#x',value:123},{action:'key',selector:'#x',key:'F12'}])assert.throws(()=>validateChecks([{...check,steps:[step]}]));
 assert.throws(()=>validateChecks([{...check,steps:Array(9).fill({action:'wait',ms:0})}]));
 assert.throws(()=>validateChecks([{...check,assertions:Array(13).fill(assertion)}]));
});
test('check normalization copies only data and never retains a pass flag or executable payload',()=>{
 const normalized=validateChecks([{...check,pass:true,code:'throw Error("not executed")',assertions:[{...assertion,pass:true,eval:'anything'}]}]);
 assert.deepEqual(normalized,[check]);
});
test('patterns reject repetition chains, backreferences and unsupported constructs before execution',()=>{
 for(const value of ['^'+'a?'.repeat(35)+'a'.repeat(35)+'$', '.*a.*b','(a+)+$','a|b','a{1,100000}','(x)\\1','(?=x)','[','x'.repeat(161)])assert.throws(()=>safePattern(value),value);
 for(const value of ['^[0-9]+$','^Ready$','feature.*','item\\?'])assert.ok(safePattern(value) instanceof RegExp,value);
});

import {canonicalBundle,validateBundle,applyEdits,canAccept,digest} from '../demo/page-contract.mjs';
import {Controller} from '../demo/page-controller.mjs';
const ORIGINAL={html:'<main><h1 id="feature">Before</h1></main>',css:'main{color:#123456}',js:''};
const REPLACEMENT={html:'<main><h1 id="feature">After</h1><button id="new">Any new feature</button></main>',css:'main{color:#654321}',js:'document.querySelector("#new").onclick=()=>document.body.dataset.clicked="yes";'};
const PLAN={summary:'Create a generic new feature.',steps:['Implement requested files.','Check actual behavior.'],checks:[{name:'Requested heading',steps:[],assertions:[{selector:'#feature',property:'text',operator:'equals',expected:'After'}]}]};
// Deliberately controlled result adapter: only contract/lifecycle proof. Browser suite executes real DOM.
const result=(bundle,checks)=>({rows:checks.map(check=>({name:check.name,expected:check.assertions,actual:[],pass:bundle.html.includes('After')})),errors:[],observations:{text:bundle.html}});
const model=({qa={summary:'Synthetic contract review, not inference.',cases:[]},code={files:REPLACEMENT}}={})=>({cancel(){},async generate(role){return JSON.stringify(role==='Planner'?PLAN:role==='Coder'?code:qa);}});
async function prepared(options={}){const c=new Controller({model:model(),execute:async(...args)=>result(...args),...options});await c.plan({request:'Replace the page with a new interactive app.',mode:'live',base:ORIGINAL});return c;}
test('complete arbitrary HTML/CSS/JS replaces an expense-specific contract',()=>{
 assert.deepEqual(validateBundle(REPLACEMENT),REPLACEMENT);
 assert.deepEqual(validateBundle({html:'',css:'',js:'document.body.textContent="A JS-built SPA"'}),{html:'',css:'',js:'document.body.textContent="A JS-built SPA"'});
 assert.equal(canonicalBundle({js:REPLACEMENT.js,html:REPLACEMENT.html,css:REPLACEMENT.css}),canonicalBundle(REPLACEMENT));
 for(const bad of [{html:'',css:'',js:''},{...ORIGINAL,html:12},{...ORIGINAL,html:'x'.repeat(32001)},{...ORIGINAL,css:'x'.repeat(32000),js:'x'.repeat(32000)},{...ORIGINAL,filename:'app.html'}])assert.throws(()=>validateBundle(bad));
});
test('source edits are exact, unique, atomic and may touch any complete file',()=>{
 const original=structuredClone(ORIGINAL);assert.deepEqual(applyEdits(original,{files:REPLACEMENT}),REPLACEMENT);assert.deepEqual(original,ORIGINAL);
 assert.equal(applyEdits(original,{edits:[{file:'html',find:'Before',replace:'During'},{file:'html',find:'During',replace:'After'}]}).html,REPLACEMENT.html.split('<button')[0]+'</main>');
 for(const bad of [{edits:[]},{edits:[{file:'html',find:'missing',replace:'x'}]},{files:{html:ORIGINAL.html}},{files:{rust:'arbitrary'}},{edits:[{file:'html',find:'Before',replace:'After'},{file:'js',find:'missing',replace:'x'}]}])assert.throws(()=>applyEdits(original,bad));
 assert.deepEqual(original,ORIGINAL);assert.throws(()=>applyEdits({...ORIGINAL,html:'a a'},{edits:[{file:'html',find:'a',replace:'b'}]}));
});
test('generic plan/implementation/checks require fresh explicit acceptance and exact hashes',async()=>{
 const c=await prepared();assert.equal(c.task.status,'Ready');assert.equal(c.task.candidate,null);await assert.rejects(()=>c.accept({base:ORIGINAL}));await c.run();assert.equal(c.task.status,'Review');assert.ok(canAccept(c.task));assert.equal(c.task.hash,await digest(canonicalBundle(REPLACEMENT)));assert.deepEqual(await c.accept({base:ORIGINAL}),REPLACEMENT);assert.equal(c.task.status,'Done');
});
test('exact source no-op cannot be promoted even if model QA says pass',async()=>{
 const c=await prepared({model:model({code:{files:ORIGINAL}})});await assert.rejects(()=>c.run(),/repair cycles/);assert.equal(c.task.status,'Blocked');assert.equal(canAccept(c.task),false);
});
test('missing/forged browser coverage and model-only verdicts are never passing evidence',async()=>{
 for(const broken of [{rows:[],errors:[]},{rows:[{name:'forged',pass:true,expected:[]}],errors:[]},{rows:[{name:PLAN.checks[0].name,pass:'true',expected:PLAN.checks[0].assertions}],errors:[]},{rows:[{name:PLAN.checks[0].name,pass:true,expected:PLAN.checks[0].assertions}]}]){
  let planned=false;const c=await prepared({execute:async(bundle,checks)=>planned?broken:(planned=true,result(bundle,checks))});await assert.rejects(()=>c.run(),/every requested check/);assert.equal(canAccept(c.task),false);
 }
 const c=await prepared({model:model({qa:{summary:'All pass without executed checks.'}})});await assert.rejects(()=>c.run(),/QA must return/);assert.equal(canAccept(c.task),false);
});
test('source/check edits and a changed accepted baseline invalidate previous successful evidence',async()=>{
 for(const change of [t=>t.candidate.js+='\n// changed',t=>t.plan.checks[0].assertions[0].expected='different',t=>t.checks[0].assertions[0].expected='different',t=>t.evidence.checksHash='forged']){
  const c=await prepared();await c.run();change(c.task);await assert.rejects(()=>c.accept({base:ORIGINAL}));assert.notEqual(c.task.status,'Done');
 }
 const c=await prepared();await c.run();await assert.rejects(()=>c.accept({base:{...ORIGINAL,css:'changed'}}),/accepted app changed/);
});
test('acceptance re-executes real executor boundary instead of trusting saved passing flags',async()=>{
 let fail=false;const c=await prepared({execute:async(bundle,checks)=>{const r=result(bundle,checks);if(fail)r.errors=['New runtime failure'];return r;}});await c.run();assert.ok(canAccept(c.task));fail=true;await assert.rejects(()=>c.accept({base:ORIGINAL}),/no longer passes/);assert.notEqual(c.task.status,'Done');
});
test('cancelled planner and acceptance results cannot revive or promote a task',async()=>{
 let release;const c=new Controller({model:{cancel(){},generate(){return new Promise(resolve=>release=resolve)}},execute:async(...args)=>result(...args)});const pending=c.plan({request:'Any feature',mode:'live',base:ORIGINAL});c.cancel();release(JSON.stringify(PLAN));await assert.rejects(()=>pending,/cancelled/);assert.equal(c.task.phase,'Cancelled');assert.equal(c.task.plan,null);
 let hold=false,finish,waiting;const boundary=new Promise(resolve=>waiting=resolve);const d=await prepared({execute:async(...args)=>{if(hold){hold=false;waiting();await new Promise(resolve=>finish=resolve);}return result(...args)}});await d.run();hold=true;const accepting=d.accept({base:ORIGINAL});await boundary;d.cancel();finish();await assert.rejects(()=>accepting,/cancelled/);assert.equal(d.task.phase,'Cancelled');assert.equal(canAccept(d.task),false);
});
test('freshly restored and explicitly aborted previous runtime sessions can be independently verified',async()=>{
 const source=await prepared();await source.run();
 const executor=async(bundle,checks,{signal}={})=>{if(signal?.aborted)throw Error('Unexpected aborted verification signal');return result(bundle,checks)};
 const restored=new Controller({model:model(),execute:executor});restored.task=structuredClone(source.task);await restored.verify();
 restored.abort=new AbortController();restored.abort.abort();await restored.verify();
});

test('cancelling a baseline await cannot start a new Planner formatting retry',async()=>{
 let rejectBaseline,started;const atBaseline=new Promise(resolve=>started=resolve);let calls=0;
 const c=new Controller({model:{cancel(){},async generate(){calls++;return JSON.stringify(PLAN)}},execute(){started();return new Promise((_,reject)=>rejectBaseline=reject)}});
 const pending=c.plan({request:'Any new app',mode:'live',base:ORIGINAL});await atBaseline;c.cancel();rejectBaseline(Error('Preview closed'));await assert.rejects(()=>pending,/cancelled/);assert.equal(calls,1);assert.equal(c.task.phase,'Cancelled');assert.equal(canAccept(c.task),false);
});

import {modelChecks,qaCases,CHECK_SEMANTICS,successfulLiveRun} from '../demo/page-contract.mjs';
test('flat model checks discard claimed outcomes while preserving mandatory typed assertions',()=>{
 const flat={name:'Any new control',selector:'#brand-new',property:'count',operator:'equals',expected:1,pass:true,actual:1,code:'not executed',commentary:'model opinion'};
 assert.deepEqual(modelChecks([flat]),[{name:flat.name,steps:[],assertions:[{selector:flat.selector,property:flat.property,operator:flat.operator,expected:flat.expected}]}]);
 for(const omitted of ['selector','property','operator','expected']){const bad={...flat};delete bad[omitted];assert.throws(()=>modelChecks([bad]),omitted);}
 assert.throws(()=>qaCases({summary:'Model says pass',cases:[{name:'Fake',pass:true,actual:1}]},2));
});
test('Recorded never plans, runs or accepts through the direct controller API',async()=>{
 const c=await prepared();await c.run();c.task.mode='recorded';c.task.status='Ready';await assert.rejects(()=>c.run(),/Live or Guided/);await assert.rejects(()=>c.plan({request:'New task',mode:'recorded',base:ORIGINAL}),/read-only/);c.task.status='Review';await assert.rejects(()=>c.accept({base:ORIGINAL}));
});
test('authorization metadata changed during acceptance is rejected after awaited checks',async()=>{
 let hold=false,release,started;const boundary=new Promise(resolve=>started=resolve);const c=await prepared({execute:async(...args)=>{if(hold){hold=false;started();await new Promise(resolve=>release=resolve);}return result(...args)}});await c.run();hold=true;const pending=c.accept({base:ORIGINAL});await boundary;c.task.planHash='forged during browser verification';release();await assert.rejects(()=>pending,/no longer passes/);assert.notEqual(c.task.status,'Done');
});

import {LocalModel,MODEL_IDENTITY} from '../demo/model.mjs';
test('imported model files receive generic identity while the default identifies its exact pinned artifact',async()=>{
 const previous=globalThis.Worker;
 globalThis.Worker=class{postMessage(message){queueMicrotask(()=>this.onmessage({data:{type:message.type==='load'?'loaded':'generated',text:'Synthetic adapter'}}))}terminate(){}};
 try{const loaded=new LocalModel(()=>{},()=>{});await loaded.start({name:'synthetic-compatible-model.litertlm'});assert.deepEqual(loaded.identity,{name:'Local Gemma model',artifact:null});assert.equal(loaded.ready,true);loaded.cancel();await loaded.start();assert.deepEqual(loaded.identity,MODEL_IDENTITY);assert.match(loaded.identity.artifact,/gemma-4-E4B.*e6a3892a8c22cf87ff99583996149cf3cb32452b/);loaded.cancel();}finally{if(previous===undefined)delete globalThis.Worker;else globalThis.Worker=previous;}
});
test('Live task provenance snapshots the loaded identity without relabeling it later',async()=>{
 const adapter=model();adapter.identity={name:'Synthetic QA model — NON_ACCEPTANCE',artifact:null};const c=await prepared({model:adapter});assert.deepEqual(c.task.model,adapter.identity);adapter.identity.name='different later session';assert.notEqual(c.task.model.name,adapter.identity.name);
});


test('old or missing check semantics cannot run, verify, accept or become Recorded evidence',async()=>{
 for(const version of [undefined,'first-node-v0','any-node-v1','unrecognized-future']){
  const pending=await prepared();let calls=0;pending.model.generate=async()=>{calls++;throw Error('Old plan must not call a model')};pending.execute=async()=>{calls++;throw Error('Old plan must not execute')};if(version===undefined)delete pending.task.plan.checkSemantics;else pending.task.plan.checkSemantics=version;pending.task.planHash=await digest(JSON.stringify(pending.task.plan));await assert.rejects(()=>pending.run(),/check rules changed/);assert.equal(calls,0);
  const reviewed=await prepared();await reviewed.run();assert.equal(reviewed.task.plan.checkSemantics,CHECK_SEMANTICS);reviewed.task.plan.checkSemantics=version;reviewed.task.planHash=await digest(JSON.stringify(reviewed.task.plan));assert.equal(canAccept(reviewed.task),false);assert.equal(successfulLiveRun(reviewed.task),false);reviewed.execute=async()=>{calls++;throw Error('Old plan must not execute')};await assert.rejects(()=>reviewed.verify(),/check rules changed/);await assert.rejects(()=>reviewed.accept({base:ORIGINAL}));assert.equal(calls,0);
 }
});
test('flat model first flag remains typed and cannot conflict with all',()=>{
 const flat={name:'First heading',selector:'h2',property:'text',operator:'equals',expected:'Before',first:true};assert.equal(modelChecks([flat])[0].assertions[0].first,true);for(const bad of [{...flat,first:'true'},{...flat,all:true}])assert.throws(()=>modelChecks([bad]));
});


test('missing-action contradictory state plans never reach approval or browser execution',async()=>{
 const checks=[false,true].map(expected=>({name:'Independent state '+expected,selector:'#tip-text',property:'visible',operator:'equals',expected}));
 for(const same of [checks,[{name:'Same scenario',steps:[],assertions:checks.map(({name,...assertion})=>assertion)}]])assert.throws(()=>modelChecks(same),/contradict/);
 let modelCalls=0,browserCalls=0;const c=new Controller({model:{cancel(){},async generate(){modelCalls++;return JSON.stringify({...PLAN,checks})}},execute:async()=>{browserCalls++;throw Error('Contradictory plan cannot run')}});
 await assert.rejects(()=>c.plan({request:'Add a Show tip toggle.',mode:'live',base:ORIGINAL}),/contradict/);assert.equal(modelCalls,2);assert.equal(browserCalls,0);assert.equal(c.task.status,'Blocked');assert.equal(canAccept(c.task),false);
 const valid=modelChecks([checks[0],{...checks[1],click:'#show-tip'}]);assert.equal(valid[1].steps[0].action,'click');assert.equal(valid[1].steps[0].selector,'#show-tip');
});
test('heterogeneous ANY values remain generic while explicit single observations reject conflicts',()=>{
 const value=(expected,extra={})=>({name:'Value '+expected,selector:'li',property:'text',operator:'equals',expected,...extra});
 assert.equal(modelChecks([value('Alpha'),value('Beta')]).length,2);
 for(const extra of [{first:true},{all:true},{selector:'#single'}])assert.throws(()=>modelChecks([value('Alpha',extra),value('Beta',extra)]),/contradict/);
 assert.equal(modelChecks([value('Alpha',{attribute:'data-a',property:'attribute',first:true}),value('Beta',{attribute:'data-b',property:'attribute',first:true})]).length,2);
});
