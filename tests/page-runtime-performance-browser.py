#!/usr/bin/env python3
"""Actual Chrome fixture timing and cleanup checks, not Gemma inference evidence.
Compares the former serial fresh-frame algorithm with production bounded workers.
Every check still executes real app HTML/CSS/JS in its own empty-storage iframe.
"""
import argparse
import functools
import http.server
import json
import pathlib
import re
import shutil
import threading
from playwright.sync_api import sync_playwright
ROOT = pathlib.Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--browser', default=shutil.which('chromium') or shutil.which('google-chrome') or '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome')
parser.add_argument('--output', type=pathlib.Path)
args = parser.parse_args()
class Handler(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *args):
        pass
server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), functools.partial(Handler, directory=str(ROOT)))
threading.Thread(target=server.serve_forever, daemon=True).start()
try:
    with sync_playwright() as p:
        browser = p.chromium.launch(executable_path=args.browser, headless=True)
        page = browser.new_page()
        page.route('https://**/*', lambda route: route.abort())
        csp = re.search(r'http-equiv="Content-Security-Policy" content="([^"]+)"', (ROOT/'demo/index.html').read_text()).group(1)
        page.route('**/__runtime-performance-host.html', lambda route: route.fulfill(content_type='text/html', body='<meta http-equiv="Content-Security-Policy" content="'+csp+'"><main>Runtime benchmark host</main>'))
        page.goto(f'http://127.0.0.1:{server.server_port}/demo/__runtime-performance-host.html')
        result = page.evaluate('''async()=>{
            const {createPagePreview,runPageChecks}=await import('./page-runtime.mjs');
            const bundle={html:'<button id="increment">Increment</button><output id="value">booting</output>',css:'',js:`
                await new Promise(resolve=>setTimeout(resolve,600));
                const value=document.querySelector('#value');
                value.textContent=(localStorage.getItem('count')||'0')+':'+(sessionStorage.getItem('count')||'0');
                document.querySelector('#increment').onclick=()=>{const next=Number(localStorage.getItem('count')||0)+1;localStorage.setItem('count',next);sessionStorage.setItem('count',next);value.textContent=next+':'+next;};
            `};
            const checks=Array.from({length:6},(_,index)=>({name:'Fresh case '+index,steps:index%2===0?[{action:'wait',ms:150},{action:'click',selector:'#increment'}]:[],assertions:[{selector:'#value',property:'text',operator:'equals',expected:index%2===0?'1:1':'0:0'}]}));
            async function serial(){
                const container=document.createElement('div');container.style.cssText='width:1200px;height:900px';document.body.append(container);const rows=[];const started=performance.now();
                try{for(const check of checks){let preview;try{preview=await createPagePreview(container,bundle);const result=await preview.check([check]);rows.push(result.rows[0]);if(result.errors.length)throw Error('Serial fixture raised an error');}finally{preview?.destroy();}}return{ms:performance.now()-started,rows};}finally{container.remove();}
            }
            function watchFrames(){let peak=0;const seen=new Set();const update=()=>{const frames=document.querySelectorAll('iframe[title="Application preview"]');peak=Math.max(peak,frames.length);for(const frame of frames)seen.add(frame);};const observer=new MutationObserver(update);observer.observe(document.body,{childList:true,subtree:true});return{read(){update();return{peak,started:seen.size,remaining:document.querySelectorAll('iframe[title="Application preview"]').length};},stop(){observer.disconnect();}};}
            const baseline=await serial();
            const monitor=watchFrames(),started=performance.now();const parallel=await runPageChecks(bundle,checks);const parallelMs=performance.now()-started,parallelFrames=monitor.read();monitor.stop();
            const abort=new AbortController(),cancelMonitor=watchFrames();setTimeout(()=>abort.abort(),180);let cancelError;
            try{await runPageChecks(bundle,checks,{signal:abort.signal});}catch(error){cancelError=error.message;}
            const cancelled=cancelMonitor.read();cancelMonitor.stop();
            const failureMonitor=watchFrames();let failureError;
            try{await runPageChecks({...bundle,js:`await new Promise(resolve=>setTimeout(resolve,120));location.href='./how-it-works/';`},checks);}catch(error){failureError=error.message;}
            const failed=failureMonitor.read();failureMonitor.stop();
            const sourceErrors=await runPageChecks({...bundle,js:'throw new Error("Deliberate benchmark source error")'},checks.slice(0,3).map(check=>({...check,steps:[],assertions:[{selector:'#value',property:'text',operator:'equals',expected:'booting'}]})));
            return{label:'Actual Chrome synthetic startup fixture; not Gemma inference timing',baseline,parallel,parallelMs,speedup:baseline.ms/parallelMs,parallelFrames,cancelError,cancelled,failureError,failed,sourceErrors};
        }''')
        assert all(row['pass'] for row in result['baseline']['rows']), result
        assert all(row['pass'] for row in result['parallel']['rows']), result
        assert [row['name'] for row in result['parallel']['rows'][:-1]] == [row['name'] for row in result['baseline']['rows']], result
        assert result['parallelFrames'] == {'peak':2, 'started':6, 'remaining':0}, result
        assert result['speedup'] > 1.35, result
        for label in ['cancelled','failed']:
            assert result[label]['remaining'] == 0 and 1 <= result[label]['peak'] <= 2 and result[label]['started'] <= 2, result
        assert result['cancelError'] and result['failureError'], result
        assert len(result['sourceErrors']['errors']) == 3 and result['sourceErrors']['rows'][-1]['pass'] is False, result
        if args.output:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(json.dumps(result, indent=2)+'\n')
        print(f"PASS: serial {result['baseline']['ms']:.0f}ms; two workers {result['parallelMs']:.0f}ms; {result['speedup']:.2f}x fixture speedup; six real fresh cases; max two frames; abort/failure cleanup; ordered rows and aggregated errors")
        browser.close()
finally:
    server.shutdown()
