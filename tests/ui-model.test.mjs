import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import { kanbanMovePayload, cardPriorityPayload, sortQueuedTasks, repositoryQueueStatus, codingAgentSelection, coalesceActivity, qaBugLabel, initialAutomationPolicy, readyTaskPayload, automationLabel, activeRun, taskStatus, attentionReason, taskPayload, executionNotice, executionReason } from "../public/workboard-model.js";

const task = {id:"task-1",status:"review",workflow:"engineering"};
const run = status => ({id:"run-1",task_id:task.id,status,created_at:"2026-10-22T12:00:00.000Z"});

test("waiting for manual merge retains execution ownership but displays Review",()=>{
  assert.equal(activeRun(task,[run("awaiting_merge")]).id,"run-1");
  assert.equal(taskStatus(task,[run("awaiting_merge")]),"review");
  assert.equal(attentionReason(task,[run("awaiting_merge")]),"Waiting for manual merge");
});
test("unknown outcome retains ownership until reconciliation",()=>{
  const open={...task,status:"active"};
  assert.equal(activeRun(open,[run("unknown")]).id,"run-1");
  assert.equal(attentionReason(open,[run("unknown")]),"Execution needs reconciliation");
  assert.equal(activeRun(open,[run("failed")]),null);
});
test("task forms preserve the manual merge veto and never enroll automatic runs",()=>{
  const form={workflow:"engineering",repository:"fixture/repo",merge_policy:"manual",automation_policy:"manual"};
  assert.equal(taskPayload(form).merge_policy,"manual");
  assert.equal(taskPayload(form).automation_policy,"manual");
  assert.equal(taskPayload({...form,merge_policy:"auto"}).merge_policy,"auto");
  assert.equal(taskPayload({workflow:"manual",merge_policy:"auto"}).merge_policy,"manual");
});
test("pause notice does not imply that an existing worker stopped",()=>{
  assert.match(executionNotice({paused:true}),/Current work may continue/);
  assert.match(executionNotice({execution_state:"reconciliation_required",paused:true}),/uncertain dispatch/);
});

test("stage failures explain the next action without internal diagnostic codes",()=>{
  assert.match(executionReason("qa_failed"),/QA found changes/);
  assert.match(executionReason("restart_requires_reconciliation"),/Check its recorded outcome/);
  assert.doesNotMatch(executionReason("unknown_internal_error"),/unknown_internal_error/);
});

test("automatic history retains the reviewed enrollment revision after internal status revisions",()=>{
  const enrolled={automation_policy:"ready_to_done",automation_revision:2,revision:2};
  assert.match(automationLabel(enrolled,null),/Enrolled for this revision/);
  assert.match(automationLabel(enrolled,{automation:{revision:1}}),/Enrolled for this revision/);
  assert.equal(automationLabel({...enrolled,revision:4},{automation:{revision:2},status:"succeeded"}),"Automatically picked up from revision 2");
  assert.equal(automationLabel({...enrolled,automation_policy:"manual",revision:5},{status:"failed"}),"Explicit Run");
  assert.equal(automationLabel({...enrolled,revision:3},null),"Explicit Run");
});

test("connected project tasks keep their queue binding and clear manual tracking authority",()=>{
 const base={project_id:"project-1",project:"Example",repository:"owner/repo",workflow:"engineering",automation_policy:"ready_to_done",merge_policy:"auto"};
 const queued=taskPayload(base);assert.equal(queued.project_id,"project-1");assert.equal(queued.qa_policy,"required");assert.equal(queued.merge_policy,"auto");assert.equal(automationLabel({...base,automation_revision:2,revision:2}),"Queued for automatic processing");
 const manual=taskPayload({...base,workflow:"manual"});assert.equal(manual.project_id,null);assert.equal(manual.repository,null);assert.equal(manual.automation_policy,"manual");assert.equal(manual.merge_policy,"manual");
});

test("project Backlog to Mark ready explicitly renews the queued revision without enrolling legacy or manual delivery",()=>{
 const queued={id:"task",project_id:"project",workflow:"engineering",revision:1,automation_policy:"ready_to_done",merge_policy:"auto",qa_policy:"required"};
 assert.deepEqual(readyTaskPayload(queued),{status:"ready",expected_revision:1,automation_policy:"ready_to_done"});
 assert.deepEqual(readyTaskPayload({...queued,project_id:null}),{status:"ready",expected_revision:1});
 assert.deepEqual(readyTaskPayload({...queued,merge_policy:"manual"}),{status:"ready",expected_revision:1});
 assert.deepEqual(readyTaskPayload({...queued,automation_policy:"manual"}),{status:"ready",expected_revision:1});
});

 test("ticket repository needs no connected project to queue Ready",()=>{
 const task={repository:"owner/repo",project:"Free label",workflow:"engineering",revision:2,automation_policy:"ready_to_done",merge_policy:"auto",qa_policy:"required"};
 assert.deepEqual(readyTaskPayload(task),{status:"ready",expected_revision:2,automation_policy:"ready_to_done"});
 const payload=taskPayload({...task,title:"Build outcome",objective:"Useful result",status:"ready"});assert.equal(payload.repository,"owner/repo");assert.equal(payload.project,"Free label");assert.equal(payload.project_id,undefined);assert.equal(payload.qa_policy,"required");assert.equal(automationLabel({...task,automation_revision:2}),"Queued for automatic processing");
 });

test("automatic Ready processing preserves manual merge independently",()=>{const ticket={workflow:"engineering",repository:"owner/repo",project:"label",merge_policy:"manual",automation_policy:"ready_to_done",qa_policy:"required",revision:1};const payload=taskPayload(ticket);assert.equal(payload.merge_policy,"manual");assert.equal(payload.automation_policy,"ready_to_done");assert.deepEqual(readyTaskPayload(ticket),{status:"ready",expected_revision:1,automation_policy:"ready_to_done"});});

test("planning-only new task never submits hidden automatic enrollment",()=>{
 for(const capabilities of [undefined,{}, {ready_to_done:false},{ready_to_done:"true"}]) {
  const payload=taskPayload({workflow:"engineering",repository:"owner/repo",title:"Plan work",objective:"Define outcome",status:"backlog",automation_policy:initialAutomationPolicy(capabilities)});
  assert.equal(payload.automation_policy,"manual");assert.equal(payload.workflow,"engineering");assert.equal(payload.status,"backlog");
 }
 assert.equal(initialAutomationPolicy({ready_to_done:true}),"ready_to_done");
 assert.equal(initialAutomationPolicy({ready_to_done:true},true),"manual");
});

test("unrelated bug labels remain text and optional criticality survives task edits",()=>{
 const summary='<img src=x onerror=alert(1)>';assert.equal(qaBugLabel({criticality:"high",path:"src/old.js",line:3,summary}),`HIGH · src/old.js:3 — ${summary}`);
 assert.equal(taskPayload({workflow:"engineering",criticality:"critical"}).criticality,"critical");assert.equal(taskPayload({}).criticality,null);
});

test("activity collapses only adjacent identical Run rows at the displayed minute",()=>{
 const base={run_id:"run",task_id:"task",type:"run.updated",stage:"coder",status:"running",reason:null,at:"2026-10-22T12:00:01.000Z"};
 const minute=at=>at.slice(0,16),later={...base,at:"2026-10-22T12:00:59.000Z",cursor:2};
 const input=[base,later];assert.deepEqual(coalesceActivity(input,minute),[base]);assert.equal(input.length,2);
 for(const changed of [{run_id:"other"},{task_id:"other"},{type:"run.failed"},{stage:"qa"},{reason:"qa_repair_queued"},{summary:"Run failed"},{task_revision:2},{summary:"Different message"},{at:"2026-10-22T12:01:00.000Z"},{cycle:1},{attempt_id:"new-attempt"}])assert.equal(coalesceActivity([base,{...later,...changed}],minute).length,2);
 const prepare={...base,type:"run.created",stage:"prepare",status:"admitted"};assert.equal(coalesceActivity([prepare,{...later,stage:"prepare",status:"running"}],minute).length,1);
 assert.equal(coalesceActivity([{...base,stage:null},{...later,stage:null,status:"failed"}],minute).length,2);
 const qa={...base,stage:"qa"};assert.deepEqual(coalesceActivity([base,qa,later],minute),[base,qa,later]);
 const edit={...base,type:"task.updated",task_revision:2};assert.equal(coalesceActivity([edit,{...edit,task_revision:3}],minute).length,2);
 assert.equal(coalesceActivity([{...base,at:null},{...base,at:null}],minute).length,2);
});

test("Main selection sends only reviewed content and stable proposal IDs",async()=>{
 const {planSelection}=await import("../public/workboard-model.js");
 const proposals=[{proposal_id:"first"},{proposal_id:"second"}],edits=new Map([["first",{selected:true,title:" Edited title ",objective:" Outcome ",acceptance_criteria:"First case\n\nSecond case",priority:"2",repository:"foreign/repo",status:"ready"}],["second",{selected:false,title:"Excluded"}]]);
 assert.deepEqual(planSelection(proposals,edits),[{proposal_id:"first",qa_policy:"required",proposal:{task_type:"feature",labels:[],depends_on:[],title:"Edited title",objective:"Outcome",acceptance_criteria:["First case","Second case"],priority:2}}]);assert.equal(proposals.length,2);
});


test("session revocation clears private task and Main drafts, dependencies and pending commands",()=>{
 const source=readFileSync(new URL("../public/workboard.js",import.meta.url),"utf8");
 const expire=source.slice(source.indexOf("function expireSession()"),source.indexOf("\nfunction applySession("));
 const main={goal:"Private goal",context:"Private context",project:"Private project",repository:"private/repo"},task={title:"Private title",objective:"Private objective",context:"Private constraints"};
 const element=()=>({hidden:false,open:false,textContent:"",children:[],replaceChildren(){this.children=[];},close(){this.open=false;}}),elements=new Map();
 const select=selector=>{if(!elements.has(selector))elements.set(selector,element());return elements.get(selector);};
 for(const [selector,values]of [["#main-goal-form",main],["#task-form",task]])select(selector).reset=()=>{for(const key of Object.keys(values))values[key]="";};
 for(const selector of ["#main-error","#editor-error"])select(selector).textContent="Private error";
 for(const selector of ["#task-dependencies","#detail-content"])select(selector).children=["Private ticket title"];
 select("#task-editor").open=true;select("#task-detail").open=true;
 const state={authGeneration:0,authenticated:true,editing:{title:"Private editing"},selected:"private-ticket",plans:[{goal:"Private plan"}]},planEdits=new Map([["proposal",{title:"Private draft"}]]),pendingCommands=new Map([["private body","request-id"]]);let closed=false;
 runInNewContext(`${expire};expireSession();`,{state,planEdits,pendingCommands,stream:{close(){closed=true;}},$:select,render(){},setConnection(){}});
 assert.ok(Object.values(main).every(value=>value===""));assert.ok(Object.values(task).every(value=>value===""));
 for(const selector of ["#main-error","#editor-error"])assert.equal(select(selector).textContent,"");
 for(const selector of ["#task-dependencies","#detail-content"])assert.equal(select(selector).children.length,0);
 assert.equal(select("#task-editor").open,false);assert.equal(select("#task-detail").open,false);assert.equal(closed,true);
 assert.equal(state.authenticated,false);assert.equal(state.plans.length,0);assert.equal(planEdits.size,0);assert.equal(pendingCommands.size,0);assert.equal(state.editing,null);assert.equal(state.selected,null);
});

test("failed session requests never retain the operator key in pending commands",async()=>{
 const source=readFileSync(new URL("../public/workboard.js",import.meta.url),"utf8");
 const definition=source.slice(source.indexOf("async function request("),source.indexOf("\nfunction setConnection("));
 const pendingCommands=new Map(),state={authGeneration:0,csrf:null};let sent;
 const context={state,pendingCommands,api:"/api/workboard/v1",crypto:{randomUUID:()=>"command-id"},setConnection(){},fetch:async(url,options)=>{sent=options;throw Error("synthetic disconnect");}};
 await assert.rejects(runInNewContext(`${definition};request('/session',{method:'POST',body:{key:'synthetic-operator-key'}});`,context),error=>error.network===true);
 assert.equal(pendingCommands.size,0);assert.equal(sent.headers["Idempotency-Key"],undefined);
 // Engineering mutation retries retain their idempotency identity until actual revocation.
 await assert.rejects(runInNewContext(`${definition};request('/tasks',{method:'POST',body:{title:'Private draft'}});`,context),error=>error.network===true);
 assert.equal(pendingCommands.size,1);assert.equal(sent.headers["Idempotency-Key"],"command-id");
});

test("ordinary disconnect and remembered restart preserve drafts but confirmed revocation clears them",async()=>{
 const source=readFileSync(new URL("../public/workboard.js",import.meta.url),"utf8");
 const definition=source.slice(source.indexOf("async function checkRememberedSession("),source.indexOf("\nfunction connectEvents("));
 const state={authenticated:true,authGeneration:0,editing:{title:"Unsaved private draft"}};let expired=0,applied=0;
 const context={state,request:async()=>{throw Error("restart disconnect");},expireSession(){expired++;state.editing=null;state.authenticated=false;},applySession(){applied++;},notify(){}};
 await runInNewContext(`${definition};checkRememberedSession();`,context);assert.equal(expired,0);assert.equal(state.editing.title,"Unsaved private draft");
 context.request=async()=>({authenticated:true});await runInNewContext(`${definition};checkRememberedSession();`,context);assert.equal(applied,1);assert.equal(expired,0);
 context.request=async()=>({authenticated:false});await runInNewContext(`${definition};checkRememberedSession();`,context);assert.equal(expired,1);assert.equal(state.editing,null);
});


test("ticket metadata filters and QA policy are explicit and independent of labels",async()=>{
 const {filterTasks,defaultQaPolicy,dependencyReason}=await import("../public/workboard-model.js");
 const research=taskPayload({workflow:"engineering",task_type:"research",labels:"QA, discovery",dependencies:["task-a"],automation_policy:"ready_to_done"});
 assert.equal(research.qa_policy,"skip");assert.equal(defaultQaPolicy("feature"),"required");
 assert.equal(taskPayload({...research,labels:"skip, research",qa_policy:"required"}).qa_policy,"required");
 assert.deepEqual(research.dependencies,["task-a"]);assert.deepEqual(research.labels,["QA","discovery"]);
 assert.deepEqual(readyTaskPayload({...research,repository:"owner/repo",revision:2}),{status:"ready",expected_revision:2,automation_policy:"ready_to_done"});
 const tickets=[{id:"legacy",title:"Legacy"},{id:"new",task_type:"research",labels:["discovery"],title:"Investigate"}];
 assert.equal(filterTasks(tickets,{task_type:"research",label:"discovery"})[0].id,"new");assert.equal(filterTasks(tickets,{task_type:"feature"})[0].id,"legacy");assert.equal(filterTasks(tickets,{search:"discovery"}).length,1);
 assert.equal(dependencyReason({dependency_blockers:[{id:"task-a",title:"<script>required</script>"}]}),"Waiting for: <script>required</script>");
});

test("completion persistence failure explains reconciliation without exposing internals",()=>{
 assert.match(executionReason("completion_persistence_failed"),/result could not be saved/);
 assert.match(executionReason("completion_persistence_failed"),/Check the run outcome/);
 assert.doesNotMatch(executionReason("completion_persistence_failed"),/completion_persistence_failed/);
});

test("task and HTTP size rejections give actionable copy without internal codes",()=>{
 const source=readFileSync(new URL("../public/workboard.js",import.meta.url),"utf8");const definition=source.slice(source.indexOf("function errorMessage("),source.indexOf("\nasync function request("));
 for(const code of ["task_too_large","body_too_large"]){const message=runInNewContext(`${definition};errorMessage({code});`,{code});assert.match(message,/Shorten the objective, context or acceptance criteria/);assert.ok(!message.includes(code));}
});

test("Factory editor retains Processing on ordinary save while legacy activation stays manual",()=>{
 const task={id:"ticket",workflow:"engineering",status:"backlog",automation_policy:"ready_to_done",title:"Existing ticket",objective:"Original objective",acceptance_criteria:[],context:[]};
 for(const capabilities of [undefined,{}, {projects:false,ready_to_done:true},{projects:"true",ready_to_done:true},{projects:true,ready_to_done:false}])assert.equal(initialAutomationPolicy(capabilities,true,task),"manual");
 assert.equal(initialAutomationPolicy({projects:true,ready_to_done:true},true,task),"ready_to_done");
 assert.equal(initialAutomationPolicy({projects:true,ready_to_done:true},true,{...task,automation_policy:"manual"}),"manual");
 const source=readFileSync(new URL("../public/workboard.js",import.meta.url),"utf8");const definition=source.slice(source.indexOf("function openEditor("),source.indexOf("\nfunction updateWorkflowHint("));
 for(const [capabilities,policy,expected]of [[{projects:true,ready_to_done:true},"ready_to_done","ready_to_done"],[{projects:true,ready_to_done:true},"manual","manual"],[{projects:false,ready_to_done:true},"ready_to_done","manual"]]){
  const element=()=>({value:"",open:false,disabled:false,replaceChildren(){},append(){},querySelector(){return{disabled:false};},remove(){},focus(){},showModal(){this.open=true;}});
  const fields=new Proxy({}, {get(object,key){return object[key]??=(element());}});const form={elements:fields,reset(){}};const elements=new Map([["#task-form",form]]);const $=selector=>{if(!elements.has(selector))elements.set(selector,element());return elements.get(selector);};
  const existing={...task,automation_policy:policy};
  runInNewContext(`${definition};openEditor(task);`,{task:existing,state:{capabilities,runs:[],tasks:[],projects:[]},editorQaExplicit:false,$,allowed:()=>true,activeRun:()=>null,array:v=>Array.isArray(v)?v:[],node:element,codingAgentSelection,initialAutomationPolicy,updateWorkflowHint(){}});
  assert.equal(fields.automation_policy.value,expected);
  fields.objective.value="Only objective changed";
  const payload=taskPayload(Object.fromEntries(Object.entries(fields).map(([key,field])=>[key,field.value])));
  assert.equal(payload.automation_policy,expected);assert.equal(payload.objective,"Only objective changed");
 }
});


test("coding agent selection preserves explicit assignment and never redirects an unavailable agent", () => {
  const config = {default_coder:"coder",agents:[{id:"coder",label:"Coder"},{id:"example",label:"Example"}]};
  assert.equal(codingAgentSelection(config).selected,"coder");
  assert.equal(codingAgentSelection(config,{coding_agent_id:"example"}).selected,"example");
  const missing = codingAgentSelection(config,{coding_agent_id:"retired"});
  assert.equal(missing.selected,""); assert.equal(missing.unavailable,true);
  assert.deepEqual(codingAgentSelection(null),{agents:[],selected:"",unavailable:false});
  const engineering = taskPayload({workflow:"engineering",coding_agent_id:"example",owner:"Someone else",qa_policy:"required",merge_policy:"manual",automation_policy:"ready_to_done"});
  assert.equal(engineering.coding_agent_id,"example"); assert.equal(engineering.owner,"Someone else");
  assert.equal(engineering.merge_policy,"manual"); assert.equal(engineering.qa_policy,"required"); assert.equal(engineering.automation_policy,"ready_to_done");
  assert.equal(taskPayload({workflow:"manual",coding_agent_id:"example"}).coding_agent_id,null);
  assert.equal(Object.hasOwn(taskPayload({workflow:"engineering"}),"coding_agent_id"),false);
});


test("repository queue distinguishes idle, waiting work, active work and launch pauses", () => {
  const project = {id:"project-1", repository:"owner/repo", runtime_status:"configured", paused:false, lane_run_id:null};
  assert.equal(repositoryQueueStatus(project), "Idle");
  assert.equal(repositoryQueueStatus(project, 2), "Queued");
  assert.equal(repositoryQueueStatus({...project, lane_run_id:"run-1"}, 0), "Working");
  assert.equal(repositoryQueueStatus({...project, paused:true}, 2), "Paused");
  assert.equal(repositoryQueueStatus(project, 2, true), "Paused");
  // Pausing launches does not mean that an existing run has stopped.
  assert.equal(repositoryQueueStatus({...project, lane_run_id:"run-1", paused:true}, 2, true), "Working");
  assert.equal(repositoryQueueStatus({...project, runtime_status:"preparing"}, 0, true), "Preparing checks");
  assert.equal(repositoryQueueStatus({...project, failure:"setup_failed", paused:true}, 2), "Needs attention");
});

test("rendered repository queue is idle after completed work and queued only for enrolled Ready revisions", () => {
  const source=readFileSync(new URL("../public/workboard.js",import.meta.url),"utf8");
  const definition=source.slice(source.indexOf("function renderProjects("),source.indexOf('\n$("#task-merge-policy")'));
  const project={id:"project-1",name:"Example",repository:"owner/repo",runtime_status:"configured",paused:false,lane_run_id:null};
  const queued={id:"next",repository:"OWNER/Repo",status:"ready",automation_policy:"ready_to_done",revision:2,automation_revision:2};
  const node=(tag,text)=>({tag,text,children:[],append(...items){this.children.push(...items);},replaceChildren(){this.children=[];},querySelectorAll(){return this.children.filter(child=>child.tag==="details"&&child.open);}});
  const list=node("div"),panel=node("details"),summary=node("span");
  const state={authenticated:true,projects:[project],execution:{paused:false},tasks:[{...queued,status:"done"},{...queued,id:"stale",automation_revision:1},{...queued,id:"manual",automation_policy:"manual"}]};
  const context={state,repositoryQueueStatus,node,$:selector=>selector==="#project-list"?list:selector==="#projects-summary"?summary:panel,commandButton:label=>node("button",label),errorMessage:()=>"Project checks failed"};
  const render=()=>runInNewContext(`${definition};renderProjects();`,context);
  const text=()=>JSON.stringify(list);
  render();assert.equal(panel.hidden,false);assert.equal(summary.textContent,"1 idle");assert.equal(list.children[0].open,false);assert.match(text(),/"text":"Idle"/);assert.match(text(),/0 queued/);assert.doesNotMatch(text(),/Queue running|"text":"Working"/);
  list.children[0].open=true;panel.open=true;
  state.tasks.push(queued);render();assert.equal(list.children[0].open,true);assert.equal(panel.open,true);assert.equal(summary.textContent,"1 queued");assert.match(text(),/"text":"Queued"/);assert.match(text(),/1 queued/);
  state.execution.paused=true;render();assert.match(text(),/"text":"Paused"/);assert.match(text(),/All launches are paused/);
  project.lane_run_id="run-1";render();assert.match(text(),/"text":"Working"/);
  state.projects.push({...project,id:"failed",repository:"owner/other",failure:"setup_failed"});render();assert.equal(summary.textContent,"1 needs attention · 1 working");
  state.authenticated=false;render();assert.equal(panel.hidden,true);assert.equal(list.children.length,0);assert.equal(summary.textContent,"");
});

test("repository strip and controls are native collapsed disclosures above the board",()=>{
 const html=readFileSync(new URL("../public/index.html",import.meta.url),"utf8");
 const strip=html.slice(html.indexOf('<details id="projects-panel"'),html.indexOf('<section id="main-view"'));
 assert.match(strip,/<summary class="queue-summary">/);assert.match(strip,/id="projects-summary"/);assert.doesNotMatch(strip,/<details[^>]*\sopen(?:\s|=|>)/);
 assert.ok(html.indexOf('<details id="projects-panel"')<html.indexOf('id="work-view"'));
});


test("compact filter controls show hidden active filters and reset without clearing search",()=>{
 const source=readFileSync(new URL("../public/workboard.js",import.meta.url),"utf8");
 const definition=source.slice(source.indexOf("function updateFilterControls("),source.indexOf("function render()"));
 const fields=new Map(),$=selector=>{if(!fields.has(selector))fields.set(selector,{value:"",open:false});return fields.get(selector);};
 $("#archive-filter").value="active";$("#task-search").value="visible search";$("#board-filter-menu").open=true;
 const state={authenticated:true};let renders=0;
 const context={state,$,render(){renders++;runInNewContext(`${definition};updateFilterControls();`,context);}};
 const update=()=>runInNewContext(`${definition};updateFilterControls();`,context);
 update();assert.equal($("#filter-count").hidden,true);assert.equal($("#reset-filters").disabled,true);
 for(const selector of ["#project-filter","#owner-filter","#type-filter","#label-filter"])$(selector).value="selected";
 $("#archive-filter").value="archive";update();assert.equal($("#filter-count").textContent,"5");assert.equal($("#filter-count").hidden,false);assert.equal($("#board-filter-menu").open,true);
 runInNewContext(`${definition};resetBoardFilters();`,context);assert.equal(renders,1);assert.equal($("#archive-filter").value,"active");assert.equal($("#filter-count").hidden,true);assert.equal($("#task-search").value,"visible search");
 state.authenticated=false;update();assert.equal($("#board-filter-menu").open,false);assert.equal($("#filter-count").textContent,"");
});

test("compact toolbar keeps all five accessible filters in a default-closed disclosure",()=>{
 const html=readFileSync(new URL("../public/index.html",import.meta.url),"utf8");
 const start=html.indexOf('<details id="board-filter-menu"'),end=html.indexOf('</details>',start),menu=html.slice(start,end);
 assert.ok(start>0);assert.match(menu,/<summary id="board-filter-toggle"/);assert.doesNotMatch(menu,/<details[^>]*\sopen(?:\s|=|>)/);
 for(const [id,label]of [["archive-filter","Task visibility"],["project-filter","Filter by project"],["owner-filter","Filter by owner"],["type-filter","Task type"],["label-filter","Label"]])assert.match(menu,new RegExp(`id="${id}" aria-label="${label}"`));
 assert.equal((html.match(/id="task-search"/g)||[]).length,1);assert.ok(html.indexOf('id="task-search"')<start);assert.ok(html.indexOf('id="task-total"')>end);
});


test("System agent editor keeps drafts separate and preserves disabled default assignments",()=>{
 const source=readFileSync(new URL("../public/workboard.js",import.meta.url),"utf8");
 const definition=source.slice(source.indexOf("function renderAgentDraft("),source.indexOf('$("#edit-agents").addEventListener'));
 const element=(tag,text)=>({tag,text,children:[],append(...v){this.children.push(...v);},prepend(v){this.children.unshift(v);},replaceChildren(){this.children=[];},setAttribute(){},addEventListener(name,fn){this[name]=fn;}});
 const nodes=new Map(),$=id=>{if(!nodes.has(id))nodes.set(id,element("div"));return nodes.get(id);};
 const system={revision:2,agents:[{id:"main",label:"Main",role:"main",enabled:true},{id:"coder",label:"Coder",role:"coder",enabled:false},{id:"qa",label:"QA",role:"qa",enabled:true}],defaults:{main:"main",coder:"coder",qa:"qa"}};
 const agentDraft=structuredClone(system);runInNewContext(`${definition};renderAgentDraft();`,{agentDraft,$,node:element,readable:s=>s});
 const selects=$("#agent-defaults").children.map(label=>label.children[0]);assert.equal(selects[1].value,"coder");assert.match(selects[1].children[0].text,/disabled/);
 const labelInput=$("#agent-fields").children[1].children[1].children[0];labelInput.value="Custom coder";labelInput.input();assert.equal(agentDraft.agents[1].label,"Custom coder");assert.equal(system.agents[1].label,"Coder");
 assert.match(source,/state.system = data.system/);assert.match(source,/expected_revision:agentDraft.revision/);
});


test("System roster is generic, text-only, and cleared when access ends",()=>{
 const source=readFileSync(new URL("../public/workboard.js",import.meta.url),"utf8");const definition=source.slice(source.indexOf("function renderSystem("),source.indexOf("function renderAgentDraft("));
 const nodes=new Map(),node=(tag,text)=>({tag,text,children:[],append(...v){this.children.push(...v);},replaceChildren(){this.children=[];}}),$=id=>{if(!nodes.has(id))nodes.set(id,node("div"));return nodes.get(id);};
 const state={authenticated:true,connected:true,execution:{paused:false},plans:[],system:{editable:true,agents:[{id:"custom",label:"<script>name</script>",role:"coder",instructions:"<b>instructions</b>",enabled:true}],defaults:{coder:"custom"}},runs:[],tasks:[]};
 const context={state,$,node,agentAssignments:agent=>agent.id==="retired"?["running: Retained task"]:[],allowed:()=>true,array:v=>Array.isArray(v)?v:[],readable:v=>v};runInNewContext(`${definition};renderSystem();`,context);
 assert.match(JSON.stringify($("#system-agents")),/<script>name/);assert.equal($("#system-agents").children[0].children[0].text,"<script>name</script>");assert.match(JSON.stringify($("#system-agents")),/Default: coder/);
 state.runs=[{stage:"qa",status:"running",agents:{qa:{id:"retired",label:"Previous QA",role:"qa"}}}];state.tasks=[];context.displayHistoricalTime=runInNewContext(`(${source.slice(source.indexOf("function displayHistoricalTime("),source.indexOf("function readable("))})`);context.commandButton=()=>node("button");runInNewContext(`${definition};renderSystem();`,context);assert.match(JSON.stringify($("#system-agents")),/Previous QA/);assert.match(JSON.stringify($("#system-agents")),/Retained for current work/);
 state.authenticated=false;runInNewContext(`${definition};renderSystem();`,context);assert.equal($("#system-agents").children.length,0);assert.equal($("#system-activity").children.length,0);assert.match($("#system-status").textContent,/Sign in/);
});


test("agent activity uses frozen assignments rather than current defaults",()=>{
 const source=readFileSync(new URL("../public/workboard.js",import.meta.url),"utf8"),definition=source.slice(source.indexOf("function agentAssignments("),source.indexOf("function renderSystem("));
 const state={runs:[{task_id:"task",stage:"qa",status:"running",agents:{qa:{id:"old-qa"}}},{task_id:"task",stage:"coder",status:"succeeded",agents:{coder:{id:"coder"}}}],tasks:[{id:"task",title:"Current task"}],plans:[{status:"generating",goal:"Plan goal",agent:{id:"main"}}]};
 const run=id=>runInNewContext(`${definition};agentAssignments({id});`,{state,id,liveRunStates:new Set(["running"]),readable:v=>v});
 assert.equal(run("old-qa")[0],"running: Current task");assert.equal(run("new-qa").length,0);assert.equal(run("coder").length,0);assert.equal(run("main")[0],"generating: Plan goal");
});

test("queued cards match execution priority and oldest Ready first, with unprioritized work last", () => {
  const items = [
    {id:"none", priority:0, created_at:"2026-01-01", ready_at:"2026-01-01"},
    {id:"normal-later", priority:3, created_at:"2025-01-01", ready_at:"2026-01-03"},
    {id:"low", priority:4, created_at:"2025-01-01"},
    {id:"normal-first", priority:3, created_at:"2026-01-02", ready_at:"2026-01-02"},
    {id:"high", priority:2, created_at:"2026-01-04"},
    {id:"urgent", priority:1, created_at:"2026-01-05"},
  ];
  const original = items.map(t => t.id);
  assert.deepEqual(sortQueuedTasks(items).map(t => t.id), ["urgent","high","normal-first","normal-later","low","none"]);
  assert.deepEqual(items.map(t => t.id), original);
  assert.deepEqual(sortQueuedTasks([{id:"b",priority:3},{id:"a",priority:3}]).map(t => t.id), ["a","b"]);
});

test("card priority saves the chosen field and reviewed processing policy; Edit and View remain separate", () => {
  const source=readFileSync(new URL("../public/workboard.js",import.meta.url),"utf8");
  const definition=source.slice(source.indexOf("function taskCard("),source.indexOf("function setView("));
  const command=source.slice(source.indexOf("function commandButton("),source.indexOf("function section("));
  const node=(tag,text,cls)=>({tag,text,cls,children:[],dataset:{},events:{},attrs:{},classList:{add(){}},append(...items){this.children.push(...items);},setAttribute(k,v){this.attrs[k]=v;},addEventListener(k,v){this.events[k]=v;}});
  const patches=[],edits=[],views=[];
  const state={runs:[],executionActivity:{}};
  let writable=true;
  const context={node,state,kanbanMovePayload,cardPriorityPayload,priorities:{0:"No priority",1:"Urgent",2:"High",3:"Normal",4:"Low"},shortId:x=>x,activeRun:(task,runs)=>runs.find(r=>r.task_id===task.id)||null,allowed:()=>writable,taskBlocker:()=>null,readable:x=>x,array:x=>x||[],mutate:(...args)=>patches.push(args),openEditor:task=>edits.push(task),openDetail:id=>views.push(id)};
  const base={id:"queued",title:"Queued task",revision:4,priority:3,status:"ready",workflow:"manual",automation_policy:"manual"};
  const render=task=>runInNewContext(`${command};${definition};taskCard(task);`,{...context,task});
  const card=render(base),actions=card.children[0].children[2],picker=actions.children[0],edit=actions.children[1],view=card.children[1];
  assert.equal(card.tag,"article");assert.equal(view.tag,"button");assert.equal(picker.tag,"select");
  assert.equal(picker.value,"3");assert.equal(picker.disabled,false);assert.equal(edit.disabled,false);
  assert.deepEqual(picker.children.map(o=>o.value),["1","2","3","4","0"]);
  picker.value="0";picker.events.change();
  assert.equal(patches[0][0],"/tasks/queued");assert.equal(patches[0][1],"PATCH");
  assert.deepEqual(JSON.parse(JSON.stringify(patches[0][2])),{expected_revision:4,priority:0,automation_policy:"manual"});
  assert.equal(edits.length,0);assert.equal(views.length,0);
  edit.events.click();assert.equal(edits[0],base);assert.equal(views.length,0);
  view.events.click();assert.deepEqual(views,["queued"]);
  for(const task of [{...base,status:"done"},{...base,status:"cancelled"},{...base,archived_at:"2026-10-23"}]){
    const controls=render(task).children[0].children[2].children;assert.equal(controls[0].disabled,true);assert.equal(controls[1].disabled,true);
  }
  state.runs=[{id:"run",task_id:"queued"}];context.activityTitle=()=>"Running";
  let controls=render(base).children[0].children[2].children;assert.equal(controls[0].disabled,true);assert.equal(controls[1].disabled,true);
  state.runs=[];writable=false;controls=render(base).children[0].children[2].children;assert.equal(controls[0].disabled,true);assert.equal(controls[1].disabled,true);
});

test("card reprioritization preserves only current automatic enrollment, never starts manual or consumed work", () => {
  const task={id:"queued",revision:7,workflow:"engineering",automation_policy:"ready_to_done",automation_revision:7};
  assert.deepEqual(cardPriorityPayload(task,"1"),{expected_revision:7,priority:1,automation_policy:"ready_to_done"});
  for(const other of [{...task,automation_policy:"manual"},{...task,automation_revision:6},{...task,automation_revision:null},{...task,workflow:"manual"}])assert.deepEqual(cardPriorityPayload(other,"0"),{expected_revision:7,priority:0,automation_policy:"manual"});
});

test("Kanban Ready drops explicitly queue engineering while Backlog unqueues and runtime stages stay protected", () => {
  const task={id:"ticket",revision:5,status:"backlog",workflow:"engineering",repository:"owner/repo",automation_policy:"manual",qa_policy:"skip",merge_policy:"manual"};
  assert.deepEqual(kanbanMovePayload(task,"ready"),{expected_revision:5,status:"ready",automation_policy:"ready_to_done"});
  assert.deepEqual(kanbanMovePayload({...task,status:"ready"},"backlog"),{expected_revision:5,status:"backlog",automation_policy:"manual"});
  assert.deepEqual(kanbanMovePayload({...task,workflow:"manual",repository:null},"ready"),{expected_revision:5,status:"ready",automation_policy:"manual"});
  for(const status of ["active","review","done","cancelled"])assert.equal(kanbanMovePayload(task,status),null);
  for(const status of ["active","review","done","cancelled"])assert.equal(kanbanMovePayload({...task,status},"ready"),null);
  assert.equal(kanbanMovePayload(task,"backlog"),null);
  assert.equal(kanbanMovePayload({...task,archived_at:"2026-10-23"},"ready"),null);
  assert.equal(kanbanMovePayload({...task,repository:null},"ready"),null);
  for(const status of ["running","unknown","awaiting_merge"])assert.equal(kanbanMovePayload(task,"ready",[{id:"run",task_id:task.id,status}]),null);
  assert.equal(task.automation_policy,"manual");assert.equal(task.qa_policy,"skip");assert.equal(task.merge_policy,"manual");
});

test("drop handlers ignore external data and recheck live ownership while preserving the dragged revision", async () => {
  const source=readFileSync(new URL("../public/workboard.js",import.meta.url),"utf8");
  const definition=source.slice(source.indexOf("function clearKanbanDrag("),source.indexOf("function taskCard("));
  const target=()=>({dataset:{dropAllowed:"true"},events:{},classList:{add(){},remove(){}},contains(){return false;},addEventListener(k,v){this.events[k]=v;}});
  const ready=target(),active=target(),patches=[];
  const task={id:"queued",revision:4,status:"backlog",workflow:"engineering",repository:"owner/repo"};
  const context={draggedTask:null,state:{runs:[],stage:"backlog"},document:{querySelectorAll:()=>[ready,active]},allowed:()=>true,kanbanMovePayload,labels:{ready:"Ready",active:"Active"},mutate:(...args)=>{patches.push(args);return Promise.resolve({task:{}});},render(){}};
  Object.assign(context,{ready,active});runInNewContext(`${definition};wireKanbanDrop(ready,"ready");wireKanbanDrop(active,"active");`,context);
  const event=()=>({dataTransfer:{getData:()=>"forged-other-id"},preventDefault(){this.prevented=true;}});
  ready.events.drop(event());assert.equal(patches.length,0);
  context.draggedTask={...task};active.events.drop(event());assert.equal(patches.length,0);
  context.state.runs=[{id:"run",task_id:task.id,status:"running"}];ready.events.drop(event());assert.equal(patches.length,0);
  context.state.runs=[];const drop=event();ready.events.drop(drop);await Promise.resolve();
  assert.equal(drop.prevented,true);assert.equal(patches.length,1);
  assert.equal(patches[0][0],"/tasks/queued");assert.deepEqual(JSON.parse(JSON.stringify(patches[0][2])),{expected_revision:4,status:"ready",automation_policy:"ready_to_done"});
  assert.equal(context.draggedTask,null);assert.equal(context.state.stage,"ready");assert.equal(ready.dataset.dropAllowed,undefined);
});

test("repository-free Research preserves automatic Ready/priority payloads without inventing Git authority",async()=>{
 const {repositoryFreeResearch,defaultQaPolicy,activityTitle}=await import("../public/workboard-model.js");
 const research={id:"research",revision:2,workflow:"engineering",task_type:"research",repository:null,status:"backlog",automation_policy:"ready_to_done",automation_revision:2};
 assert.equal(repositoryFreeResearch(research),true);assert.equal(repositoryFreeResearch({...research,repository:"owner/repo"}),false);assert.equal(repositoryFreeResearch({...research,task_type:"feature"}),false);
 assert.equal(defaultQaPolicy("research"),"skip");assert.equal(kanbanMovePayload(research,"ready").automation_policy,"ready_to_done");
 assert.equal(cardPriorityPayload({...research,status:"ready"},1).automation_policy,"ready_to_done");
 assert.equal(automationLabel(research),"Queued for automatic processing");
 assert.equal(activityTitle({stage:"report_ready"}),"Research report delivered");
 const payload=taskPayload({...research,title:"A research report",objective:"Cite sources",acceptance_criteria:"Bounded report",qa_policy:"required"});assert.equal(payload.repository,null);assert.equal(payload.qa_policy,"required");
});


test("Research editor explains report delivery without coding checks or repository queue copy",()=>{
 const source=readFileSync(new URL("../public/workboard.js",import.meta.url),"utf8");
 const definition=source.slice(source.indexOf("function updateWorkflowHint()"),source.indexOf("async function mutate("));
 const option={textContent:""},nodes={};
 const $=id=>nodes[id]??=(id==="#task-workflow"?{value:"engineering",querySelector:()=>option}:id==="#task-type"?{value:"research"}:id==="#task-repository"?{value:""}:{});
 const context={$,state:{capabilities:{ready_to_done:true},codingAgents:{}},codingAgentSelection:()=>({agents:[]})};
 runInNewContext(`${definition};updateWorkflowHint();`,context);
 assert.equal(option.textContent,"Research · agent → report");assert.equal($("#task-repository").required,false);assert.equal($("#merge-policy-field").hidden,true);
 assert.match($("#editor-queue-hint").textContent,/web research.*report/);assert.doesNotMatch($("#editor-queue-hint").textContent,/Coder|required checks|repository/);
 $("#task-repository").value="owner/repo";runInNewContext("updateWorkflowHint()",context);
 assert.match(option.textContent,/Engineering/);assert.equal($("#merge-policy-field").hidden,false);
 assert.doesNotMatch(readFileSync(new URL("../public/index.html",import.meta.url),"utf8"),/Skip QA · Coder checks still required/);
});
