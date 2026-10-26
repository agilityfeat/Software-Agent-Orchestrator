/* Trusted preview harness. This is a classic script so an opaque sandbox does
 * not need a CORS-enabled static server. Candidate code never receives its port. */
(()=>{
  'use strict';
  const apply=Reflect.apply,define=Object.defineProperty,create=Object.create,keys=Object.keys,freeze=Object.freeze;
  const nativeThen=Promise.prototype.then,NativeBlob=Blob,blobURL=URL.createObjectURL.bind(URL),revokeURL=URL.revokeObjectURL.bind(URL);
  const NativePromise=Promise,NativeRegExp=RegExp,NativeError=Error,NativeString=String;
  const timeout=window.setTimeout.bind(window),add=EventTarget.prototype.addEventListener,remove=EventTarget.prototype.removeEventListener,dispatch=EventTarget.prototype.dispatchEvent;
  const qsa=Document.prototype.querySelectorAll,eqsa=Element.prototype.querySelectorAll,attr=Element.prototype.getAttribute,setAttr=Element.prototype.setAttribute,rect=Element.prototype.getBoundingClientRect;
  const innerTextGetter=Object.getOwnPropertyDescriptor(HTMLElement.prototype,'innerText').get;
  const textGetter=Object.getOwnPropertyDescriptor(Node.prototype,'textContent').get,htmlGetter=Object.getOwnPropertyDescriptor(Element.prototype,'outerHTML').get,tagGetter=Object.getOwnPropertyDescriptor(Element.prototype,'tagName').get;
  const inputValue=Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value'),textValue=Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value'),selectValue=Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype,'value'),checkedGetter=Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'checked').get;
  const click=HTMLElement.prototype.click,append=Node.prototype.appendChild,createElement=Document.prototype.createElement,domParse=DOMParser.prototype.parseFromString;
  const rectWidth=Object.getOwnPropertyDescriptor(DOMRectReadOnly.prototype,'width').get,rectHeight=Object.getOwnPropertyDescriptor(DOMRectReadOnly.prototype,'height').get;
  const styleGetter=window.getComputedStyle.bind(window),styleValue=CSSStyleDeclaration.prototype.getPropertyValue;
  const listLength=Object.getOwnPropertyDescriptor(NodeList.prototype,'length').get;
  const NativeEvent=Event,NativeKeyboardEvent=KeyboardEvent,NativeDOMParser=DOMParser,regexExec=RegExp.prototype.exec;
  const messageData=Object.getOwnPropertyDescriptor(MessageEvent.prototype,'data').get;
  const errorMessage=Object.getOwnPropertyDescriptor(ErrorEvent.prototype,'message').get,rejectionReason=Object.getOwnPropertyDescriptor(PromiseRejectionEvent.prototype,'reason').get;
  const portPost=MessagePort.prototype.postMessage,portStart=MessagePort.prototype.start;
  const slice=String.prototype.slice,includes=String.prototype.includes;
  const put=(array,value)=>{define(array,array.length,{value,writable:true,enumerable:true,configurable:true});},cut=(value,n)=>apply(slice,value,[0,n]);
  const charCode=String.prototype.charCodeAt;
  const trim=value=>{let out='',space=false;for(let i=0;i<value.length;i++){const code=apply(charCode,value,[i]);if(code===32||code===9||code===10||code===13||code===12){if(out)space=true;}else{if(space){out+=' ';space=false;}out+=value[i];}}return out;};
  const length=list=>apply(listLength,list,[]);
  const copyArray=value=>{const out=[];for(let i=0;i<value.length;i++)put(out,value[i]);return out;};
  const data=value=>{const out=create(null),names=keys(value);for(let i=0;i<names.length;i++)define(out,names[i],{value:value[names[i]],enumerable:true,writable:true,configurable:true});return out;};
  const wait=ms=>new NativePromise(resolve=>timeout(resolve,ms));
  let port,loaded=false,errors=[],store=create(null);
  const send=value=>{if(port)apply(portPost,port,[value]);};
  const errorText=value=>typeof value==='string'?cut(value,1000):'The page raised an error.';
  const record=value=>{const message=errorText(value);if(errors.length<30)put(errors,message);send({type:'error',error:message});};
  apply(add,window,['error',event=>{if(loaded){let message;try{message=apply(errorMessage,event,[]);}catch{}record(message||'A page resource failed to load.');}},true]);
  apply(add,window,['unhandledrejection',event=>{if(loaded){const reason=apply(rejectionReason,event,[]);record(typeof reason==='string'?reason:reason?.message);}},true]);
  function storageSnapshot(){const out=create(null),names=keys(store);for(let i=0;i<names.length;i++)out[names[i]]=store[names[i]];return out;}
  function updateStorage(){send({type:'storage',storage:storageSnapshot()});}
  function installStorage(initial){
    function makeStorage(seed,persist){
      let memory=create(null),bytes=0;
      if(seed&&typeof seed==='object'){const names=keys(seed);for(let i=0;i<names.length&&i<200;i++){const key=names[i],value=seed[key];if(typeof value==='string'&&key.length<500&&value.length<100000&&bytes+value.length<500000){memory[key]=value;bytes+=value.length;}}}
      if(persist)store=memory;
      const changed=()=>{if(persist){store=memory;updateStorage();}};
      const api={getItem(key){key=NativeString(key);return memory[key]!==undefined?memory[key]:null;},setItem(key,value){key=NativeString(key);value=NativeString(value);if((memory[key]===undefined&&keys(memory).length>=200)||key.length>500||value.length>100000||bytes-(memory[key]?.length||0)+value.length>500000)throw new NativeError('App storage limit reached.');bytes-=memory[key]?.length||0;memory[key]=value;bytes+=value.length;changed();},removeItem(key){key=NativeString(key);bytes-=memory[key]?.length||0;delete memory[key];changed();},clear(){memory=create(null);bytes=0;changed();},key(index){return keys(memory)[index]??null;}};
      define(api,'length',{get:()=>keys(memory).length});return freeze(api);
    }
    define(window,'localStorage',{value:makeStorage(initial,true),writable:false,configurable:false});
    define(window,'sessionStorage',{value:makeStorage({},false),writable:false,configurable:false});
  }
  function query(selector){return apply(qsa,document,[selector]);}
  function nodeValue(node,property,attribute){
    if(property==='text'){let value;try{value=apply(innerTextGetter,node,[]);}catch{value=apply(textGetter,node,[]);}return trim(value||'');}
    if(property==='style')return apply(styleValue,styleGetter(node),[attribute]);
    if(property==='attribute')return apply(attr,node,[attribute]);
    if(property==='checked')return apply(checkedGetter,node,[]);
    if(property==='value'){const tag=apply(tagGetter,node,[]);return apply((tag==='TEXTAREA'?textValue:tag==='SELECT'?selectValue:inputValue).get,node,[]);}
    if(property==='visible'){const box=apply(rect,node,[]),style=styleGetter(node);return apply(rectWidth,box,[])>0&&apply(rectHeight,box,[])>0&&apply(styleValue,style,['display'])!=='none'&&apply(styleValue,style,['visibility'])!=='hidden'&&apply(styleValue,style,['opacity'])!=='0';}
    throw new NativeError('Unknown property.');
  }
  function compare(actual,operator,expected){
    if(operator==='equals')return actual===expected;
    if(operator==='includes')return typeof actual==='string'&&apply(includes,actual,[expected]);
    if(operator==='matches')return typeof actual==='string'&&apply(regexExec,new NativeRegExp(expected,'u'),[actual])!==null;
    if(operator==='gt')return actual>expected;
    if(operator==='gte')return actual>=expected;
    return false;
  }
  async function action(step){
    if(step.action==='wait'){await wait(step.ms);return;}
    const nodes=query(step.selector);if(!length(nodes))throw new NativeError('Action element not found: '+step.selector);const node=nodes[0];
    if(step.action==='click')apply(click,node,[]);
    else if(step.action==='key'){apply(dispatch,node,[new NativeKeyboardEvent('keydown',{key:step.key,bubbles:true})]);apply(dispatch,node,[new NativeKeyboardEvent('keyup',{key:step.key,bubbles:true})]);}
    else{const tag=apply(tagGetter,node,[]),descriptor=tag==='TEXTAREA'?textValue:tag==='SELECT'?selectValue:inputValue;apply(descriptor.set,node,[step.value]);apply(dispatch,node,[new NativeEvent('input',{bubbles:true})]);apply(dispatch,node,[new NativeEvent('change',{bubbles:true})]);}
    await wait(35);
  }
  function inspect(){const root=document.documentElement;return data({html:cut(root?apply(htmlGetter,root,[]):'',40000),text:cut(document.body?trim(apply(innerTextGetter,document.body,[])||''):'',12000),errors:copyArray(errors),storage:storageSnapshot()});}
  async function check(checks){
    const rows=[];
    for(let i=0;i<checks.length;i++){
      const check=checks[i],details=[];let pass=true;
      try{
        for(let j=0;j<check.steps.length;j++)await action(check.steps[j]);
        for(let j=0;j<check.assertions.length;j++){
          const assertion=check.assertions[j],nodes=query(assertion.selector),actual=[];let okay=true;
          if(assertion.property==='count'){put(actual,length(nodes));okay=compare(length(nodes),assertion.operator,assertion.expected);}
          else if(!length(nodes))okay=false;
          else{
            const count=assertion.first?1:length(nodes);okay=assertion.all===true;
            for(let k=0;k<count&&k<10000;k++){
              let matches=false;
              try{const value=nodeValue(nodes[k],assertion.property,assertion.attribute);put(actual,typeof value==='string'?cut(value,1000):value);matches=compare(value,assertion.operator,assertion.expected);}catch(error){put(actual,{error:errorText(error?.message)});}
              if(assertion.all){if(!matches)okay=false;}else if(matches)okay=true;
            }
            if(length(nodes)>10000)okay=false;
          }
          if(!okay)pass=false;put(details,{selector:assertion.selector,property:assertion.property,operator:assertion.operator,expected:assertion.expected,actual,pass:okay});
        }
      }catch(error){pass=false;put(details,{error:errorText(error?.message)});}
      put(rows,{name:check.name,pass,expected:check.assertions,actual:details});
    }
    await wait(75);
    put(rows,{name:'Page runs without browser errors',pass:errors.length===0,expected:[],actual:copyArray(errors)});
    return data({rows,errors:copyArray(errors),observations:inspect()});
  }
  function executeModule(source,url){return new NativePromise(resolve=>{let done=false,objectURL;const finish=error=>{if(done)return;done=true;if(objectURL)revokeURL(objectURL);if(error)record(errorText(error?.message||error));resolve();};try{const moduleURL=url||(objectURL=blobURL(new NativeBlob([source],{type:'text/javascript'})));const promise=import(moduleURL);apply(nativeThen,promise,[()=>finish(),error=>finish(error)]);timeout(()=>finish('App JavaScript did not finish loading within five seconds.'),5000);}catch(error){finish(error);}});}
  function completion(element,label){return new NativePromise(resolve=>{let done=false;const finish=failed=>{if(done)return;done=true;if(failed)record(label);resolve();};apply(add,element,['load',()=>finish(false),{once:true}]);apply(add,element,['error',()=>finish(true),{once:true}]);timeout(()=>finish(true),5000);});}
  async function load(bundle,initialStorage){
    installStorage(initialStorage);loaded=true;
    const parser=new NativeDOMParser(),parsed=apply(domParse,parser,[bundle.html,'text/html']),scripts=[];
    // Copy app markup, preserving arbitrary HTML while executing scripts only
    // after the private connection and trusted capture have been established.
    const allScripts=apply(eqsa,parsed.documentElement,['script']);for(let i=0;i<allScripts.length;i++)put(scripts,allScripts[i]);
    for(let i=0;i<scripts.length;i++)scripts[i].remove();
    const headNodes=parsed.head.childNodes;while(headNodes.length){const node=headNodes[0];if(node.nodeType===1&&['BASE','META','TITLE'].includes(node.tagName)){node.remove();continue;}apply(append,document.head,[node]);}
    for(const pair of [[parsed.documentElement,document.documentElement],[parsed.body,document.body]]){const attributes=pair[0].attributes;for(let i=0;i<attributes.length;i++)apply(setAttr,pair[1],[attributes[i].name,attributes[i].value]);}
    const bodyNodes=parsed.body.childNodes;while(bodyNodes.length)apply(append,document.body,[bodyNodes[0]]);
    const style=apply(createElement,document,['style']);style.textContent=bundle.css;apply(append,document.head,[style]);
    for(let i=0;i<scripts.length;i++){
      const original=scripts[i];if(original.type==='module'){await executeModule(apply(textGetter,original,[])||'',original.src||undefined);continue;}const script=apply(createElement,document,['script']);
      const attributes=original.attributes;for(let j=0;j<attributes.length;j++){const a=attributes[j];apply(setAttr,script,[a.name,a.value]);}
      script.textContent=apply(textGetter,original,[])||'';
      let settled;
      if(script.src||script.type==='module')settled=completion(script,'A page script failed or timed out: '+(script.src||'module'));
      apply(append,document.body,[script]);if(settled)await settled;
    }
    if(bundle.js)await executeModule(bundle.js);
    apply(dispatch,document,[new NativeEvent('DOMContentLoaded',{bubbles:true})]);apply(dispatch,window,[new NativeEvent('load')]);await wait(100);return inspect();
  }
  const token=new URLSearchParams(location.search).get('preview-token');
  function connect(event){
    if(event.source!==parent||event.data?.type!=='page-preview-connect'||event.data?.token!==token||port||!event.ports?.[0])return;
    apply(remove,window,['message',connect,true]);event.stopImmediatePropagation();port=event.ports[0];
    apply(add,port,['message',async event=>{
      const message=apply(messageData,event,[]);if(!message||typeof message!=='object')return;
      try{let result;if(message.type==='load'){if(loaded)throw new NativeError('Preview already loaded.');result=await load(message.bundle,message.storage);}else if(message.type==='inspect')result=inspect();else if(message.type==='check')result=await check(message.checks);else throw new NativeError('Unknown preview request.');send({id:message.id,result});}catch(error){send({id:message.id,error:errorText(error?.message)});}
    }]);apply(portStart,port,[]);
  }
  apply(add,window,['message',connect,true]);parent.postMessage({type:'page-preview-boot',token},'*');
})();
