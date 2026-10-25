#!/usr/bin/env node
/** Generate the Pages preview from the official UI, without a backend transport. */
import {readFile, writeFile, mkdir} from 'node:fs/promises';
import {createHash} from 'node:crypto';
import {fileURLToPath} from 'node:url';
import path from 'node:path';

const root=fileURLToPath(new URL('../',import.meta.url));
const check=process.argv.includes('--check');
if(process.argv.slice(2).some(arg=>arg!=='--check'))throw new Error('Usage: node scripts/generate-workboard-preview.mjs [--check]');
const sourceNames=['index.html','styles.css','workboard.js','workboard-model.js'];
const sources=Object.fromEntries(await Promise.all(sourceNames.map(async name=>[name,await readFile(path.join(root,'public',name),'utf8')])));
function once(source, before, after){
  if(source.split(before).length!==2)throw new Error(`Official UI adaptation anchor changed: ${before.slice(0,80)}`);
  return source.replace(before,after);
}
function boundary(source,start,end,replacement){
  const from=source.indexOf(start),to=source.indexOf(end,from+start.length);
  if(from<0||to<0||source.indexOf(start,from+start.length)!==-1)throw new Error(`Official UI function boundary changed: ${start}`);
  return source.slice(0,from)+replacement+'\n'+source.slice(to);
}
let html=sources['index.html'];
html=once(html,'<title>Software Agent Orchestrator</title>','<title>Developer workboard · Read-only preview</title>');
html=once(html,'href="/styles.css"','href="./styles.css"');
html=once(html,'src="/workboard.js"','src="./workboard.js"');
html=once(html,'</head>',`  <meta http-equiv="Content-Security-Policy" content="default-src 'self'; connect-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; object-src 'none'; base-uri 'self'; form-action 'none'">\n  <link rel="icon" href="data:,">\n  <link rel="stylesheet" href="./preview.css">\n</head>`);
html=once(html,'  <header class="workspace-header">',`  <aside class="preview-banner" aria-label="Preview information"><strong>Developer workboard · Read-only preview with sample tasks</strong><span>The official workboard UI with a frozen, fictional workspace. No agents are running and nothing can be changed.</span><a href="../">← Back to Agents at Work</a></aside>\n  <header class="workspace-header">`);
html=once(html,'Private workspace</span>','Public sample workspace</span>');
html=once(html,'YOUR OPERATING SPACE','OFFICIAL DEVELOPER WORKBOARD');
html=once(html,'Tasks stay on this computer','Fictional tasks · no runtime connection');
html=once(html,'Main generates proposals from your goal and context. Leave repository blank for Research-only proposals. Repository source is not inspected. Review and edit the proposals before adding selected tasks to Backlog.','In the full application, Main turns a goal into proposals for your review. This preview shows a sample accepted plan; planning and task creation are disabled.');
html=once(html,'Main plans, Coder implements, QA reviews. Customize your team here.','Main plans, Coder implements, QA independently reviews. This fictional roster and activity are read-only; no agents are running.');
// Keep the official form layout; no editable goal implies a real planning service.
const formStart=html.indexOf('<form id="main-goal-form"'),formEnd=html.indexOf('</form>',formStart);
if(formStart<0||formEnd<0)throw new Error('Official Main form changed');
const form=html.slice(formStart,formEnd);
if((form.match(/<(?:input|textarea)\b/g)||[]).length!==4)throw new Error('Official Main fields changed');
html=html.slice(0,formStart)+form.replace(/<(input|textarea)\b/g,'<$1 disabled')+html.slice(formEnd);

let js=sources['workboard.js'];
js='// Generated from public/workboard.js. Run scripts/generate-workboard-preview.mjs to update.\nimport {previewRequest, snapshotAt} from "./preview-data.mjs";\n'+js;
js=once(js,'const api = "/api/workboard/v1";','const api = "./unavailable-preview-api"; // No network transport is present.');
js=js.replaceAll('"workboard-stage"','"workboard-preview-stage"');
js=boundary(js,'async function request(path,','function setConnection(connected)',`async function request(path, options = {}) {\n  return previewRequest(path, options);\n}`);
js=boundary(js,'function connectEvents() {','async function refresh()',`function connectEvents() {\n  // Frozen sample: intentionally no EventSource, subscription or background worker.\n}`);
js=once(js,'connected ? "Connected" : "Disconnected"','connected ? "Sample data" : "Preview unavailable"');
js=once(js,'const notice = executionNotice(state.capabilities, state.connected);','const notice = "Read-only sample. Task changes, agent runs, planning and delivery are disabled.";');
js=once(js,'state.lastUpdate = new Date().toISOString();','state.lastUpdate = snapshotAt;');
js=once(js,'`Updated ${displayTime(state.lastUpdate)}${state.connected ? "" : " · stale"}`','`Sample snapshot · ${displayHistoricalTime(state.lastUpdate)}`');
js=once(js,'"Archive completed tasks from their detail view."','"Browse sample tasks and their details. Changes are disabled."');
js=once(js,'!state.capabilities.planning?"AI planning is not configured on this host."','!state.capabilities.planning?"Read-only sample plan. AI generation is disabled in this preview."');
js=once(js,'`${state.connected?"Connected":"Connection interrupted"} · ${!state.execution?.approved?"Execution unavailable":state.execution.paused?"Launches paused":"Launches enabled"}`','"Sample agent roster · no agents are running"');
// These three official command buttons only navigate; preserve that behavior in read-only mode.
js=once(js,'commandButton("View accepted task",()=>void openDetail(id))','commandButton("View accepted task",()=>void openDetail(id),{capability:"read"})');
js=once(js,'commandButton("Open task",()=>void openDetail(task.id))','commandButton("Open task",()=>void openDetail(task.id),{capability:"read"})');
js=once(js,'commandButton(`${dep?.title||id} · ${readable(dep?.status||"missing")}`,()=>void openDetail(id))','commandButton(`${dep?.title||id} · ${readable(dep?.status||"missing")}`,()=>void openDetail(id),{capability:"read"})');
// Source polling is useful for a real host, but unnecessary for a frozen sample.
js=once(js,'setInterval(() => { if (!document.hidden && state.authenticated) void refresh(); }, 60000);','// No timer polling: the sample snapshot is immutable.');
if(/\b(?:fetch\s*\(|new\s+EventSource\s*\()/u.test(js))throw new Error('Generated preview still contains a network transport');
const manifest={generator:'scripts/generate-workboard-preview.mjs',source:'public/',sha256:Object.fromEntries(sourceNames.map(name=>[name,createHash('sha256').update(sources[name]).digest('hex')]))};
const outputs={'index.html':'<!-- Generated from public/index.html; edit the generator, not this file. -->\n'+html,'workboard.js':js,'styles.css':sources['styles.css'],'workboard-model.js':sources['workboard-model.js'],'source-manifest.json':JSON.stringify(manifest,null,2)+'\n'};
const out=path.join(root,'demo/workboard');
await readFile(path.join(out,'preview-data.mjs'));await readFile(path.join(out,'preview.css'));
if(!check)await mkdir(out,{recursive:true});
let stale=false;
for(const [name,content] of Object.entries(outputs)){
  const target=path.join(out,name);
  if(check){let actual;try{actual=await readFile(target,'utf8');}catch{}if(actual!==content){console.error(`Stale generated preview: demo/workboard/${name}`);stale=true;}}
  else await writeFile(target,content);
}
if(stale)process.exitCode=1;
else console.log(check?'Official workboard preview is up to date.':'Generated official workboard preview in demo/workboard/.');
