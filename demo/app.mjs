import {mountRunLog} from './run-log.mjs';
import {renderAgents} from './agent-activity.mjs';
import {DEFAULT_REQUEST,canonicalBundle,validateBundle,canAccept,successfulLiveRun,digest} from './page-contract.mjs';
import {INITIAL_BUNDLE} from './page-baseline.mjs';
import {Controller} from './page-controller.mjs';
import {createPagePreview,runPageChecks} from './page-runtime.mjs';
import {LocalModel} from './model.mjs';
import {load,save} from './storage.mjs';
const $=id=>document.getElementById(id);
const text=(tag,content,className='')=>{const node=document.createElement(tag);node.textContent=content;if(className)node.className=className;return node;};
let accepted=structuredClone(INITIAL_BUNDLE),acceptedStorage={},history=[],legacy=null,tab='plan',preview='accepted',saveQueue=Promise.resolve(),loading=false,previewEpoch=0,previewKey='',previewHandle=null,previewAbort=null,acceptedGeneration=0,candidateSession=null,uiEpoch=0;
const runLog=mountRunLog();
const model=new LocalModel(value=>{$('model-status').textContent=value;updateControls();},undefined,event=>runLog.model(event));
const controller=new Controller({model,execute:(bundle,checks,options)=>{runLog.observe(controller.task,controller.busy);return runLog.checks(runPageChecks,bundle,checks,options);},onChange:()=>{render();persist();}});
function notice(message='',error=false){$('notice-message').textContent=message;$('notice').hidden=!message;$('notice-details').hidden=!message||!error;}
function openErrorLog(event){if(controller.task?.status==='Blocked')runLog.restoreOutputs(controller.task);runLog.open({details:true,trigger:event.currentTarget});}
$('notice-details').onclick=openErrorLog;
function errorLogButton(){const button=text('button','View error log','quiet error-details');button.type='button';button.setAttribute('aria-haspopup','dialog');button.setAttribute('aria-controls','run-log-dialog');button.onclick=openErrorLog;return button;}
function mode(){return $('mode').value;}
function persist(){const value=structuredClone({schema:3,accepted,acceptedStorage,task:controller.task,history:history.slice(-5),legacy,layout:$('workspace').classList.contains('stacked')});saveQueue=saveQueue.catch(()=>{}).then(()=>save(value)).catch(()=>notice('Browser storage is unavailable. Export your workspace before closing.'));}
function updateControls(){
 const t=controller.task,busy=controller.busy||loading;
 for(const id of ['request','mode','model-file'])$(id).disabled=busy;
 $('request').disabled=busy||mode()==='recorded';$('plan').disabled=busy||mode()==='recorded'||mode()==='live'&&!model.ready;
 $('run').disabled=busy||t?.status!=='Ready'||t.mode!==mode();$('run').hidden=['Review','Done'].includes(t?.status);
 $('accept').hidden=t?.status!=='Review'||mode()==='recorded';$('accept').disabled=busy||!canAccept(t);
 $('cancel').hidden=!busy;$('start-over').hidden=!t&&!loading;$('start-over').disabled=false;$('reset').disabled=false;$('export').disabled=false;$('load-model').disabled=busy||model.ready;
 $('show-candidate').disabled=!t?.candidate||!['Review','Done'].includes(t.status);$('export-legacy').hidden=!legacy;
}
function renderBoard(){
 const t=controller.task;$('board').replaceChildren();
 for(const status of ['Backlog','Ready','Active','Review','Done']){const col=text('div','','column'),head=text('div',status,'column-heading'),current=t&&(t.status===status||t.status==='Blocked'&&status==='Active');head.append(text('b',current?'1':'0'));const slot=text('div',current?'App change':'—',`slot${current?' current':''}`);if(current)slot.append(text('small',t.status==='Blocked'?'Blocked':t.phase));col.append(head,slot);$('board').append(col);}
 renderAgents($('agents'),t);$('task-mode').textContent=t?.mode?.toUpperCase()||mode().toUpperCase();$('attempt').textContent=t?.attempt?`Attempt ${t.attempt}/3`:t?.status||'Waiting for a task';$('test-count').textContent=t?.evidence?`${t.evidence.rows.filter(row=>row.pass).length}/${t.evidence.rows.length}`:'—';
}
function renderArtifact(){
 const body=$('artifact-body'),t=controller.task;body.replaceChildren();for(const button of document.querySelectorAll('[data-tab]')){button.classList.toggle('selected',button.dataset.tab===tab);button.setAttribute('aria-selected',String(button.dataset.tab===tab));}$('artifact-title').textContent=t?.phase||'Describe your app change';
 if(!t){body.append(text('p','Ask the team to change this app: add a table column, build an interaction, change its design, or create a new feature. Live agents edit the full HTML, CSS and JavaScript.'));body.append(text('p','Guided demonstrates the date-column example with scripted agents and real browser checks.','muted'));return;}
 if(t.status==='Blocked'){const last=t.events?.filter(event=>event.role==='Controller').at(-1);body.append(text('p',t.phase==='Cancelled'?'This run was stopped.':t.phase==='Interrupted by reload'?'This run was interrupted when the page reloaded.':last?.text||'The team could not finish this request.'),errorLogButton(),text('p','Choose Start over to try a new request. Your accepted app and saved successful runs will stay.','muted'));if(tab==='plan'&&!t.plan)return;}
 if(tab==='plan'){if(t.plan)body.append(text('p',t.plan.verification==='preserve'?'Checks preserve existing behavior. Review the code to confirm the requested refactor.':'Checks verify the requested change.','muted'));body.append(text('p',t.plan?.summary||'The planner is reviewing the current app…'));const list=text('ol','');for(const step of t.plan?.steps||[])list.append(text('li',step));body.append(list,text('h4','What the browser will check'));const checks=text('ul','');for(const check of t.plan?.checks||[])checks.append(text('li',check.name));body.append(checks);if(t.mode==='guided')body.append(text('p','Scripted date-column example. Choose Live for your own request.','muted'));}
 else if(tab==='code'){const bundle=t.candidate||t.base;for(const file of ['html','css','js'])body.append(text('h4',file.toUpperCase()),text('pre',bundle?.[file]||'(empty file)'));}
 else if(tab==='tests'){if(!t.evidence){body.append(text('p','Browser checks run against the complete candidate in a fresh preview.'));return;}for(const row of t.evidence.rows){const line=text('div','','test-row');line.append(text('b',row.pass?'✓':'×',row.pass?'pass':'fail'),text('span',row.name),text('span',row.pass?'PASS':'FAIL'));body.append(line);if(!row.pass)body.append(text('pre',JSON.stringify(row.actual)));}}
 else for(const event of t.events){const row=text('div','','activity-entry');row.append(text('strong',event.role),text('small',new Date(event.at).toLocaleTimeString()),text('div',event.text));body.append(row);}
}
async function renderPreview(){
 const t=controller.task,role=preview,bundle=role==='candidate'&&t?.candidate?t.candidate:accepted,source=canonicalBundle(bundle),identity=role==='candidate'?`${t?.id}:${t?.hash}`:`accepted:${acceptedGeneration}`,key=role+'|'+identity+'|'+source;
 $('show-accepted').classList.toggle('selected',role==='accepted');$('show-candidate').classList.toggle('selected',role==='candidate');$('show-accepted').setAttribute('aria-pressed',String(role==='accepted'));$('show-candidate').setAttribute('aria-pressed',String(role==='candidate'));$('preview-status').textContent=role==='candidate'?'Candidate · not yet accepted':'Accepted app · local only';
 if(key===previewKey)return;const epoch=++previewEpoch;previewKey=key;previewHandle?.destroy();previewHandle=null;previewAbort?.abort();previewAbort=new AbortController();
 if(role==='candidate'&&candidateSession?.identity!==identity)candidateSession={identity,storage:structuredClone(t?.previewStorage||acceptedStorage)};
 const storage=structuredClone(role==='accepted'?acceptedStorage:candidateSession.storage),current=()=>epoch===previewEpoch&&key===previewKey;
 try{const handle=await createPagePreview($('app-preview'),bundle,{storage,signal:previewAbort.signal,onStorage:value=>{if(!current())return;if(role==='accepted'&&source===canonicalBundle(accepted)){acceptedStorage=value;persist();}else if(role==='candidate'&&candidateSession?.identity===identity)candidateSession.storage=value;},onError:error=>{if(!current())return;const message=`App preview: ${String(error.message||error)}`;notice(message,true);if(role==='candidate'&&t===controller.task&&source===canonicalBundle(t.candidate)&&['Review','Done'].includes(t.status)){history=history.filter(run=>run.id!==t.id||run.hash!==t.hash);controller.block(Error(message));preview='accepted';void renderPreview();}}});if(!current()){handle.destroy();return;}previewHandle=handle;}
 catch(error){if(current()){previewKey='';notice(`App preview could not load: ${error.message}`,true);if(role==='candidate'&&t===controller.task&&['Review','Done'].includes(t.status)){history=history.filter(run=>run.id!==t.id||run.hash!==t.hash);controller.block(error);preview='accepted';void renderPreview();}}}
}
function render(){runLog.observe(controller.task,controller.busy);renderBoard();renderArtifact();updateControls();void renderPreview();}
async function safely(action){const epoch=uiEpoch;notice();try{await action(epoch);}catch(error){if(epoch===uiEpoch)notice(String(error.message||error),true);}finally{if(epoch===uiEpoch)render();}}
$('request-form').onsubmit=event=>{event.preventDefault();safely(async()=>{preview='accepted';candidateSession=null;tab='plan';await controller.plan({request:$('request').value,mode:mode(),base:accepted});});};
$('request').oninput=()=>{if(controller.busy||mode()==='recorded')return;notice();uiEpoch++;controller.epoch++;controller.abort?.abort();controller.task=null;preview='accepted';render();persist();};
$('run').onclick=()=>safely(async()=>{const t=await controller.run();if(t!==controller.task||!canAccept(t))return;t.previewStorage=structuredClone(acceptedStorage);preview='candidate';candidateSession=null;tab='tests';if(successfulLiveRun(t))history.push(structuredClone(t));persist();});
$('accept').onclick=()=>safely(async()=>{const t=controller.task;const bundle=await controller.accept({base:accepted});if(t!==controller.task||t.status!=='Done')throw Error('The task changed; nothing accepted.');accepted=bundle;if(candidateSession?.identity===`${t.id}:${t.hash}`)acceptedStorage=structuredClone(candidateSession.storage);acceptedGeneration++;candidateSession=null;preview='accepted';if(t.mode==='live')history.push(structuredClone(t));persist();notice('Change accepted locally. The app is now running the tested HTML, CSS and JavaScript.');});
function discardPreview(){previewEpoch++;previewKey='';previewAbort?.abort();previewAbort=null;previewHandle?.destroy();previewHandle=null;candidateSession=null;preview='accepted';}
function recoveryMessage(){return mode()==='live'&&!model.ready?'Ready for a new request. Click Load Gemma to resume from the browser cache.':'Ready for a new request. Your accepted app and saved successful runs are unchanged.';}
function startOver(){uiEpoch++;loading=false;discardPreview();controller.restart();tab='plan';if(mode()==='recorded'){$('mode').value=model.ready?'live':'guided';setModeNote();}render();persist();notice(recoveryMessage());$('request').focus();}
$('start-over').onclick=startOver;
$('cancel').onclick=()=>{uiEpoch++;loading=false;discardPreview();controller.cancel();render();persist();notice('Run stopped. Choose Start over to enter a new request.');};
$('load-model').onclick=()=>startModel();$('model-file').onchange=event=>startModel(event.target.files[0]);
async function startModel(file=null){await safely(async epoch=>{if(model.ready)return;loading=true;updateControls();try{await navigator.storage?.persist();if(epoch!==uiEpoch)return;await model.start(file);if(epoch!==uiEpoch)return;$('mode').value='live';setModeNote();}finally{if(epoch===uiEpoch)loading=false;}});}
function setModeNote(){$('mode-note').textContent=mode()==='live'?'One downloaded Gemma · full HTML, CSS and JavaScript · no cloud inference.':mode()==='recorded'?'Review a successful Live app change. Read-only; no new inference.':'Scripted date-column example, real browser checks. No model download.';}
$('mode').onchange=()=>{uiEpoch++;void safely(async epoch=>{controller.epoch++;controller.abort?.abort();controller.task=null;preview='accepted';setModeNote();if(mode()==='recorded'){const last=history.filter(successfulLiveRun).at(-1);if(!last)notice('No successful Live full-app run recorded yet. Complete a Live run first.');else{const t=structuredClone(last);controller.task=t;try{await controller.verify(t);}catch(error){if(epoch===uiEpoch&&t===controller.task){controller.block(error);preview='accepted';}throw error;}if(epoch!==uiEpoch||t!==controller.task)return;t.mode='recorded';$('request').value=t.request;preview='candidate';}}persist();});};
$('show-accepted').onclick=()=>{preview='accepted';void renderPreview();};$('show-candidate').onclick=()=>{if(!['Review','Done'].includes(controller.task?.status))return;preview='candidate';void renderPreview();};
for(const button of document.querySelectorAll('[data-tab]'))button.onclick=()=>{tab=button.dataset.tab;renderArtifact();};
function download(value,name){const blob=new Blob([JSON.stringify(value,null,2)],{type:'application/json'}),url=URL.createObjectURL(blob),a=document.createElement('a');a.href=url;a.download=name;a.click();setTimeout(()=>URL.revokeObjectURL(url),1000);}
$('export').onclick=()=>download({schema:3,exportedAt:new Date().toISOString(),accepted,acceptedStorage,task:controller.task,history,legacy,modelMetrics:model.metrics||[],modelActivity:model.activity||null},'agent-demo-workspace.json');
$('export-legacy').onclick=()=>download(legacy,'previous-agent-demo-workspace.json');
function layout(stacked){$('workspace').classList.toggle('stacked',stacked);$('layout').textContent=stacked?'◫ Side by side':'▤ Top / bottom';$('layout').setAttribute('aria-label',stacked?'Switch to side by side layout':'Switch to top and bottom layout');$('divider').setAttribute('aria-orientation',stacked?'horizontal':'vertical');persist();}
$('layout').onclick=()=>layout(!$('workspace').classList.contains('stacked'));
let split=48;function resize(value){split=Math.max(30,Math.min(70,value));$('workspace').style.setProperty('--split',`${split}%`);$('divider').setAttribute('aria-valuenow',String(Math.round(split)));}
$('divider').onpointerdown=event=>{event.preventDefault();const divider=$('divider');divider.setPointerCapture(event.pointerId);divider.onpointermove=e=>{const rect=$('workspace').getBoundingClientRect();resize($('workspace').classList.contains('stacked')?(e.clientY-rect.top)/rect.height*100:(e.clientX-rect.left)/rect.width*100);};divider.onpointerup=()=>{divider.onpointermove=null;divider.onpointerup=null;};};
$('divider').onkeydown=event=>{if(['ArrowLeft','ArrowUp','ArrowRight','ArrowDown'].includes(event.key)){event.preventDefault();resize(split+(['ArrowLeft','ArrowUp'].includes(event.key)?-2:2));}};
$('reset').onclick=()=>{if(!confirm('Reset the sample app and clear its saved runs? Your downloaded model will stay cached.'))return;uiEpoch++;loading=false;discardPreview();controller.restart();accepted=structuredClone(INITIAL_BUNDLE);acceptedStorage={};acceptedGeneration++;history=[];tab='plan';$('request').value=DEFAULT_REQUEST;if(mode()==='recorded')$('mode').value='guided';setModeNote();render();persist();notice('Sample app reset. '+(mode()==='live'&&!model.ready?'Click Load Gemma to resume from the browser cache.':'Enter a request to begin.'));};
const restoreEpoch=uiEpoch;
try{const stored=await load();if(restoreEpoch!==uiEpoch)throw Error('Saved workspace loading was cancelled.');
 if(stored?.schema===3){accepted=validateBundle(stored.accepted);acceptedStorage=stored.acceptedStorage||{};legacy=stored.legacy||null;history=(stored.history||[]).filter(successfulLiveRun).slice(-5);
  if(stored.task?.kind==='workspace'){controller.task=stored.task;$('request').value=stored.task.request||DEFAULT_REQUEST;$('mode').value=stored.task.mode;setModeNote();if(stored.task.status==='Active'){controller.task.status='Blocked';controller.task.phase='Interrupted by reload';}else if(['Review','Done'].includes(stored.task.status)){loading=true;render();await controller.verify(stored.task);if(restoreEpoch!==uiEpoch)throw Error('Saved workspace verification was cancelled.');loading=false;if(stored.task.status==='Done'&&canonicalBundle(accepted)!==canonicalBundle(stored.task.candidate))throw Error('Saved accepted app does not match its completed change.');}}
  if(stored.layout)layout(true);
 }else if(stored){legacy=stored;notice('Your previous demo workspace is preserved for export below. This new full-app workspace starts with the sample app; previous checks cannot approve it.');persist();}
}catch(error){if(restoreEpoch===uiEpoch){loading=false;if(controller.task)controller.block(error);notice(`Saved workspace needs attention: ${error.message}. Choose Start over to try a new request, or export it before resetting.`,true);}}
render();
