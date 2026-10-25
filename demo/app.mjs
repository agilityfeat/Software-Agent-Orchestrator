import {DEFAULT_REQUEST,policyFromPrompt} from './prompt-policy.mjs';
import {renderAgents} from './agent-activity.mjs';
import {BASE_SOURCE,EXPENSES,money,criteria,baselineOutputs,checkOutput,canAccept,score} from './core.mjs';
import {Controller} from './controller.mjs';
import {execute,cancelAll} from './sandbox.mjs';
import {LocalModel} from './model.mjs';
import {load,save} from './storage.mjs';
const $=id=>document.getElementById(id);
const text=(tag,content,className='')=>{const node=document.createElement(tag);node.textContent=content;if(className)node.className=className;return node;};
const labels={eligible:'Eligible',manager_review:'Manager review',receipt_required:'Receipt needed',paid:'Paid'};
let accepted=BASE_SOURCE,acceptedResults=baselineOutputs(),candidateResults=null,selected='e1',tab='plan',preview='accepted',history=[],renderEpoch=0,saveQueue=Promise.resolve(),loading=false;
const model=new LocalModel(value=>{$('model-status').textContent=value;updateControls();});
const controller=new Controller({model,execute,onChange:()=>{render();persist();}});
function notice(message=''){ $('notice').textContent=message;$('notice').hidden=!message; }
function persist(){const value={schema:1,accepted,task:controller.task,history:history.slice(-5),layout:$('workspace').classList.contains('stacked')};saveQueue=saveQueue.catch(()=>{}).then(()=>save(value)).catch(()=>notice('Browser storage is unavailable. This session still works; export the run before closing.'));}
function mode(){return $('mode').value;}
function updateControls(){
  const busy=controller.busy||loading,t=controller.task;
  ['request','mode','reset','model-file'].forEach(id=>$(id).disabled=busy);
  $('request').disabled=busy||mode()==='recorded';
  $('plan').disabled=busy||mode()==='recorded'||(mode()==='live'&&!model.ready);
  $('run').disabled=busy||t?.status!=='Ready'||t?.mode!==mode();
  $('run').hidden=t?.status==='Review'||t?.status==='Done';
  $('accept').hidden=t?.status!=='Review'||mode()==='recorded';$('accept').disabled=busy||!canAccept(t);
  $('cancel').hidden=!busy;$('export').disabled=!t;
  $('load-model').disabled=busy||model.ready;
  $('show-candidate').disabled=!t?.candidate||!candidateResults;
}
function renderBoard(){
  const t=controller.task;$('board').replaceChildren();
  for(const status of ['Backlog','Ready','Active','Review','Done']){
    const col=text('div','','column'),head=text('div',status,'column-heading'),current=t&&(t.status===status||(t.status==='Blocked'&&status==='Active'));
    head.append(text('b',current?'1':'0'));const slot=text('div',current?'Expense policy':'—',`slot${current?' current':''}`);
    if(current)slot.append(text('small',t.status==='Blocked'?'Blocked':t.phase));col.append(head,slot);$('board').append(col);
  }
  renderAgents($('agents'),t);
  $('task-mode').textContent=t?.mode?.toUpperCase()||mode().toUpperCase();
  $('attempt').textContent=t?.attempt?`Attempt ${t.attempt}/3`:t?.status||'Waiting for a task';
  $('test-count').textContent=t?.evidence?`${t.evidence.rows.filter(r=>r.pass).length}/${t.evidence.rows.length}`:'—';
}
function renderArtifact(){
  const body=$('artifact-body'),t=controller.task;body.replaceChildren();
  document.querySelectorAll('[data-tab]').forEach(b=>{b.classList.toggle('selected',b.dataset.tab===tab);b.setAttribute('aria-selected',String(b.dataset.tab===tab));});
  $('artifact-title').textContent=t?.phase||'A clear contract comes first';
  if(!t){body.append(text('p','Describe your expense policy above. The team will turn it into a plan for you to review.'));const list=text('ul','');['Same downloaded model for every role','Real tests, not model-generated pass/fail','At most two corrections; you accept the result'].forEach(x=>list.append(text('li',x)));body.append(list);return;}
  if(tab==='plan'){
    body.append(text('p',t.plan?.summary||'The planner is preparing the work…'));const list=text('ol','');for(const step of t.plan?.steps||[])list.append(text('li',step));body.append(list,text('h4','What the change must do'));const checks=text('ul','');criteria(t.rule).forEach(c=>checks.append(text('li',c)));body.append(checks);
    if(t.mode==='guided')body.append(text('p','Guided example intentionally starts with a >= boundary bug. Agents are scripted; the runner executes the actual candidate.','muted'));
  } else if(tab==='code'){
    body.append(text('h4','Accepted source at task start'),text('pre',t.base),text('h4',`Candidate ${t.hash?.slice(0,12)||'not generated'}`),text('pre',t.candidate||'No candidate yet.'));
  } else if(tab==='tests'){
    if(!t.evidence){body.append(text('p','No evidence yet. Tests run against an isolated candidate.'));return;}
    body.append(text('p',`Tested SHA-256: ${t.evidence.hash.slice(0,20)}…`,'muted'));
    for(const r of t.evidence.rows){const row=text('div','','test-row');row.append(text('b',r.pass?'✓':'×',r.pass?'pass':'fail'),text('span',r.name),text('span',r.pass?'PASS':`Expected ${r.expected}`));body.append(row);if(!r.pass)body.append(text('pre',JSON.stringify(r.actual)));}
  } else {
    for(const event of t.events){const row=text('div','','activity-entry');row.append(text('strong',event.role),text('small',new Date(event.at).toLocaleTimeString()),text('div',event.text));body.append(row);}
  }
}
function renderExpenses(){
  const values=preview==='candidate'&&candidateResults?candidateResults:acceptedResults;
  $('show-accepted').classList.toggle('selected',preview==='accepted');$('show-candidate').classList.toggle('selected',preview==='candidate');
  $('show-accepted').setAttribute('aria-pressed',String(preview==='accepted'));$('show-candidate').setAttribute('aria-pressed',String(preview==='candidate'));
  $('submitted-total').textContent=money(EXPENSES.filter(e=>e.status!=='paid').reduce((a,e)=>a+e.amountCents,0));
  $('paid-total').textContent=money(EXPENSES.filter(e=>e.status==='paid').reduce((a,e)=>a+e.amountCents,0));
  $('needs-count').textContent=values.filter(x=>['manager_review','receipt_required'].includes(x.status)).length;
  $('needs-note').textContent=preview==='candidate'?'Candidate policy':'Accepted policy';
  const query=$('search').value.toLowerCase(),filter=$('filter').value,body=$('expenses');body.replaceChildren();
  for(const [i,e]of EXPENSES.entries()){
    const result=values[i];if(!`${e.merchant} ${e.person}`.toLowerCase().includes(query))continue;
    if(filter==='attention'&&!['manager_review','receipt_required'].includes(result.status))continue;
    if(['paid','eligible'].includes(filter)&&result.status!==filter)continue;
    const tr=text('tr','',e.id===selected?'selected-row':'');tr.tabIndex=0;tr.setAttribute('aria-label',`${e.merchant}, ${money(e.amountCents)}, ${labels[result.status]}`);
    tr.onclick=()=>{selected=e.id;renderExpenses();};tr.onkeydown=event=>{if(event.key==='Enter'||event.key===' '){event.preventDefault();selected=e.id;renderExpenses();}};
    const merchant=text('td',''),cell=text('div','','merchant-cell'),info=text('div','');info.append(text('b',e.merchant),text('small',e.person));cell.append(text('span',e.initials,'merchant-icon'),info);merchant.append(cell);
    const badgeCell=text('td','');badgeCell.append(text('span',labels[result.status],`badge ${result.status}`));tr.append(merchant,text('td',money(e.amountCents)),badgeCell);body.append(tr);
  }
  if(!body.children.length){const tr=text('tr',''),td=text('td','No expenses match this filter.');td.colSpan=3;tr.append(td);body.append(tr);}
  const e=EXPENSES.find(e=>e.id===selected),result=values[EXPENSES.indexOf(e)],detail=$('expense-detail');detail.replaceChildren(text('h3',`${e.merchant} · ${money(e.amountCents)}`),text('p',result.reason));
  const receipt=text('button',e.receipt?'View receipt':'Receipt missing');receipt.onclick=()=>{$('receipt-copy').textContent=e.receipt?`${e.merchant} — ${money(e.amountCents)}. Paid by ${e.person}.`:'No receipt attached to this example expense.';$('receipt-dialog').showModal();};
  const approve=text('button',result.canApprove?'Approve expense':labels[result.status],'approve-expense');approve.disabled=!result.canApprove;approve.onclick=()=>notice('Demo action only: this expense is eligible under the displayed policy. No money moves and the fixture stays unchanged.');detail.append(receipt,approve);
  $('preview-status').textContent=preview==='candidate'?`Candidate ${controller.task?.hash?.slice(0,10)||''} · not yet accepted`:'Accepted version · local only';
}
function render(){renderBoard();renderArtifact();renderExpenses();updateControls();}
function validateCandidatePreview(task,values){
  const cases=EXPENSES.map(input=>({name:input.merchant,input}));
  const failures=score(cases,values,task.rule).filter(row=>!row.pass);
  if(failures.length)throw Error(`Candidate violates current policy: ${failures.map(row=>`${row.name} must be ${row.expected}`).join('; ')}. Nothing accepted.`);
}
async function refreshCandidate(){
  const t=controller.task;if(!t?.candidate)return;
  const epoch=++renderEpoch,source=t.candidate;const results=await execute(source,EXPENSES);
  if(epoch!==renderEpoch||t!==controller.task||source!==t.candidate)return;
  try {validateCandidatePreview(t,results);}catch(error){candidateResults=null;preview='accepted';controller.block(error);throw error;}
  candidateResults=results;preview='candidate';render();
}
async function safely(action){notice();try{await action();}catch(error){notice(String(error.message||error));}finally{render();}}
$('request-form').onsubmit=event=>{event.preventDefault();safely(async()=>{const request=$('request').value;let rule;try{rule=policyFromPrompt(request);}catch(error){controller.task=null;candidateResults=null;preview='accepted';persist();throw error;}candidateResults=null;preview='accepted';tab='plan';await controller.plan({request,rule,mode:mode(),base:accepted});});};
$('request').oninput=()=>{if(controller.busy||mode()==='recorded'||!controller.task)return;controller.task=null;candidateResults=null;preview='accepted';render();persist();};
$('run').onclick=()=>safely(async()=>{loading=true;updateControls();try{await controller.run();await refreshCandidate();if(controller.task.mode==='live'){history.push(structuredClone(controller.task));persist();}tab='tests';}finally{loading=false;}});
$('accept').onclick=()=>safely(async()=>{loading=true;updateControls();try{const task=controller.task,source=task.candidate,values=await execute(source,EXPENSES);if(task!==controller.task||source!==task.candidate)throw Error('Preview validation failed; nothing accepted.');try{validateCandidatePreview(task,values);}catch(error){candidateResults=null;preview='accepted';controller.block(error);throw error;}accepted=await controller.accept();acceptedResults=values;preview='accepted';if(task.mode==='live')history.push(structuredClone(task));persist();notice('Change accepted locally. The expense inbox is now running the tested policy.');}finally{loading=false;}});
$('cancel').onclick=()=>{renderEpoch++;cancelAll();loading=false;controller.cancel();};
$('load-model').onclick=()=>startModel();$('model-file').onchange=event=>startModel(event.target.files[0]);
async function startModel(file=null){await safely(async()=>{if(model.ready)return;loading=true;updateControls();try{await navigator.storage?.persist();await model.start(file);$('mode').value='live';setModeNote();}finally{loading=false;}});}
function setModeNote(){ $('mode-note').textContent=mode()==='live'?'One downloaded Gemma · three separate contexts · no cloud inference.':mode()==='recorded'?'Historical live evidence. Read-only; no new inference or publication.':'Scripted agents, real executed tests. No model download required.'; }
$('mode').onchange=()=>safely(async()=>{setModeNote();candidateResults=null;preview='accepted';if(mode()==='recorded'){const last=history.at(-1);if(!last){controller.task=null;notice('No live run recorded yet. Complete a Live run first; Guided examples are never labeled as real recordings.');}else{controller.task={...structuredClone(last),mode:'recorded'};$('request').value=controller.task.request||DEFAULT_REQUEST;await refreshCandidate();}}else{controller.task=null;}persist();});
$('show-accepted').onclick=()=>{preview='accepted';renderExpenses();};$('show-candidate').onclick=()=>{preview='candidate';renderExpenses();};
$('search').oninput=renderExpenses;$('filter').onchange=renderExpenses;
for(const button of document.querySelectorAll('[data-tab]'))button.onclick=()=>{tab=button.dataset.tab;renderArtifact();};
$('export').onclick=()=>{if(!controller.task)return;const blob=new Blob([JSON.stringify({schema:1,exportedAt:new Date().toISOString(),task:controller.task},null,2)],{type:'application/json'});const url=URL.createObjectURL(blob),a=document.createElement('a');a.href=url;a.download='agent-demo-run.json';a.click();setTimeout(()=>URL.revokeObjectURL(url),1000);};
function layout(stacked){$('workspace').classList.toggle('stacked',stacked);$('layout').textContent=stacked?'◫ Side by side':'▤ Top / bottom';$('layout').setAttribute('aria-label',stacked?'Switch to side by side layout':'Switch to top and bottom layout');$('divider').setAttribute('aria-orientation',stacked?'horizontal':'vertical');persist();}
$('layout').onclick=()=>layout(!$('workspace').classList.contains('stacked'));
let split=48;function resize(value){split=Math.max(30,Math.min(70,value));$('workspace').style.setProperty('--split',`${split}%`);$('divider').setAttribute('aria-valuenow',String(Math.round(split)));}
$('divider').onpointerdown=event=>{event.preventDefault();const divider=$('divider');divider.setPointerCapture(event.pointerId);const move=e=>{const r=$('workspace').getBoundingClientRect();resize($('workspace').classList.contains('stacked')?(e.clientY-r.top)/r.height*100:(e.clientX-r.left)/r.width*100);};divider.onpointermove=move;divider.onpointerup=()=>{divider.onpointermove=null;divider.onpointerup=null;};};
$('divider').onkeydown=event=>{if(['ArrowLeft','ArrowUp','ArrowRight','ArrowDown'].includes(event.key)){event.preventDefault();resize(split+(['ArrowLeft','ArrowUp'].includes(event.key)?-2:2));}};
$('reset').onclick=()=>safely(async()=>{if(!confirm('Reset the demo workspace? The downloaded model will remain cached.'))return;accepted=BASE_SOURCE;acceptedResults=baselineOutputs();candidateResults=null;controller.task=null;preview='accepted';renderEpoch++;$('request').value=DEFAULT_REQUEST;persist();});
render();
try {
  const stored=await load();if(stored?.schema===1){
    history=(stored.history||[]).filter(t=>t.mode==='live'&&['Review','Done'].includes(t.status)).slice(-5);
    if(stored.accepted&&stored.accepted!==BASE_SOURCE){const values=await execute(stored.accepted,EXPENSES);if(values.every(checkOutput)){accepted=stored.accepted;acceptedResults=values;}}
    if(stored.task){controller.task=stored.task;$('request').value=controller.task.request||DEFAULT_REQUEST;if(['Ready','Review'].includes(controller.task.status)&&controller.task.mode!=='recorded'){try{const interpreted=policyFromPrompt(controller.task.request);if(interpreted.thresholdCents!==controller.task.rule?.thresholdCents||interpreted.receiptCents!==controller.task.rule?.receiptCents)throw Error('Changed request');}catch{const error=Error('This saved plan needs to be created again from its request. Review the request and choose Create plan.');controller.block(error);notice(error.message);}}if(controller.task.status==='Active'){controller.task.status='Blocked';controller.task.phase='Interrupted by reload';}if(controller.task.mode!=='recorded')$('mode').value=controller.task.mode;else $('mode').value='recorded';setModeNote();}
    if(stored.layout)layout(true);
    if(controller.task?.candidate&&controller.task.status!=='Blocked')await refreshCandidate();
  }
}catch(error){notice(`Workspace restore could not complete: ${error.message}. The baseline remains available.`);}
render();
