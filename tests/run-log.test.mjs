import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {RunLog,seconds} from '../demo/run-log.mjs';
import {LocalModel} from '../demo/model.mjs';

const task = (extra = {}) => ({id:'synthetic-task',status:'Active',phase:'Coder',attempt:1,mode:'live',events:[],...extra});
const setup = () => {let time = 100000; const log = new RunLog({now:()=>time}); return {log,advance:ms=>time+=ms};};
test('elapsed display is measured and clamped',()=>{assert.equal(seconds(1540),'1.5s');assert.equal(seconds(-10),'0.0s');});
test('wait status is honest, updates with elapsed time, and has no fake progress',()=>{const {log,advance}=setup();log.observe(task(),true);log.model({type:'start',role:'Coder',operation:'generate',inputChars:1000});advance(2300);assert.match(log.status().detail,/Elapsed 2.3s/);assert.match(log.status().detail,/Last runtime event 2.3s/);assert.match(log.status().detail,/does not indicate percentage/);assert.equal(log.rows.length,1);});
test('stage changes append once; identical status does not grow the log',()=>{const {log}=setup();log.model({type:'start',role:'Coder',operation:'generate',inputChars:50});for(let i=0;i<50;i++)log.model({type:'status',text:'Generating app patch'});assert.equal(log.rows.length,2);});
test('completed model output is explicitly unvalidated and no longer busy',()=>{const {log,advance}=setup();log.model({type:'start',role:'Coder',operation:'generate',inputChars:4});advance(2100);log.model({type:'complete',operation:'generate',output:'{"edits":[]}'});assert.match(log.rows.at(-1).message,/not yet validated/);assert.match(log.rows.at(-1).message,/2.1s/);assert.equal(log.rows.at(-1).detail,'{"edits":[]}');assert.equal(log.active.size,0);});
test('observer captures historical task milestones once with original timestamps',()=>{const {log}=setup();const t=task({events:[{role:'Tester',text:'A check failed',at:'2026-01-01T00:00:00Z'}]});log.observe(t,true);log.observe(t,true);assert.equal(log.rows.length,1);assert.equal(log.rows[0].at,Date.parse(t.events[0].at));});
test('new task removes old output and stale operation completion',()=>{const {log}=setup();log.observe(task(),true);const step=log.begin('Coder','Old request');log.observe(task({id:'another-task'}),true);log.finish(step,'Late completion','old output');assert.equal(log.rows.length,0);assert.equal(log.active.size,0);});
test('reset removes prior logs and running timers',()=>{const {log}=setup();log.observe(task(),true);log.begin('Browser','Checks');log.observe(null,false);assert.equal(log.rows.length,0);assert.equal(log.active.size,0);assert.equal(log.status().title,'No active task');});
test('cancelled tasks cannot keep a check timer running',()=>{const {log}=setup();const t=task();log.observe(t,true);const step=log.begin('Browser','Checks');log.observe({...t,status:'Blocked',phase:'Cancelled'},false);log.finish(step,'late results');assert.equal(log.active.size,0);assert.match(log.status().title,/Cancelled/);assert.ok(!log.rows.some(r=>r.message==='late results'));});
test('model error and stop terminate the current operation',()=>{for(const type of ['error','stopped']){const {log}=setup();log.model({type:'start',operation:'generate',role:'Coder',inputChars:1});log.model({type,message:'Synthetic reason'});assert.equal(log.active.size,0);assert.equal(log.rows.at(-1).detail,'Synthetic reason');}});
test('recorded mode does not claim a live model run',()=>{const {log}=setup();log.observe(task({mode:'recorded',status:'Done',phase:'Accepted locally'}),false);assert.equal(log.status().busy,false);assert.match(log.status().detail,/historical events, not live execution/);});
test('checks preserve evidence reference and report failures accurately',async()=>{const {log}=setup();const result={rows:[{name:'one',pass:true},{name:'two',pass:false}],errors:['Synthetic runtime error']};assert.equal(await log.checks(async()=>result,{},[{},{}],{}),result);assert.match(log.rows.at(-1).message,/1\/2 checks passed; 1 runtime errors/);assert.equal(log.rows.at(-1).detail,'PASS · one\nFAIL · two');});
test('check exceptions propagate unchanged',async()=>{const {log}=setup();const failure=Error('Synthetic failure');await assert.rejects(log.checks(async()=>{throw failure;},{},[],{}),e=>e===failure);assert.equal(log.active.size,0);assert.match(log.rows.at(-1).message,/failed/);});
test('aborted checks are labeled stopped and not successful',async()=>{const {log}=setup(),abort=new AbortController();abort.abort();await assert.rejects(log.checks(async()=>{throw Error('Abort');},{},[],{signal:abort.signal}));assert.match(log.rows.at(-1).message,/stopped/);});
test('bounded log and details do not grow indefinitely',()=>{const {log}=setup();for(let i=0;i<300;i++)log.add('Coder',String(i),'x'.repeat(7000));assert.equal(log.rows.length,200);assert.equal(log.dropped,100);assert.equal(log.rows[0].message,'100');assert.match(log.rows[0].detail,/truncated in log/);assert.ok(log.rows[0].detail.length<6100);});
test('export includes original times, role, mode, output and omitted count',()=>{const {log}=setup();log.observe(task(),true);log.add('Coder','Response received','{"edits":[]}');const out=log.exportText();assert.match(out,/live/);assert.match(out,/synthetic-task/);assert.match(out,/\[Coder\]/);assert.match(out,/edits/);});
test('rendering errors cannot stop execution',async()=>{const log=new RunLog({onChange:()=>{throw Error('Synthetic UI failure');}});const result={rows:[],errors:[]};assert.equal(await log.checks(async()=>result,{},[],{}),result);});

// Exercise the production model facade; the engine itself is a fixture.
async function withWorker(run){
 const original=globalThis.Worker;class Worker{static instances=[];constructor(url,options){this.url=url;this.options=options;this.messages=[];Worker.instances.push(this);}postMessage(data){this.messages.push(data);}terminate(){this.terminated=true;}emit(data){this.onmessage({data});}}
 globalThis.Worker=Worker;
 try{await run(Worker);}finally{if(original===undefined)delete globalThis.Worker;else globalThis.Worker=original;}
}
test('model facade emits actual lifecycle events without storing prompts',()=>withWorker(async Worker=>{
 const events=[],model=new LocalModel(()=>{},()=>{},event=>events.push(event));
 const loading=model.start();const worker=Worker.instances[0];assert.equal(worker.options.type,'classic');worker.emit({type:'loaded'});await loading;
 const pending=model.generate('Coder','Synthetic input');worker.emit({type:'status',phase:'working',text:'Preparing context'});worker.emit({type:'generated',text:'{"edits":[]}'});assert.equal(await pending,'{"edits":[]}');
 const generation=events.find(e=>e.type==='start'&&e.operation==='generate');assert.equal(generation.role,'Coder');assert.equal(generation.inputChars,15);assert.equal(generation.prompt,undefined);assert.equal(events.at(-1).output,'{"edits":[]}');assert.equal(model.ready,true);
}));
test('diagnostic failures do not change model output or readiness',()=>withWorker(async Worker=>{
 const model=new LocalModel(()=>{},()=>{},()=>{throw Error('UI gone');});const ready=model.start();Worker.instances[0].emit({type:'loaded'});await ready;const result=model.generate('Planner','x');Worker.instances[0].emit({type:'generated',text:'answer'});assert.equal(await result,'answer');assert.equal(model.ready,true);
}));
test('cancelled worker cannot append late logs or complete a replacement run',()=>withWorker(async Worker=>{
 const events=[],model=new LocalModel(()=>{},()=>{},e=>events.push(e));const first=model.start();const rejected=assert.rejects(first,/stopped/);const old=Worker.instances[0];model.cancel();await rejected;const count=events.length;old.emit({type:'status',phase:'initializing',text:'late'});old.emit({type:'loaded'});assert.equal(events.length,count);assert.equal(model.ready,false);
 const next=model.start();Worker.instances[1].emit({type:'loaded'});await next;assert.equal(model.ready,true);
}));
test('model error is observable without emitting a successful completion',()=>withWorker(async Worker=>{
 const events=[],model=new LocalModel(()=>{},()=>{},e=>events.push(e));const p=model.start();const fail=assert.rejects(p,/Synthetic error/);Worker.instances[0].emit({error:'Synthetic error'});await fail;assert.equal(events.at(-1).type,'error');assert.equal(events.some(e=>e.type==='complete'),false);
}));
test('worker diagnostics preserve complete streaming proposals, phase milestones and constrained decoding',async()=>{const source=await readFile(new URL('../demo/model-worker.mjs',import.meta.url),'utf8');assert.match(source,/await readResponse\(conversation,data.prompt,receive\)/);assert.match(source,/conversation\.sendMessageStreaming\(prompt\)/);assert.match(source,/enableConstrainedDecoding:true/);assert.match(source,/validating output format/);assert.match(source,/preparing conversation and context/);assert.match(source,/type:'diagnostic',role,stage,text/);assert.match(source,/self\.postMessage\(\{type:'generated',\.\.\.await generateResponse\(data\)\}\)/);});
test('app wires logs to the current full-app controller and real checks',async()=>{const source=await readFile(new URL('../demo/app.mjs',import.meta.url),'utf8');assert.match(source,/runLog\.observe\(controller.task,controller.busy\)/);assert.match(source,/runLog\.checks\(runPageChecks,bundle,checks,options\)/);assert.match(source,/event=>runLog.model\(event\)/);});
test('log UI never interprets generated content as HTML or code',async()=>{const source=await readFile(new URL('../demo/run-log.mjs',import.meta.url),'utf8');assert.doesNotMatch(source,/innerHTML|eval\(|new Function/);assert.match(source,/textContent = content/);assert.match(source,/clearInterval\(interval\)/);assert.match(source,/if \(!dialog.open\) return/);});

test('mode changes refresh the modal even when task status is unchanged',()=>{let changes=0;const log=new RunLog({onChange:()=>changes++});const t=task({status:'Done',phase:'Accepted locally'});log.observe(t,false);const before=changes;log.observe({...t,mode:'recorded'},false);assert.ok(changes>before);assert.match(log.status().detail,/not live execution/);});
test('active headline does not duplicate the role prefix',()=>{const {log}=setup();log.model({type:'start',role:'Coder',operation:'generate',inputChars:1});log.model({type:'status',text:'Coder: generating an app patch'});assert.equal(log.status().title,'Coder · generating an app patch');});
