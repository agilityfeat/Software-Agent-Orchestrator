import {validateChecks} from './page-checks.mjs';
import {digest,extractJSON} from './core.mjs';
export {digest,extractJSON};
export const CHECK_SEMANTICS='any-node-isolated-v2';
export const DEFAULT_REQUEST="Let's add a date in table.";
export function validateBundle(value){
  if(!value||typeof value!=='object'||Array.isArray(value)||Object.keys(value).sort().join(',')!=='css,html,js')throw Error('The app needs HTML, CSS and JavaScript files.');
  const bundle={};for(const file of ['html','css','js']){if(typeof value[file]!=='string'||value[file].length>32000)throw Error('Each app file must be text, up to 32,000 characters.');bundle[file]=value[file];}
  if(!Object.values(bundle).join('').trim()||Object.values(bundle).join('').length>64000)throw Error('App files must contain source and fit within the local workspace size limit.');
  return bundle;
}
export function canonicalBundle(value){return JSON.stringify(validateBundle(value));}
export function applyEdits(base,value){
  const next=validateBundle(base);
  if(!value||typeof value!=='object'||Array.isArray(value))throw Error('Coder must return a JSON edit object.');
  if(Object.keys(value).length!==1)throw Error('Return only edits or files.');
  if(Array.isArray(value.edits)){
    if(!value.edits.length||value.edits.length>16)throw Error('Return between one and sixteen exact edits.');
    for(const edit of value.edits){
      if(!edit||Object.keys(edit).sort().join(',')!=='file,find,replace'||!['html','css','js'].includes(edit.file)||typeof edit.find!=='string'||!edit.find||typeof edit.replace!=='string')throw Error('Each edit needs file, nonempty find, and replace text.');
      if(edit.find===edit.replace)throw Error('This edit changes nothing; omit it and implement the requested rendering or behavior.');
      const source=next[edit.file],at=source.indexOf(edit.find);
      if(at<0){let near='';for(const size of [60,30,15,8]){const anchor=edit.find.slice(0,size),position=source.indexOf(anchor);if(position>=0){near=source.slice(Math.max(0,position-30),position+320);break;}}throw Error(`The ${edit.file} find text was not found: ${JSON.stringify(edit.find.slice(0,300))}. Use a shorter unique exact substring; preserve whitespace.${near?' Nearby current source: '+JSON.stringify(near):''}`);}
      if(source.indexOf(edit.find,at+1)!==-1)throw Error(`The ${edit.file} find text matches more than one place: ${JSON.stringify(edit.find.slice(0,300))}. Include enough nearby text to identify one exact place.`);
      next[edit.file]=source.slice(0,at)+edit.replace+source.slice(at+edit.find.length);
    }
  }else if(value.files&&typeof value.files==='object'&&!Array.isArray(value.files)&&Object.keys(value.files).length){
    for(const [file,content]of Object.entries(value.files)){if(!['html','css','js'].includes(file)||typeof content!=='string')throw Error('Replacement files must be HTML, CSS or JavaScript text.');next[file]=content;}
  }else throw Error('Coder must return edits or complete replacement files.');
  const result=validateBundle(next);if(canonicalBundle(result)===canonicalBundle(base))throw Error('The candidate did not change any app source.');return result;
}
export function validateConsistentChecks(value){
  const checks=validateChecks(value),observations=new Map();
  for(const check of checks)for(const assertion of check.assertions){
    if(assertion.operator!=='equals')continue;
    const quantifier=assertion.first===true?'first':assertion.all===true?'all':'any';
    // ANY over a heterogeneous list may legitimately match different values.
    const singleton=assertion.property==='count'||quantifier!=='any'||/^#[A-Za-z_][\w-]*$/.test(assertion.selector)||['body','html',':root'].includes(assertion.selector);
    if(!singleton)continue;
    const key=JSON.stringify([check.steps,assertion.selector,assertion.property,assertion.attribute||'',quantifier]);
    if(observations.has(key)&&!Object.is(observations.get(key),assertion.expected))throw Error('Browser checks contradict each other for the same observation and actions. Supply explicit click/actions for a state change, or distinct selectors for different nodes.');
    observations.set(key,assertion.expected);
  }
  return checks;
}
export function modelChecks(value){
  if(!Array.isArray(value))throw Error('Browser checks must be an array.');
  const normalized=value.map(check=>{
    if(check?.assertions)return check;
    if(!check||typeof check!=='object')throw Error('Use flat browser checks with name, selector, property, operator and expected.');
    const {name,click,actions,selector,property,operator,expected,attribute,all,first}=check;
    const assertion={selector,property,operator,expected};if(attribute!==undefined)assertion.attribute=attribute;if(all!==undefined)assertion.all=all;if(first!==undefined)assertion.first=first;
    if(click!==undefined&&(typeof click!=='string'||!click.trim()))throw Error('click must be a DOM selector.');
    if(actions!==undefined&&!Array.isArray(actions))throw Error('actions must be an array of browser actions.');
    return {name,steps:[...(actions||[]),...(click?[{action:'click',selector:click}]:[])],assertions:[assertion]};
  });return validateConsistentChecks(normalized);
}
export function validatePlan(value){
  if(!value||typeof value.summary!=='string'||!value.summary.trim()||value.summary.length>600||!Array.isArray(value.steps)||!value.steps.length||value.steps.length>3||value.steps.some(step=>typeof step!=='string'||!step.trim()||step.length>300))throw Error('Planner must return a summary, one to three steps and browser checks.');
  return {summary:value.summary,steps:value.steps,checks:modelChecks(value.checks),checkSemantics:CHECK_SEMANTICS};
}
export function qaCases(value,remaining){
  if(!value||Object.keys(value).sort().join(',')!=='cases,summary'||typeof value.summary!=='string'||!value.summary.trim()||value.summary.length>1000||!Array.isArray(value.cases)||value.cases.length>remaining)throw Error(`QA must return a summary and at most ${remaining} additional browser checks. Use cases:[] when the approved checks cover the request.`);
  return value.cases.length?modelChecks(value.cases):[];
}
export function canAccept(t){return Boolean(t?.kind==='workspace'&&t.plan?.checkSemantics===CHECK_SEMANTICS&&t.mode!=='recorded'&&t.status==='Review'&&t.candidate&&t.hash&&t.evidence?.hash===t.hash&&t.evidence?.checksHash&&t.evidence.rows?.length&&t.evidence.rows.every(row=>row.pass===true));}
export function successfulLiveRun(t){return Boolean(t?.kind==='workspace'&&t.mode==='live'&&['Review','Done'].includes(t.status)&&canAccept({...t,status:'Review'}));}
export const CHECK_HELP=`Use 1–2 simple flat browser checks by default, at most 4: {"name":"what is checked","selector":"body","property":"text","operator":"includes","expected":"requested visible text"}. Each object is one assertion, no assertions/steps nesting needed. Each check starts in a fresh page with empty app storage; it does not inherit actions or state from another check. To check a changed state, include every required click/action in that check; naming an interaction does not perform it. For static visible-text outcomes default to selector body, property text, operator includes. Use an exact existing stable ID only when the location matters. Avoid invented or deep descendant chains: an ID already selects that element, so do not append its own tag as a child. For new interactive elements choose short explicit IDs. Node assertions pass when ANY matched node satisfies the assertion by default. Use all:true to require EVERY matched node, or first:true to check only the FIRST. Never combine all:true with first:true. A selector matching no nodes fails node assertions. Count checks count every match; actions target the first match. For visible text you may also use a container such as body or thead. For static text DO NOT click. Check only rendered DOM, never JavaScript variables/source: there is no js element. property: text,count,value,checked,visible,attribute,style. Operators: equals,includes,matches,gt,gte; expected is a string for text/value/style/attribute, number for count, boolean for visible/checked. For style/attribute also supply attribute (CSS property/DOM attribute name). Prefer includes/equals. matches permits exact {N} repeats up to 100, no ranges/groups/nesting, at most one variable quantifier. Only if an interaction is required add "click":"button selector"; for multiple actions use "actions":[{"action":"input","selector":"input selector","value":"text"},{"action":"click","selector":"button selector"}]. No code, pass flags or JS-source selectors. Valid hidden-state check: {"name":"Panel starts hidden","selector":"#details","property":"visible","operator":"equals","expected":false}. Valid interaction check: {"name":"Click shows panel","click":"#show-details","selector":"#details","property":"visible","operator":"equals","expected":true}. For a style assertion use attribute:"display", expected:"none", never expected:"display: none". Absence is property:"count", operator:"equals", expected:0 with a precise CSS selector; never invent not-includes or children.`;
