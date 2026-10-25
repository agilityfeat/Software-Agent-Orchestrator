import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile,writeFile,cp,mkdtemp,rm,mkdir} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {spawnSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {previewRequest,snapshotAt} from '../demo/workboard/preview-data.mjs';
import {columns,taskStatus,attentionReason} from '../demo/workboard/workboard-model.js';
const root=fileURLToPath(new URL('../',import.meta.url));
const read=name=>readFile(path.join(root,name),'utf8');
const generate=location=>spawnSync(process.execPath,[path.join(location,'scripts/generate-workboard-preview.mjs'),'--check'],{encoding:'utf8'});

test('generated preview matches current official source, with exact CSS and model parity',async()=>{
  const result=generate(root);assert.equal(result.status,0,result.stderr);
  for(const name of ['styles.css','workboard-model.js'])assert.equal(await read(`demo/workboard/${name}`),await read(`public/${name}`));
  const manifest=JSON.parse(await read('demo/workboard/source-manifest.json'));
  for(const [name,hash]of Object.entries(manifest.sha256))assert.equal(hash,createHash('sha256').update(await read(`public/${name}`)).digest('hex'));
});

test('drift check rejects changed generated files, changed source, and moved transport boundaries',async()=>{
  const temp=await mkdtemp(path.join(tmpdir(),'workboard-generation-'));
  try{
    await mkdir(path.join(temp,'scripts'));await cp(path.join(root,'scripts/generate-workboard-preview.mjs'),path.join(temp,'scripts/generate-workboard-preview.mjs'));
    await cp(path.join(root,'public'),path.join(temp,'public'),{recursive:true});await cp(path.join(root,'demo/workboard'),path.join(temp,'demo/workboard'),{recursive:true});
    assert.equal(generate(temp).status,0);
    const css=await read('public/styles.css');await writeFile(path.join(temp,'demo/workboard/styles.css'),css+'\n/* drift */');
    assert.equal(generate(temp).status,1);
    await writeFile(path.join(temp,'demo/workboard/styles.css'),css);await writeFile(path.join(temp,'public/styles.css'),css+'\n/* upstream */');
    assert.equal(generate(temp).status,1);
    await writeFile(path.join(temp,'public/styles.css'),css);
    const js=await read('public/workboard.js');await writeFile(path.join(temp,'public/workboard.js'),js.replace('function connectEvents() {','function changedEvents() {'));
    assert.notEqual(generate(temp).status,0);
  }finally{await rm(temp,{recursive:true,force:true});}
});

test('preview transport rejects all writes and unknown routes without a network fallback',async()=>{
  for(const method of ['POST','PUT','PATCH','DELETE','HEAD','OPTIONS','get']){
    await assert.rejects(previewRequest('/session',{method}),error=>error.status===403);
    await assert.rejects(previewRequest('/tasks/sample-001',{method,body:{status:'ready'}}),error=>error.status===403);
  }
  await assert.rejects(previewRequest('/snapshot',{body:{}}),error=>error.status===403);
  for(const target of ['/events','/plans','/runs/sample-run-1/report.md','/tasks/sample-999','/tasks/../session','https://example.invalid/api',new Request('https://example.invalid/',{method:'POST'})])await assert.rejects(previewRequest(target));
  const adapter=await read('demo/workboard/preview-data.mjs');const js=await read('demo/workboard/workboard.js');
  assert.doesNotMatch(adapter,/\b(?:fetch\s*\(|new\s+EventSource\s*\()/);
  assert.doesNotMatch(js,/\b(?:fetch\s*\(|new\s+EventSource\s*\()/);
  assert.doesNotMatch(js,/\bsetInterval\s*\(/);
});

test('sessions grant only read access and snapshots/details are isolated immutable copies',async()=>{
  const session=await previewRequest('/session');assert.equal(session.authenticated,true);assert.equal(session.csrf_token,undefined);
  for(const [key,value]of Object.entries(session.capabilities))assert.equal(value,key==='read');
  session.capabilities.write=true;
  assert.equal((await previewRequest('/session')).capabilities.write,false);
  const snapshot=await previewRequest('/snapshot'),first=snapshot.tasks[0];first.title='tampered';snapshot.capabilities.run=true;snapshot.tasks.pop();
  const fresh=await previewRequest('/snapshot');assert.equal(fresh.tasks.length,12);assert.equal(fresh.capabilities.run,false);assert.notEqual(fresh.tasks[0].title,'tampered');
  const detail=await previewRequest('/tasks/sample-005');assert.equal(detail.task.id,'sample-005');assert.equal(detail.runs.length,1);assert.equal(detail.events.length,1);
  detail.task.title='tampered';assert.notEqual((await previewRequest('/tasks/sample-005')).task.title,'tampered');
});

test('fictional fixtures exercise every stage, independent roles, priority, dependencies and attention',async()=>{
  const data=await previewRequest('/snapshot');assert.equal(new Date(snapshotAt).toISOString(),snapshotAt);
  assert.deepEqual([...new Set(data.tasks.map(task=>taskStatus(task,data.runs)))].sort(),[...columns].sort());
  assert.deepEqual(data.system.agents.map(agent=>agent.role),['main','coder','qa']);assert.equal(data.system.editable,false);
  assert.ok(data.tasks.some(task=>task.archived_at));assert.ok(data.tasks.some(task=>task.dependencies.length));assert.ok(data.tasks.some(task=>attentionReason(task,data.runs)));
  assert.ok(data.tasks.some(task=>task.priority===1));assert.equal(data.projects.length,2);assert.equal(data.plans[0].status,'accepted');
  for(const project of data.projects){const active=data.tasks.filter(task=>task.repository===project.repository&&taskStatus(task,data.runs)==='active');assert.equal(active.length,1);assert.equal(data.runs.find(run=>run.id===project.lane_run_id).task_id,active[0].id);}
  for(const task of data.tasks)assert.ok(task.context.some(text=>text.includes('Fictional')||text.includes('Sample')));
});

test('public preview provides clear labels, safe subpath assets, and a visible round-trip link',async()=>{
  const html=await read('demo/workboard/index.html');
  assert.match(html,/Developer workboard · Read-only preview with sample tasks/);
  assert.match(html,/connect-src 'none'/);assert.match(html,/form-action 'none'/);assert.match(html,/rel="icon" href="data:,"/);
  assert.match(html,/href="\.\.\/"/);assert.match(html,/href="\.\/styles.css"/);assert.match(html,/src="\.\/workboard.js"/);
  assert.doesNotMatch(html,/(?:src|href)="\/(?:styles|workboard)/);
  assert.match(await read('demo/index.html'),/<a class="quiet" href="\.\/workboard\/">Developer workboard<\/a>/);
  assert.match(await read('demo/index.html'),/rel="icon" href="data:,"/);
  const mainForm=html.match(/<form id="main-goal-form"[\s\S]*?<\/form>/)[0];
  assert.equal((mainForm.match(/<(?:input|textarea) disabled\b/g)||[]).length,4);
});
