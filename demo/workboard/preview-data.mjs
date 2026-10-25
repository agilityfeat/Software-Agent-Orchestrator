/** Fictional, frozen data for the official read-only workboard preview.
 * This adapter is explicit: it never contacts an API, opens a stream, or changes data.
 */
export const snapshotAt='2026-01-15T14:30:00.000Z';
const capabilities={read:true,write:false,run:false,stop:false,accept:false,planning:false,projects:false,ready_to_done:false};
const agents=[
  {id:'sample-main',label:'Main',role:'main',enabled:true,model:'Sample planning agent',instructions:'Break a goal into small, independently verifiable tasks. Ask for review before adding work.'},
  {id:'sample-coder',label:'Coder',role:'coder',enabled:true,model:'Sample implementation agent',instructions:'Implement one bounded task, run its checks and retain the exact result.'},
  {id:'sample-qa',label:'QA',role:'qa',enabled:true,model:'Sample independent reviewer',instructions:'Review the tested revision independently. Report evidence and actionable findings.'},
];
const defaults={main:'sample-main',coder:'sample-coder',qa:'sample-qa'};
const repository='sample-org/harbor-app';
function task(number,title,status,extra={}){
  return {id:`sample-${String(number).padStart(3,'0')}`,revision:1,title,status,task_type:'improvement',priority:3,project:'Harbor sample app',project_id:'sample-project-harbor',repository,owner:'Coder',workflow:'engineering',qa_policy:'required',merge_policy:'manual',automation_policy:'manual',labels:['sample','usability'],dependencies:[],context:['Fictional example for the public workboard preview. No live repository or runtime is connected.'],objective:`${title}. Keep the change small and reviewable.`,acceptance_criteria:['The stated outcome can be independently verified.','Existing behavior remains available.','Coder records checks and QA reviews the exact revision.'],created_at:'2026-01-15T09:00:00.000Z',updated_at:snapshotAt,...extra};
}
const tasks=[
  task(1,'Explore clearer empty states','backlog',{owner:'Main',task_type:'research',priority:4,objective:'Compare two plain-language empty-state approaches before implementation. This is a fictional planning task.'}),
  task(2,'Add a keyboard shortcut guide','backlog',{priority:3}),
  task(3,'Clarify the receipt action','ready',{priority:2,automation_policy:'ready_to_done',automation_revision:1,ready_at:'2026-01-15T10:30:00.000Z'}),
  task(4,'Polish summary-card spacing','ready',{priority:3,dependencies:['sample-006'],dependency_blockers:[{id:'sample-006',title:'Review accessible color pairs'}],automation_policy:'ready_to_done',automation_revision:1,ready_at:'2026-01-15T10:00:00.000Z'}),
  task(5,'Keep filters visible on small screens','active',{priority:2}),
  task(6,'Review accessible color pairs','active',{project:'Field notes sample',project_id:'sample-project-notes',repository:'sample-org/field-notes',owner:'QA',task_type:'bug',priority:2,criticality:'medium'}),
  task(7,'Make export labels easier to understand','review',{owner:'Main',workflow:'manual',repository:null,project_id:null,objective:'Review the sample wording and confirm that people can distinguish export formats.',acceptance_criteria:['Labels describe the file contents without jargon.']}),
  task(8,'Preserve paid expense status','done',{completed_at:'2026-01-15T12:00:00.000Z',labels:['sample','policy']}),
  task(9,'Group navigation landmarks','done',{completed_at:'2026-01-15T11:15:00.000Z'}),
  task(10,'Replace the navigation pattern','cancelled',{cancelled_at:'2026-01-15T11:30:00.000Z',owner:'Main',context:['Sample decision: improve the existing navigation rather than replacing it.']}),
  task(11,'Wait for a design decision','ready',{project:'Field notes sample',project_id:'sample-project-notes',repository:'sample-org/field-notes',owner:'Main',blocked_reason:'Sample blocker: choose the final label before implementation.',priority:1}),
  task(12,'Archive the old welcome copy','done',{archived_at:'2026-01-15T12:15:00.000Z',completed_at:'2026-01-15T11:45:00.000Z'}),
];
function run(number,taskNumber,status,stage,extra={}){return {id:`sample-run-${number}`,task_id:`sample-${String(taskNumber).padStart(3,'0')}`,task_revision:1,status,stage,created_at:'2026-01-15T13:00:00.000Z',updated_at:'2026-01-15T14:20:00.000Z',agents:{coder:agents[1],qa:agents[2]},...extra};}
const runs=[
  run(1,5,'running','coder_running',{evidence:{coder_status:'running',qa_status:'not_started'}}),
  run(2,6,'running','qa_running',{evidence:{coder_status:'complete',qa_status:'running'}}),
  run(3,8,'succeeded','delivery_complete',{evidence:{coder_status:'complete',qa_status:'passed'}}),
  run(4,9,'succeeded','delivery_complete',{evidence:{coder_status:'complete',qa_status:'passed'}}),
];
const events=[
  {id:'sample-event-1',type:'task.created',task_id:'sample-003',at:'2026-01-15T10:30:00.000Z',summary:'Sample task reviewed and queued.'},
  {id:'sample-event-2',type:'run.updated',task_id:'sample-005',run_id:'sample-run-1',stage:'coder_running',at:'2026-01-15T14:00:00.000Z',summary:'Sample Coder implementation started.'},
  {id:'sample-event-3',type:'run.updated',task_id:'sample-006',run_id:'sample-run-2',stage:'qa_running',at:'2026-01-15T14:20:00.000Z',summary:'Sample QA review started after Coder checks.'},
  {id:'sample-event-4',type:'task.updated',task_id:'sample-007',at:'2026-01-15T14:25:00.000Z',summary:'Sample result is ready for a decision.'},
];
const execution_activity={state:'available',tasks:{
  'sample-005':{coder:{status:'running',at:'2026-01-15T14:00:00.000Z',basis:'sample_snapshot'}},
  'sample-006':{coder:{status:'complete',at:'2026-01-15T13:45:00.000Z',basis:'worker_completion'},qa:{status:'running',at:'2026-01-15T14:20:00.000Z',basis:'sample_snapshot'}},
  'sample-008':{coder:{status:'complete',at:'2026-01-15T11:30:00.000Z',basis:'worker_completion'},qa:{status:'complete',at:'2026-01-15T11:50:00.000Z',basis:'worker_completion'}},
}};
const snapshot={capabilities,watermark:4,tasks,runs,events,execution_activity,
  execution:{approved:false,paused:true},
  coding_agents:{agents:[agents[1]],default_coder:'sample-coder'},
  system:{editable:false,agents,defaults},
  projects:[{id:'sample-project-harbor',revision:1,repository,default_branch:'main',runtime_status:'ready',lane_run_id:'sample-run-1',paused:false,validation_summary:'Sample checks: unit tests, build and independent QA.'},{id:'sample-project-notes',revision:1,repository:'sample-org/field-notes',default_branch:'main',runtime_status:'ready',lane_run_id:'sample-run-2',paused:false,validation_summary:'Sample checks: accessibility review and independent QA.'}],
  plans:[{id:'sample-plan-1',revision:1,status:'accepted',goal:'Make the sample app easier to use',project:'Harbor sample app',repository,summary:'A fictional Main plan split the goal into a receipt action, spacing cleanup and accessibility review. These sample tasks are already on the board.',context:'Demonstrate small tasks, explicit dependencies and independent QA.',accepted_task_ids:['sample-003','sample-004','sample-006'],agent:agents[0]}],
};
const session={authenticated:true,login_method:'tailnet',identity:'Sample workspace · read-only',capabilities};
/** GET allowlist only. A rejected path/method never falls through to a real API. */
export async function previewRequest(path,{method='GET',body}={}){
  if(method!=='GET'||body!==undefined)throw Object.assign(new Error('This sample workspace is read-only.'),{code:'forbidden',status:403});
  if(path==='/session')return structuredClone(session);
  if(path==='/snapshot')return structuredClone(snapshot);
  const match=typeof path==='string'&&/^\/tasks\/(sample-[0-9]{3})$/.exec(path);
  const found=match&&tasks.find(task=>task.id===match[1]);
  if(found)return structuredClone({task:found,runs:runs.filter(run=>run.task_id===found.id),events:events.filter(event=>event.task_id===found.id),execution_activity});
  throw Object.assign(new Error('This sample item is unavailable.'),{code:'not_found',status:404});
}
