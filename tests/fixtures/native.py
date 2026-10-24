#!/usr/bin/python3
# Synthetic CLI protocol only: never a provider/network invocation.
import os,sys,json,subprocess,time,uuid,pathlib,shlex
mode=MODE_VALUE
workspace=pathlib.Path(sys.argv[sys.argv.index('--cd')+1])
cache=pathlib.Path(os.environ['HOME']).parent
assert 'GH_TOKEN' not in os.environ and 'GITHUB_TOKEN' not in os.environ
assert '--ignore-user-config' in sys.argv and '--ignore-rules' in sys.argv
if mode.startswith('research'):
 assert sys.argv[1:3]==['--search','exec']
 assert '--skip-git-repo-check' in sys.argv
 assert 'permissions.workboard-native.extends=":read-only"' in sys.argv
 assert 'permissions.workboard-native.network.enabled=false' in sys.argv
 assert 'shell_environment_policy.inherit="none"' in sys.argv
 assert 'GH_TOKEN' not in os.environ and 'GITHUB_TOKEN' not in os.environ
 assert list(workspace.iterdir())==[]
 prompt=sys.stdin.read();assert 'No repository has been inspected' in prompt
 qa='"passed"' in pathlib.Path(sys.argv[sys.argv.index('--output-schema')+1]).read_text()
 assert ('Frozen research QA instruction' if qa else 'Frozen research coder instruction') in prompt
 def emit(value):print(json.dumps(value),flush=True)
 emit({'type':'thread.started','thread_id':str(uuid.uuid4())})
 if mode=='research_tool':emit({'type':'item.started','item':{'id':'bad','type':'command_execution'}})
 if mode!='research_no_search':emit({'type':'item.completed','item':{'id':'search_1','type':'web_search','query':'synthetic question','action':{'type':'search','query':'synthetic question'}}})
 report={'markdown':'# Synthetic research report\nThe synthetic result has bounded conclusions and explicit uncertainty.','sources':[{'title':'Synthetic primary source','url':'javascript:invalid' if mode=='research_bad_url' else 'https://example.org/research'}]}
 value={'passed':True,'findings':[]} if qa else report
 emit({'type':'item.completed','item':{'id':'message','type':'agent_message','text':json.dumps(value)}})
 if mode!='research_incomplete':emit({'type':'turn.completed'})
 sys.exit(0)
if not mode.startswith('plan'):
 assert 'permissions.workboard-native.extends=":workspace"' in sys.argv
 assert json.loads(pathlib.Path(sys.argv[sys.argv.index('--output-schema')+1]).read_text())['properties']['checks']['items']['properties']['command_id']=={'type':'null'}
assert 'default_permissions="workboard-native"' in sys.argv and '--sandbox' not in sys.argv
assert len([a for a in sys.argv if a.startswith('permissions.workboard-native.filesystem={')])==1
assert len([a for a in sys.argv if a.startswith('projects={')])==1
assert not any(a.startswith('permissions.workboard-native.filesystem.') or a.startswith('projects.') for a in sys.argv)
if mode.startswith('plan'):
 assert '--search' not in sys.argv
 assert '--skip-git-repo-check' in sys.argv
 assert 'permissions.workboard-native.extends=":read-only"' in sys.argv
 assert 'permissions.workboard-native.network.enabled=false' in sys.argv
 assert 'shell_environment_policy.inherit="none"' in sys.argv
 filesystem=next(a for a in sys.argv if a.startswith('permissions.workboard-native.filesystem='))
 assert json.dumps(str(workspace))+'="read"' in filesystem
 assert '/unneeded-project-sdk' not in filesystem
 assert list(workspace.iterdir())==[]
 prompt=sys.stdin.read()
 assert 'source HAS NOT BEEN INSPECTED' in prompt and 'Frozen synthetic Main instructions' in prompt
 if mode=='plan_sleep':
  child=subprocess.Popen(['/bin/sleep','60']);(cache/'child.pid').write_text(str(child.pid));time.sleep(60)
 def emit(x):print(json.dumps(x),flush=True)
 emit({'type':'thread.started','thread_id':'synthetic-plan-session'})
 if mode=='plan_tool':emit({'type':'item.completed','item':{'id':'tool','type':'web_search'}})
 plan={'summary':'Synthetic goal-based plan; source not inspected.','tasks':[{'title':'Synthetic proposal','objective':'Bounded implementation','acceptance_criteria':['Synthetic acceptance'],'priority':3,'task_type':'feature','labels':[],'depends_on':[]}]}
 emit({'type':'item.completed','item':{'type':'agent_message','text':'invalid' if mode=='plan_malformed' else json.dumps(plan)}})
 if mode!='plan_incomplete':emit({'type':'turn.completed'})
 sys.exit(0)
if mode in ('sleep','broken_pipe'):
 child=subprocess.Popen(['/bin/sleep','60'])
 (cache/'child.pid').write_text(str(child.pid))
 if mode=='broken_pipe':
  os.close(0)
  time.sleep(.3)
  sys.exit(8)
 time.sleep(60)
 sys.exit(1)
prompt=sys.stdin.read()
if mode=='model_override':
 assert sys.argv[sys.argv.index('--model')+1]=='registry-model'
 assert 'Frozen registry operator instruction' in prompt
if mode in ('refresh','foreign_account','auth_drift'):
 authpath=pathlib.Path(os.environ['CODEX_HOME'])/'auth.json'
 auth=json.loads(authpath.read_text());auth['tokens']['access_token']='synthetic-refreshed-only'
 if mode=='foreign_account':auth['tokens']['account_id']='different-account'
 authpath.write_text(json.dumps(auth))
 if mode=='auth_drift':
  source=pathlib.Path(os.environ['CODEX_HOME']).parents[3]/'auth/auth.json'
  canonical=json.loads(source.read_text());canonical['tokens']['access_token']='synthetic-newer-only';source.write_text(json.dumps(canonical))
if mode=='exit':sys.exit(7)
qa=workspace.name=='qa-workspace'
if not qa or mode=='qa_mutate':(workspace/'code.py').write_text('value = 2\n')
commands=[('/usr/bin/python3 -c "assert 1 + 1 == 2"','check'),('/usr/bin/git diff --check','self_review')]
if mode=='wrapped':commands[0]=('/usr/bin/python3 -c \'assert "quoted" == "quoted"\'','check')
checks=[]
def emit(x):print(json.dumps(x),flush=True)
emit({'type':'thread.started','thread_id':str(uuid.uuid4())})
for i,(cmd,kind) in enumerate(commands):
 if mode=='echo' and i==0:cmd='echo fake test'
 r=subprocess.run(['/bin/sh','-c',cmd],cwd=workspace,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
 wrapped=shlex.join(['/bin/zsh','-c',cmd]) if mode=='wrapped' else '/bin/sh -c '+repr(cmd)
 emit({'type':'item.completed','item':{'type':'command_execution','id':'item_'+str(i),'command':wrapped,'status':'completed','exit_code':r.returncode}})
 checks.append({'command_id':'call_synthetic' if mode=='id_mismatch' else None,'command':cmd if mode!='mismatch' else 'invented test','category':kind,'purpose':'Synthetic actual validation','result':'passed'})
verdict={'completed':True,'passed':True,'summary':'Synthetic checked','findings':[],'out_of_scope_bugs':[],'setup_status':'blocked' if mode=='blocked' else 'ready','checks':checks,'limitations':[]}
emit({'type':'item.completed','item':{'type':'agent_message','text':json.dumps(verdict)}})
emit({'type':'turn.completed'})
