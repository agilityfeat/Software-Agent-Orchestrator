#!/usr/bin/python3
"""SYNTHETIC ORCHESTRATION FIXTURE ONLY: never real worker/GitHub evidence."""
import hashlib, json, os, pathlib, subprocess, sys, time
root=pathlib.Path(__file__).resolve().parent
request=json.load(sys.stdin)
mode=json.loads((root/'mode.json').read_text())
mode={**mode,**mode.get('mode_by_repository',{}).get(request['repository'],{})}
action=request['action']; workspace=pathlib.Path(request['workspace'])
prior=[json.loads(v) for v in (root/'audit.jsonl').read_text().splitlines()] if (root/'audit.jsonl').exists() else []
cycle=sum(v['action']==action and v['run_id']==request['run_id'] for v in prior)
with (root/'audit.jsonl').open('a') as log:
 log.write(json.dumps({'action':action,'action_id':request['action_id'],'run_id':request['run_id'],'merge_policy':request.get('merge_policy'),'at':time.time(),'synthetic':True,'cycle':cycle,'head_sha':request.get('head_sha'),'repair_context':request.get('repair_context'),'deadline_ms':request['deadline_ms'],'task_context':request['task_snapshot'].get('context'),'agent_config':(request.get('adapter_policy') or {}).get('agent_config')})+'\n')
response={key:request[key] for key in ['schema_version','action_id','run_id','action','repository']}
response.update(outcome='ok',cleanup_complete=True)
def git(*args):
 return subprocess.check_output(['/usr/bin/git','-C',str(workspace),*args],stderr=subprocess.DEVNULL,text=True).strip()
def commit(message):
 return git('-c','user.name=QA Fixture','-c','user.email=synthetic@example.invalid','commit','--allow-empty','-m',message)
if action=='plan':
 if mode.get('sleep_plan'):time.sleep(mode['sleep_plan'])
 plan={'summary':'SYNTHETIC goal-based proposals; repository source was not inspected.','tasks':[{'title':'Synthetic first outcome','objective':'Implement the requested first bounded outcome.','acceptance_criteria':['A deterministic regression demonstrates the requested behavior.'],'priority':3},{'title':'Synthetic second outcome','objective':'Implement the next independently useful outcome.','acceptance_criteria':['The second outcome has independent acceptance evidence.'],'priority':2}]}
 if mode.get('plan_fields'):
  plan['tasks'][0].update(task_type='research',labels=['Discovery'],depends_on=[])
  plan['tasks'][1].update(task_type='feature',labels=['UI'],depends_on=[0])
 if not request['repository']:
  for task in plan['tasks']:task.update(task_type='research',labels=[],depends_on=[])
 if mode.get('bad_plan'):plan['tasks'][0]['repository']='foreign/authority'
 response.update(role='plan',session_id='plan-'+request['run_id'],plan=plan)
 if mode.get('unclean_plan'):response['cleanup_complete']=False
elif action=='prepare':
 workspace.mkdir(mode=0o700,parents=True,exist_ok=True); workspace.chmod(0o700)
 git('init','--initial-branch=main');git('remote','add','origin','https://github.com/'+request['repository']+'.git')
 commit('Synthetic base');base=git('rev-parse','HEAD');git('update-ref','refs/remotes/origin/main',base)
 response.update(workspace=str(workspace),base_sha=base,head_sha=base,base_ref='refs/remotes/origin/main')
elif action=='coder':
 if mode.get('driver_failure'):
  if mode.get('typed_failure'):
   failure={'schema_version':1,'stage':'codex','category':'authentication','exit_code':7,'signal':None,'http_status':401,'session_started':True}
   if mode.get('unsafe_failure'):failure['raw']='synthetic-private-auth-and-prompt'
   if mode.get('oversize_failure'):failure['raw']='synthetic-private-'+('x'*2048)
   print(json.dumps(failure),file=sys.stderr)
  else:print('synthetic-private-auth-and-prompt',file=sys.stderr)
  if mode.get('stdout_failure'):print(json.dumps({'schema_version':1,'stage':'codex','category':'authentication','exit_code':7,'signal':None,'http_status':401,'session_started':True}))
  else:print('synthetic-private-stdout')
  sys.exit(7)
 if mode.get('dirty_before_sleep'):(workspace/'unfinished.txt').write_text('incomplete synthetic worker output\n')
 if mode.get('sleep_coder'):time.sleep(mode['sleep_coder'])
 if cycle and mode.get('non_descendant_repair'):git('reset','--hard',request['base_sha'])
 (workspace/'synthetic.txt').write_text('Synthetic orchestration fixture only '+str(cycle)+'\n');git('add','synthetic.txt');commit('Synthetic implementation '+str(cycle))
 head=git('rev-parse','HEAD')
 if mode.get('malicious_fsmonitor'):
  monitor=root/'fsmonitor.sh';monitor.write_text('#!/bin/sh\nprintf triggered > '+str(root/'fsmonitor-ran.txt')+'\n');monitor.chmod(0o700);git('config','core.fsmonitor',str(monitor))
 if mode.get('external_gitdir'):
  external=root/'external.git';(workspace/'.git').rename(external);(workspace/'.git').write_text('gitdir: '+str(external)+'\n')
 response.update(workspace=str(workspace),base_sha=request['base_sha'],head_sha=head,session_id='coder-'+request['run_id']+(('-'+str(cycle)) if cycle and not mode.get('reuse_coder_session') else ''),role='coder')
 if request['task_snapshot'].get('qa_policy')=='skip':response['passed']=not mode.get('coder_validation_failed')
elif action=='qa':
 if mode.get('sleep_qa'):time.sleep(mode['sleep_qa'])
 reviewed=request['head_sha'] if not mode.get('wrong_qa_sha') else '0'*40
 response.update(head_sha=request['head_sha'],reviewed_sha=reviewed,passed=not (mode.get('qa_failed') or mode.get('qa_verdict_false') or cycle<mode.get('qa_rejections',0)),session_id=('coder-' if mode.get('same_qa_session') else 'qa-')+request['run_id']+(('-'+str(cycle)) if cycle and not mode.get('reuse_qa_session') else ''),role='qa')
 if mode.get('qa_failed'):response['outcome']='failed'
 if cycle<mode.get('qa_rejections',0):
  response.update(outcome='failed',qa_completed=True,findings=[] if mode.get('empty_feedback') else [{'path':'../outside' if mode.get('malformed_feedback') else 'synthetic.txt','line':1,'summary':'Fix the rejected implementation against the required acceptance case.'}])
 if mode.get('qa_out_of_scope'):
  response['qa_completed']=not mode.get('out_scope_incomplete')
  bugs=[]
  for i,criticality in enumerate(['critical','high','medium','low'][:mode.get('out_scope_count',1)]):
   bug={'path':'unrelated'+str(i)+'.txt','line':1,'summary':'Pre-existing independent synthetic defect '+str(i),'criticality':criticality if cycle==0 else 'low','reproduction':'Use the independent legacy boundary input.','expected':'The legacy boundary returns a valid result.','actual':'The legacy boundary returns the existing error.','base_sha':request['base_sha'],'base_reproduction':'The exact base reproduces the same legacy error.','unrelated_reason':'This independent legacy path is neither changed, exposed nor worsened by this task.'}
   if mode.get('out_scope_wrong_base'):bug['base_sha']='0'*40
   if mode.get('out_scope_injection'):bug[mode['out_scope_injection']]='foreign/repository' if mode['out_scope_injection']=='repository' else 'ready'
   if mode.get('out_scope_missing_evidence'):bug['base_reproduction']=''
   bugs.append(bug)
  response['out_of_scope_bugs']=bugs
elif action in ('research','research_qa'):
 if mode.get('sleep_research'):time.sleep(mode['sleep_research'])
 def report_digest(report):return hashlib.sha256(json.dumps(report,separators=(',',':'),ensure_ascii=False).encode()).hexdigest()
 response.update(role='coder' if action=='research' else 'qa',session_id=('research-'+request['run_id']+'-0' if mode.get('research_reused_session') else action+'-'+request['run_id']+'-'+str(cycle)),search_count=1,passed=True)
 if action=='research':
  report={'markdown':'# Synthetic research report\nA bounded synthetic conclusion with explicit uncertainty and source attribution.','sources':[{'title':'Synthetic primary source','url':'https://example.org/research'}]}
  if mode.get('research_invalid_url'):report['sources'][0]['url']='javascript:invalid'
  response.update(report=report,report_digest=report_digest(report))
 else:
  passed=cycle>=mode.get('research_rejections',0)
  response.update(report_digest=report_digest(request['research_report']) if not mode.get('research_wrong_digest') else '0'*64,passed=passed,research_findings=[] if passed else ['Clarify the source-backed conclusion and its uncertainty.'])
elif action=='deliver':
 merged=request['merge_policy']=='auto'
 response.update(head_sha=request['head_sha'],pull_request_url='https://github.com/'+request['repository']+'/pull/1',merged=merged)
 if merged:
  merge=git('-c','user.name=QA Fixture','-c','user.email=synthetic@example.invalid','commit-tree',git('rev-parse','HEAD^{tree}'),'-p',request['base_sha'],'-p',request['head_sha'],'-m','Synthetic merge only')
  response['merge_commit_sha']=merge;git('update-ref','refs/remotes/origin/main',merge)
elif action=='observe':
 response['observed_action_id']=request['observed_action_id']
 if mode.get('observed_merged'):
  merge=git('-c','user.name=QA Fixture','-c','user.email=synthetic@example.invalid','commit-tree',git('rev-parse','HEAD^{tree}'),'-p',request['base_sha'],'-p',request['head_sha'],'-m','Synthetic observed merge only')
  git('update-ref','refs/remotes/origin/main',merge)
  response.update(head_sha=('0'*40 if mode.get('observed_wrong_head') else request['head_sha']),merged=True,pull_request_url='https://github.com/'+request['repository']+'/pull/1',merge_commit_sha=merge)
 elif mode.get('observe_pending'):
  response.update(merged=False,pull_request_url='https://github.com/'+request['repository']+'/pull/1')
 else:response['merged']=False
elif action=='cancel':
 pass
else:response.update(outcome='unknown',cleanup_complete=False)
if mode.get('bad_receipt')==action or action in mode.get('bad_receipts',[]):print('invalid-json');sys.exit(0)
if mode.get('unknown_fields')==action:response['untrusted_extra']='must reject'
if mode.get('wrong_repository')==action:response['repository']='wrong/repository'
print(json.dumps(response,separators=(',',':')))
