#!/usr/bin/env python3
"""Real browser checks for isolated app storage and native geometry assertions.
Uses synthetic app data, never loads Gemma, and blocks external requests.
Requires Playwright and a desktop Chromium/Chrome executable.
"""
import argparse
import functools
import http.server
import json
import pathlib
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
        page.goto(f'http://127.0.0.1:{server.server_port}/demo/')
        result = page.evaluate('''async()=>{
            const {createPagePreview,runPageChecks}=await import('./page-runtime.mjs');
            const container=document.createElement('div');container.style.cssText='width:600px;height:300px';document.body.append(container);
            const updates=[];
            const first=await createPagePreview(container,{html:'<output id="value"></output>',css:'',js:`
                localStorage.setItem(7,42);localStorage.setItem('bool',false);
                sessionStorage.setItem('count',99);sessionStorage.setItem(7,8);
                document.querySelector('#value').textContent=JSON.stringify([localStorage.getItem(7),localStorage.getItem('count'),sessionStorage.getItem('count'),sessionStorage.getItem(7),localStorage.getItem('bool')]);
            `},{storage:{count:'3'},onStorage:value=>updates.push(value)});
            const firstChecks=await first.check([{name:'Storage coerces values and separates app maps',assertions:[{selector:'#value',property:'text',operator:'equals',expected:'["42","3","99","8","false"]'}]}]);
            const snapshot=await first.inspect();first.destroy();
            const second=await createPagePreview(container,{html:'<output id="value"></output>',css:'',js:`document.querySelector('#value').textContent=localStorage.getItem(7)+':'+sessionStorage.getItem('count');`},{storage:snapshot.storage});
            const reloadChecks=await second.check([{name:'Only local app storage persists in a new frame',assertions:[{selector:'#value',property:'text',operator:'equals',expected:'42:null'}]}]);second.destroy();container.remove();
            const geometry=await runPageChecks({html:'<div id="zero"></div>',css:'#zero{width:0;height:0}',js:`Object.defineProperty(DOMRect.prototype,'width',{get:()=>500});Object.defineProperty(DOMRect.prototype,'height',{get:()=>500});`},[{name:'A zero-size element cannot forge visibility',assertions:[{selector:'#zero',property:'visible',operator:'equals',expected:true}]}]);
            return {firstChecks,reloadChecks,snapshot,updates,geometry,parentValue:localStorage.getItem('7')};
        }''')
        assert all(row['pass'] for row in result['firstChecks']['rows']), result
        assert all(row['pass'] for row in result['reloadChecks']['rows']), result
        assert result['snapshot']['storage'] == {'count': '3', '7': '42', 'bool': 'false'}, result
        assert len(result['updates']) == 2 and result['updates'][-1] == result['snapshot']['storage'], result
        assert result['parentValue'] is None, result
        assert result['geometry']['rows'][0]['pass'] is False, result
        assert result['geometry']['rows'][-1]['pass'] is True, result
        if args.output:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(json.dumps(result, indent=2)+'\n')
        print('PASS: numeric/boolean storage coercion, session/local isolation, local-only reload persistence, private storage callbacks, parent isolation, native visibility geometry')
        browser.close()
finally:
    server.shutdown()
