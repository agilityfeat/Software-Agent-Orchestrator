"""Credential-free onboarding; all login material and tools are synthetic."""
import http.client
import json
import os
from pathlib import Path
import shutil
import signal
import socket
import stat
import subprocess
import sys
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parents[1]
CLI = ROOT / 'orchestrator'

class Bootstrap(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tools_temp = tempfile.TemporaryDirectory()
        cls.tools = Path(cls.tools_temp.name).resolve()
        source = cls.tools / 'tool.c'
        source.write_text(r'''
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>
int main(int argc,char**argv){
char*name=strrchr(argv[0],'/');name=name?name+1:argv[0];
if(!strcmp(name,"cargo")){
 if(getenv("GH_TOKEN")||getenv("OPENAI_API_KEY")||getenv("CODEX_HOME"))return 31;
 if(argc<3||strcmp(argv[1],"build"))return 32;
 mkdir(getenv("CARGO_TARGET_DIR"),0700);char dir[4096];snprintf(dir,sizeof(dir),"%s/release",getenv("CARGO_TARGET_DIR"));mkdir(dir,0700);
 const char*names[]={"workboard-rs","workboard-worker","workboard-publisher"};
 for(int i=0;i<3;i++){char path[8192];snprintf(path,sizeof(path),"%s/%s",dir,names[i]);FILE*in=fopen(argv[0],"rb"),*out=fopen(path,"wb");int ch;while((ch=fgetc(in))!=EOF)fputc(ch,out);fclose(in);fclose(out);chmod(path,0700);}return 0;
}
if(argc>1&&!strcmp(argv[1],"--version")){puts("codex-cli 0.153.4");return 0;}
if(argc>2&&!strcmp(argv[1],"auth")&&!strcmp(argv[2],"token")){puts("synthetic-gh-token");return 0;}
return 0;}
''')
        subprocess.run([shutil.which('cc'), str(source), '-o', str(cls.tools / 'native')], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        for name in ('codex', 'gh', 'rustup'):
            shutil.copyfile(cls.tools / 'native', cls.tools / name)
            (cls.tools / name).chmod(0o700)
        (cls.tools / 'cargo').symlink_to('rustup')
        cls.fake_bins = cls.tools / 'bin'
        cls.fake_bins.mkdir(mode=0o700)
        for name in ('workboard-rs', 'workboard-worker', 'workboard-publisher'):
            shutil.copyfile(cls.tools / 'native', cls.fake_bins / name)
            (cls.fake_bins / name).chmod(0o700)

    @classmethod
    def tearDownClass(cls):
        cls.tools_temp.cleanup()

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name).resolve()
        self.home = self.root / 'home'
        self.home.mkdir(mode=0o700)
        self.auth = self.home / '.codex'
        self.auth.mkdir(mode=0o700)
        (self.auth / 'auth.json').write_text(json.dumps({'tokens': {'account_id': 'synthetic-account', 'access_token': 'synthetic-private-not-real'}}))
        (self.auth / 'auth.json').chmod(0o600)
        self.state = self.root / 'app'
        self.env = {'PATH': str(self.tools) + ':/usr/bin:/bin', 'HOME': str(self.home)}

    def tearDown(self):
        self.temp.cleanup()

    def cli(self, *args, success=True, env=None):
        result = subprocess.run([sys.executable, str(CLI), *map(str,args), '--state-dir', str(self.state)], env=env or self.env, capture_output=True, text=True, timeout=240)
        self.assertEqual(result.returncode == 0, success, result.stdout + result.stderr)
        self.assertNotIn('synthetic-private-not-real', result.stdout + result.stderr)
        self.assertNotIn('synthetic-gh-token', result.stdout + result.stderr)
        return result

    def init(self, *args, binary_dir=None):
        return self.cli('init', '--codex-home', self.auth, '--codex', self.tools/'codex', '--gh', self.tools/'gh', '--git', shutil.which('git'), '--binary-dir', binary_dir or self.fake_bins, *args)

    def test_init_doctor_idempotence_and_private_key(self):
        first = self.init()
        key = (self.state/'secrets/operator.key').read_text()
        self.assertNotIn(key, first.stdout)
        self.assertEqual(stat.S_IMODE((self.state/'secrets/operator.key').stat().st_mode), 0o600)
        before = {p.relative_to(self.state):p.read_bytes() for p in self.state.rglob('*') if p.is_file()}
        self.init()
        self.assertEqual(before, {p.relative_to(self.state):p.read_bytes() for p in self.state.rglob('*') if p.is_file()})
        self.cli('doctor')
        result = self.cli('key', success=False)
        self.assertNotIn(key, result.stdout + result.stderr)
        setup = json.loads((self.state/'state/setup.json').read_text())
        self.assertTrue(setup['planning_enabled'])
        self.assertTrue(setup['runtime']['adapter_policy']['native_agent']['native_planning'])
        self.assertFalse((self.state/'state/workboard.sqlite').exists())

    def test_no_overwrite_changed_settings_or_pins(self):
        self.init()
        self.cli('init', '--codex-home', self.auth, '--codex', self.tools/'codex', '--gh', self.tools/'gh', '--git', shutil.which('git'), '--port', 4444, success=False)
        key = (self.state/'secrets/operator.key').read_bytes()
        (self.state/'bin/workboard-rs').write_bytes(b'changed')
        self.cli('doctor', success=False)
        self.assertEqual(key, (self.state/'secrets/operator.key').read_bytes())

    def test_symlinks_and_insecure_auth_are_rejected(self):
        (self.auth/'auth.json').chmod(0o644)
        self.cli('init', '--codex-home', self.auth, '--codex', self.tools/'codex', '--gh', self.tools/'gh', '--git', shutil.which('git'), '--binary-dir', self.fake_bins, success=False)
        self.assertFalse(self.state.exists())
        (self.auth/'auth.json').chmod(0o600)
        self.state.symlink_to(self.home, target_is_directory=True)
        self.cli('init', '--codex-home', self.auth, '--codex', self.tools/'codex', '--gh', self.tools/'gh', '--git', shutil.which('git'), success=False)

    def test_rustup_multicall_and_build_env_are_preserved(self):
        env=dict(self.env, GH_TOKEN='synthetic-not-forwarded', OPENAI_API_KEY='synthetic-not-forwarded', CODEX_HOME=str(self.auth))
        self.cli('init', '--cargo', self.tools/'cargo', '--codex-home', self.auth, '--codex', self.tools/'codex', '--gh', self.tools/'gh', '--git', shutil.which('git'), env=env)
        self.assertTrue((self.state/'bin/workboard-worker').is_file())

    def test_nested_projects_override_and_private_parent_checks(self):
        self.init('--projects-root', self.state/'projects/managed')
        self.assertTrue((self.state/'projects/managed').is_dir())
        (self.state/'secrets').chmod(0o755)
        self.cli('doctor', success=False)

    def npm_alias_fixture(self):
        package=self.root/'node_modules/@openai/codex'
        (package/'bin').mkdir(parents=True)
        import platform
        arch='arm64' if platform.machine().lower() in ('arm64','aarch64') else 'x64'
        system='darwin' if sys.platform=='darwin' else 'linux'
        suffix=system+'-'+arch
        alias='@openai/codex-'+suffix
        parent={'name':'@openai/codex','version':'0.153.4','optionalDependencies':{alias:'npm:@openai/codex@0.153.4-'+suffix}}
        (package/'package.json').write_text(json.dumps(parent))
        wrapper=package/'bin/codex.js';wrapper.write_text('#!/usr/bin/env node\nthrow new Error("must not execute wrapper")');wrapper.chmod(0o700)
        payload=package.parent/('codex-'+suffix);payload.mkdir()
        metadata={'name':'@openai/codex','version':'0.153.4-'+suffix}
        (payload/'package.json').write_text(json.dumps(metadata))
        target=('aarch64' if arch=='arm64' else 'x86_64')+('-apple-darwin' if system=='darwin' else '-unknown-linux-musl')
        binary=payload/'vendor'/target/'bin/codex';binary.parent.mkdir(parents=True)
        shutil.copyfile(self.tools/'native',binary);binary.chmod(0o700)
        return package,wrapper,payload,binary,parent,metadata,alias,suffix

    def test_npm_wrapper_resolves_real_alias_without_running_javascript(self):
        _,wrapper,_,binary,_,_,_,_=self.npm_alias_fixture()
        self.cli('init','--codex',wrapper,'--codex-home',self.auth,'--gh',self.tools/'gh','--git',shutil.which('git'),'--binary-dir',self.fake_bins)
        data=json.loads((self.state/'bootstrap.json').read_text())
        self.assertEqual(data['inputs']['codex'],str(binary))

    def test_npm_alias_wrong_platform_version_or_identity_is_rejected(self):
        _,wrapper,payload,_,_,metadata,_,suffix=self.npm_alias_fixture()
        wrong_platform='linux' if suffix.startswith('darwin') else 'darwin'
        for override in ({'version':'0.153.3-'+suffix}, {'version':'0.153.4-'+wrong_platform+'-'+suffix.split('-')[1]}, {'name':'@openai/codex-'+suffix}):
            with self.subTest(metadata=override):
                (payload/'package.json').write_text(json.dumps(dict(metadata,**override)))
                self.cli('init','--codex',wrapper,'--codex-home',self.auth,'--gh',self.tools/'gh','--git',shutil.which('git'),'--binary-dir',self.fake_bins,success=False)
                self.assertFalse(self.state.exists())

    def test_npm_alias_must_match_parent_optional_dependency(self):
        package,wrapper,_,_,parent,_,alias,suffix=self.npm_alias_fixture()
        for mapping in ({}, {alias:'npm:@openai/codex@0.153.3-'+suffix}, {alias:'file:../unreviewed'}):
            with self.subTest(mapping=mapping):
                parent['optionalDependencies']=mapping
                (package/'package.json').write_text(json.dumps(parent))
                self.cli('init','--codex',wrapper,'--codex-home',self.auth,'--gh',self.tools/'gh','--git',shutil.which('git'),'--binary-dir',self.fake_bins,success=False)
                self.assertFalse(self.state.exists())

    @unittest.skipUnless(os.environ.get('ORCHESTRATOR_TEST_BIN_DIR') or os.environ.get('ORCHESTRATOR_TEST_CARGO'), 'set ORCHESTRATOR_TEST_BIN_DIR to locally built host binaries for API onboarding')
    def test_real_server_paused_login_manual_task_and_live_update_refusal(self):
        with socket.socket() as sock:
            sock.bind(('127.0.0.1',0));port=sock.getsockname()[1]
        if os.environ.get('ORCHESTRATOR_TEST_CARGO'):
            env=dict(self.env, **{k:os.environ[k] for k in ('RUSTUP_HOME','CARGO_HOME') if k in os.environ})
            self.cli('init','--port',port,'--cargo',os.environ['ORCHESTRATOR_TEST_CARGO'],'--codex-home',self.auth,'--codex',self.tools/'codex','--gh',self.tools/'gh','--git',shutil.which('git'),env=env)
        else:
            self.init('--port',port,binary_dir=Path(os.environ['ORCHESTRATOR_TEST_BIN_DIR']))
        process=subprocess.Popen([sys.executable,str(CLI),'start','--state-dir',str(self.state)],env=self.env,stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,start_new_session=True)
        try:
            for _ in range(100):
                try:
                    connection=http.client.HTTPConnection('127.0.0.1',port,timeout=1);connection.request('GET','/api/workboard/v1/session');response=connection.getresponse();response.read();connection.close();break
                except OSError:
                    time.sleep(.05)
            else:self.fail('server did not start')
            origin='http://127.0.0.1:'+str(port)
            def request(method,path,value=None,headers=None):
                conn=http.client.HTTPConnection('127.0.0.1',port,timeout=10)
                body=None if value is None else json.dumps(value)
                h={'Origin':origin,'Content-Type':'application/json'};h.update(headers or {})
                conn.request(method,'/api/workboard/v1'+path,body,h);res=conn.getresponse();data=json.loads(res.read());conn.close();return res,data
            res,data=request('POST','/session',{'key':(self.state/'secrets/operator.key').read_text()})
            self.assertEqual(res.status,200)
            signed={'Cookie':res.getheader('Set-Cookie').split(';')[0],'X-CSRF-Token':data['csrf_token'],'Idempotency-Key':'synthetic-onboarding'}
            _,snapshot=request('GET','/snapshot',headers=signed)
            self.assertTrue(snapshot['execution']['paused']);self.assertEqual(snapshot['runs'],[])
            res,task=request('POST','/tasks',{'title':'Synthetic onboarding','objective':'Verify local manual ticket','workflow':'manual'},signed)
            self.assertEqual(res.status,201)
            self.assertEqual(task['task']['workflow'],'manual')
            _,snapshot=request('GET','/snapshot',headers=signed);self.assertEqual(len(snapshot['tasks']),1);self.assertEqual(snapshot['runs'],[])
            self.cli('init','--codex-home',self.auth,'--codex',self.tools/'codex','--gh',self.tools/'gh','--git',shutil.which('git'),'--port',port+1,success=False)
        finally:
            os.killpg(process.pid,signal.SIGTERM)
            process.wait(timeout=10)
            self.assertEqual(process.returncode,0)

if __name__=='__main__':unittest.main()
