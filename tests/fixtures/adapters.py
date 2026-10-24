#!/usr/bin/python3
"""Synthetic adapter programs. No network, Docker daemon, provider or GitHub calls."""
import json,os,pathlib,subprocess,sys,hashlib
root=pathlib.Path(__file__).parent
mode=json.loads((root/'mode.json').read_text());statefile=root/'state.json'
s=json.loads(statefile.read_text()) if statefile.exists() else {'containers':[],'pr':None}
a=sys.argv[1:];tool=pathlib.Path(__file__).name
with (root/'audit.jsonl').open('a') as log:log.write(json.dumps({'tool':tool,'args':a,'env_names':sorted(os.environ),'secret_in_args':any('synthetic-token' in v for v in a)})+'\n')
def save():statefile.write_text(json.dumps(s))
def realgit(*args,input=None):return subprocess.check_output(['/usr/bin/git',*args],input=input,stderr=subprocess.DEVNULL).decode().strip()
def wgit(*args):return realgit('-C',s['workspace'],*args)
def pr():
 return {'number':1,'html_url':'https://github.com/fixture/private/pull/1','head':{'repo':{'full_name':'fixture/private'},'ref':'workboard/'+s['run'],'sha':s['head']},'base':{'repo':{'full_name':'fixture/private'},'ref':'main','sha':s['base']},'merged':s.get('merged',False),'merge_commit_sha':s.get('merge'),'state':'closed' if s.get('merged') else 'open','mergeable':not mode.get('merge_conflict'),'draft':False}
if tool=='docker':
 a=a[2:]
 if a[:2]==['network','inspect']:print(json.dumps([{'Id':'1'*64,'Internal':True}]))
 elif a[:1]==['inspect']:
  if a[1]=='2'*64:print(json.dumps([{'Id':'2'*64,'Image':'sha256:'+'3'*64,'State':{'Running':True},'Config':{'Env':['CODEX_EGRESS_PROXY_ENABLED=true','CODEX_EGRESS_ALLOWED_CONNECT_HOSTS=chatgpt.com','CODEX_EGRESS_POLICY_SHA256='+'4'*64]},'NetworkSettings':{'Networks':{'isolated':{'NetworkID':'1'*64,'IPAddress':'192.0.2.2'}}},'Mounts':[]}]))
  else:print(json.dumps([s['container']]))
 elif a[:1]==['ps']:print('\n'.join(s['containers']))
 elif a[:1]==['create']:
  name=a[a.index('--name')+1];labels={}
  for i,v in enumerate(a):
   if v=='--label':k,val=a[i+1].split('=',1);labels[k]=val
  s['run']=labels['io.workboard.run'];s['container']={'Id':'7'*64,'Name':'/'+name,'Image':'sha256:'+'5'*64,'State':{'Running':True},'Config':{'Labels':labels}};s['containers']=['7'*64];save();print('7'*64)
 elif a[:1]==['start']:print(a[1])
 elif a[:1]==['exec']:
  data=sys.stdin.buffer.read()
  if a[-1]=='--stop-worker':print(json.dumps({'stopped':True}))
  elif a[-1]=='--export-auth':print(json.dumps({'fixture':'rotated-synthetic-auth'}) if mode.get('auth_rotate') else (root/'auth'/'auth.json').read_text(),end='')
  elif a[-1]=='/usr/local/bin/workboard-contained':
   req=json.loads(data);s['contained_input']=req;role=req['role'];s['role']=role;s['session']=role+'-session-123456';s['validation_profile']=req.get('validation_profile');s['validation_head']=req['head_sha'];save()
   if mode.get('contained_failure'):
    print(json.dumps({'schema_version':1,'stage':'codex','category':'proxy_connection','exit_code':17,'signal':None,'http_status':407,'session_started':False}),file=sys.stderr)
    print('synthetic-private-provider-output');sys.exit(17)
   if role=='plan':
    plan={'summary':'SYNTHETIC goal-based plan; no repository inspection.','tasks':[{'title':'Proposed bounded outcome','objective':'Implement the requested bounded goal.','acceptance_criteria':['A deterministic acceptance test demonstrates success.'],'priority':3}]}
    if mode.get('bad_plan'):plan['tasks'][0]['repository']='foreign/authority'
    print(json.dumps({'schema_version':1,'completed':not mode.get('incomplete_plan'),'session_id':s['session'],'plan':plan}));sys.exit(0)
   result={'schema_version':1,'completed':not mode.get('incomplete_qa') if role=='qa' else True,'passed':not mode.get('qa_false') if role=='qa' else True,'session_id':s['session']}
   if role=='qa' and mode.get('qa_empty_findings'):result['findings']=[]
   if role=='qa' and mode.get('qa_findings'):result['findings']=[{'path':'f.txt','line':1,'summary':'The implementation violates the stated acceptance case.'}]
   if role=='qa' and mode.get('qa_out_of_scope'):
    result['out_of_scope_bugs']=[{'path':'legacy.txt','line':1,'summary':'Independent legacy boundary returns the wrong result.','criticality':'high','reproduction':'Call the existing legacy boundary with the documented value.','expected':'It returns the documented valid response.','actual':'It returns the same pre-existing invalid response.','base_sha':s['base'],'base_reproduction':'The exact base produces the same invalid response.','unrelated_reason':'This legacy path is independent and neither exposed nor worsened by this task.'}]
    if mode.get('out_scope_wrong_base'):result['out_of_scope_bugs'][0]['base_sha']='0'*40
    if mode.get('out_scope_injection'):result['out_of_scope_bugs'][0][mode['out_scope_injection']]='ready'
   if role=='qa' and mode.get('qa_result_schema_invalid'):result['private']='synthetic-private-provider-output'
   print(json.dumps(result))
  elif a[-1]=='--export-evidence':
   if mode.get('validation_symlink'):
    print('contained_export_invalid',file=sys.stderr);sys.exit(1)
   evidence={'schema_version':1,'session_id':s['session'],'turn_completed':True,'successful_tests':1,'failed_tests':0,'diff_reviews':1,'commands':[{'kind':'test','command':'python3 -m unittest -v','exit_code':0,'completed':True}],'unittest':{'run':1,'executed':1,'skipped':0,'failures':0,'errors':0}}
   profile=s.get('validation_profile')
   if profile and profile['id'] in ('node-npm-v1','node-unit-syntax-v1') and not mode.get('legacy_node_evidence'):
    def tree(after=False):
     files={}
     for file in pathlib.Path(s['workspace']).rglob('*'):
      rel=file.relative_to(s['workspace'])
      if rel.parts[0] in ('.git','node_modules') or not file.is_file():continue
      data=b'after\n' if after and str(rel)=='f.txt' else file.read_bytes()
      files[str(rel)]=[hashlib.sha256(data).hexdigest(),bool(file.stat().st_mode&0o111)]
     return hashlib.sha256(json.dumps(files,sort_keys=True,separators=(',',':')).encode()).hexdigest()
    evidence={'schema_version':2,'session_id':s['session'],'turn_completed':True,'successful_tests':1,'failed_tests':0,'diff_reviews':1,'commands':[],'profile':profile,'validation':{'install_exit_code':0,'dependency_import_exit_code':0,'dependency_import_count':4,'syntax_check_exit_code':0,'test_exit_code':0,'tests':{'tests':1,'pass':1,'fail':0,'cancelled':0,'skipped':0,'todo':0},'source_before_sha256':tree(),'source_after_sha256':tree(s['role']=='coder'),'qa_source_unchanged':True,'head_sha':s['validation_head']}}
    if profile['id']=='node-unit-syntax-v1':
     evidence['schema_version']=5
     for k in ['install_exit_code','dependency_import_exit_code','dependency_import_count']:evidence['validation'].pop(k)
     evidence['validation']['syntax_check_count']=1
    if mode.get('preflight_evidence'):evidence['mode']='validation_preflight'
    if mode.get('wrong_profile_cache'):evidence['profile']['cache_sha256']='0'*64
    if mode.get('wrong_profile_head'):evidence['validation']['head_sha']='0'*40
    if mode.get('wrong_profile_tree'):evidence['validation']['source_after_sha256']='0'*64
    if mode.get('zero_node_tests'):evidence['validation']['tests']={'tests':1,'pass':0,'fail':0,'cancelled':0,'skipped':1,'todo':0};evidence['successful_tests']=0;evidence['failed_tests']=1
   if mode.get('failed_validation') and s['role']=='qa':evidence.update(successful_tests=0,failed_tests=1);evidence['unittest'].update(failures=1);evidence['commands'][0]['exit_code']=1
   if mode.get('signalled_validation') and s['role']=='qa':evidence['commands'][0]['exit_code']=-1
   if s['role']=='qa':
    if mode.get('qa_evidence_session_mismatch'):evidence['session_id']='synthetic-private-session-content'
    if mode.get('qa_turn_incomplete'):evidence['turn_completed']=False
    if mode.get('qa_success_missing'):evidence['successful_tests']=0
    if mode.get('qa_unsafe_evidence'):evidence['private']='synthetic-private-auth-or-message'
   if mode.get('unsafe_evidence'):evidence['private']='synthetic-private-auth-or-message'
   print(json.dumps(evidence))
  elif a[-1]=='--export-patch':print('diff --git a/f.txt b/f.txt\nindex 90be1f3..2f84a65 100644\n--- a/f.txt\n+++ b/f.txt\n@@ -1 +1 @@\n-before\n+after\n',end='')
 elif a[:1]==['rm']:s['containers']=[];save();print(a[-1])
 else:sys.exit(2)
elif tool=='git':
 data=sys.stdin.buffer.read()
 if 'clone' in a:
  # Exercise the actual publisher askpass dispatch without sending credentials to a server.
  ask=os.environ['GIT_ASKPASS'];repo=os.environ['WORKBOARD_GIT_REPOSITORY']
  assert subprocess.check_output([ask,"Username for 'https://github.com/"+repo+".git':"]).decode().strip()=='x-access-token'
  assert subprocess.check_output([ask,"Password for 'https://x-access-token@github.com/"+repo+".git':"]).decode().strip()=='synthetic-token'
  workspace=pathlib.Path(a[-1]);workspace.mkdir();s['workspace']=str(workspace)
  realgit('-C',str(workspace),'init','--initial-branch=main');(workspace/'f.txt').write_text('before\n')
  if mode.get('node_profile'):
   (workspace/'scripts').mkdir();(workspace/'package.json').write_text('{"scripts":{"check":"node scripts/check.js","test":"node --test"}}');(workspace/'package-lock.json').write_text('{"lockfileVersion":3}');(workspace/'scripts'/'check.js').write_text('process.exit(0);\n')
  if mode.get('unit_profile'):
   (workspace/'test').mkdir();(workspace/'package.json').write_text('{"type":"module","scripts":{"test":"node --test"},"dependencies":{"express":"^5.1.0"}}');(workspace/'test'/'logic.test.mjs').write_text("import test from 'node:test';test('synthetic',()=>{});\n")
  wgit('add','-A');wgit('-c','user.name=Synthetic','-c','user.email=synthetic@example.invalid','commit','-m','base');wgit('remote','add','origin','https://github.com/fixture/private.git')
  s['base']=wgit('rev-parse','HEAD');wgit('update-ref','refs/remotes/origin/main',s['base']);save()
 elif 'push' in a:s['head']=a[-1].split(':')[0];save()
 elif 'fetch' in a:pass
 else:
  result=subprocess.run(['/usr/bin/git',*a],input=data,stdout=subprocess.PIPE,stderr=subprocess.DEVNULL);sys.stdout.buffer.write(result.stdout);sys.exit(result.returncode)
elif tool=='gh':
 body=json.load(sys.stdin) if '--input' in a else None
 path='graphql' if 'graphql' in a else next(v for v in a if v.startswith('repos/'))
 if path=='graphql':print(json.dumps({'data':{'repository':{'pullRequest':{'reviewThreads':{'nodes':[{'isResolved':False}] if mode.get('unresolved_thread') else [],'pageInfo':{'hasNextPage':False}}}}}}))
 elif path.endswith('/reviews?per_page=100'):print(json.dumps([{'id':1,'state':'CHANGES_REQUESTED','user':{'login':'reviewer'}}] if mode.get('review_veto') else []))
 elif path=='repos/fixture/private':print(json.dumps({'id':99,'full_name':'fixture/private' if not mode.get('wrong_repository') else 'other/private','private':True,'permissions':{'push':True},'default_branch':'main'}))
 elif '/pulls?' in path:print(json.dumps([pr()] if s.get('pr') else []))
 elif path.endswith('/pulls'):
  s['pr']=True;s['pr_body']=body['body'];s['run']=body['head'].split('/')[-1];save();print(json.dumps(pr()))
 elif path.endswith('/merge'):
  if mode.get('uncertain_merge'):sys.exit(1)
  s['merged']=True;s['merge']=wgit('-c','user.name=Synthetic','-c','user.email=synthetic@example.invalid','commit-tree',wgit('rev-parse','HEAD^{tree}'),'-p',s['base'],'-p',s['head'],'-m','synthetic merge');wgit('update-ref','refs/remotes/origin/main',s['merge']);save();print(json.dumps({'merged':True}))
 elif '/pulls/' in path:print(json.dumps(pr()))
 elif '/git/ref/heads/' in path:print(json.dumps({'object':{'sha':s['head'] if path.endswith(s.get('run','x')) else s['base']}}))
 else:sys.exit(2)
else:sys.exit(2)
