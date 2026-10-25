// Models can choose values here, never selectors, HTML, JavaScript or arbitrary CSS.
export const DEFAULT_APP_REQUEST='Make view receipt match the rest of the look and feel.';
export const SURFACES={
  expenseApp:{label:'Expense app',selector:'.fintech',properties:['background','color','borderColor','borderRadius']},
  receiptDialog:{label:'Receipt dialog',selector:'#receipt-dialog',properties:['background','color','borderColor','borderRadius','fontSize','padding']},
  receiptButton:{label:'View receipt button',selector:'[data-receipt-trigger]',properties:['background','color','borderColor','borderRadius','fontSize']},
  dialogButton:{label:'Receipt close button',selector:'#receipt-dialog button',properties:['background','color','borderColor','borderRadius','fontSize']},
  cards:{label:'Summary cards',selector:'.stats>div',properties:['background','color','borderColor','borderRadius']},
  headings:{label:'App heading',selector:'.expense-heading h2',properties:['color','fontSize']},
  tableRows:{label:'Expense rows',selector:'.fintech td',properties:['color','fontSize','paddingBlock']},
};
const CSS={background:'background-color',color:'color',borderColor:'border-color',borderRadius:'border-radius',fontSize:'font-size',padding:'padding',paddingBlock:'padding-block'};
const LIMITS={borderRadius:[0,24],fontSize:[9,32],padding:[4,32],paddingBlock:[4,24]};
const object=value=>Boolean(value&&typeof value==='object'&&!Array.isArray(value));
const keys=(value,allowed)=>object(value)&&Object.keys(value).every(key=>allowed.includes(key));
function checkedValue(property,value){
  if(['background','color','borderColor'].includes(property)){
    if(property==='borderColor'&&value==='transparent')return value;
    if(typeof value!=='string'||!/^#[\da-f]{6}$/i.test(value))throw Error(`${property} must be a six-digit hex color.`);
    return value.toLowerCase();
  }
  if(typeof value==='string'&&/^(?:0|[1-9]\d*)(?:\.\d{1,2})?$/.test(value.trim()))value=Number(value.trim());
  const [min,max]=LIMITS[property]||[];
  if(typeof value!=='number'||!Number.isFinite(value)||value<min||value>max)throw Error(`${property} must be a number from ${min} to ${max}.`);
  return Math.round(value*100)/100;
}
export function canonicalAppearance(config){
  if(!keys(config,['version','surfaces'])||config.version!==1||!keys(config.surfaces,Object.keys(SURFACES)))throw Error('Appearance configuration has an unsupported shape.');
  const surfaces={};
  for(const [target,spec]of Object.entries(SURFACES)){
    const values=config.surfaces[target];
    if(!keys(values,spec.properties)||Object.keys(values).length!==spec.properties.length)throw Error(`Appearance must include the complete ${target} baseline.`);
    surfaces[target]=Object.fromEntries(spec.properties.map(property=>[property,checkedValue(property,values[property])]));
  }
  return JSON.stringify({version:1,surfaces});
}
export function appearanceGoals(goals){
  if(!Array.isArray(goals)||!goals.length||goals.length>7)throw Error('Plan one to seven supported appearance surfaces.');
  const seen=new Set();
  const result=goals.map(goal=>{
    if(!keys(goal,['target','intent'])||!Object.hasOwn(SURFACES,goal.target)||!['match-app','adjust'].includes(goal.intent)||seen.has(goal.target))throw Error('Planner must choose unique supported surfaces and match-app or adjust intent.');
    seen.add(goal.target);return {target:goal.target,intent:goal.intent};
  });
  const receiptTargets=['receiptDialog','receiptButton','dialogButton'];
  if(result.some(goal=>receiptTargets.includes(goal.target)&&goal.intent==='match-app')){
    if(result.some(goal=>receiptTargets.includes(goal.target)&&goal.intent!=='match-app'))throw Error('Matching the receipt experience must use one consistent match-app intent.');
    for(const target of receiptTargets)if(!seen.has(target))result.push({target,intent:'match-app'});
  }
  return result;
}
export function appearanceDiff(base,config){
  const before=JSON.parse(canonicalAppearance(base)),after=JSON.parse(canonicalAppearance(config)),changes={};
  for(const [target,spec]of Object.entries(SURFACES))for(const property of spec.properties)if(before.surfaces[target][property]!==after.surfaces[target][property]){
    changes[target]??={};changes[target][property]=after.surfaces[target][property];
  }
  return changes;
}
export function validateOverrides(overrides){
  if(!keys(overrides,Object.keys(SURFACES)))throw Error('Appearance overrides contain unsupported surfaces.');
  const result={};
  for(const target of Object.keys(SURFACES).filter(target=>Object.hasOwn(overrides,target))){
    const values=overrides[target];
    if(!keys(values,SURFACES[target].properties))throw Error('Appearance overrides contain unsupported properties.');
    result[target]=Object.fromEntries(SURFACES[target].properties.filter(property=>Object.hasOwn(values,property)).map(property=>[property,checkedValue(property,values[property])]));
  }
  return result;
}
export function mergeOverrides(base,changes){
  const result=validateOverrides(base||{});
  for(const [target,values]of Object.entries(validateOverrides(changes)))result[target]={...(result[target]||{}),...values};
  return result;
}
export function matchProfile(target,base){
  const app=base.surfaces.expenseApp,cards=base.surfaces.cards;
  const profile={background:cards.background,color:app.color,borderColor:app.borderColor,borderRadius:app.borderRadius};
  if(['receiptButton','dialogButton'].includes(target))Object.assign(profile,{background:cards.background,color:app.color,borderRadius:7,fontSize:12});
  if(target==='receiptDialog')profile.fontSize=13;
  return Object.fromEntries(Object.entries(profile).filter(([property])=>SURFACES[target].properties.includes(property)));
}
export function normalizeAppearancePatch(value,base,goals){
  if(!keys(value,['changes'])||!Array.isArray(value.changes)||!value.changes.length||value.changes.length>32)throw Error('Return JSON with changes: one to 32 appearance token changes.');
  const config=JSON.parse(canonicalAppearance(base)),approved=new Set(appearanceGoals(goals).map(goal=>goal.target)),seen=new Set();
  for(const change of value.changes){
    if(!keys(change,['target','property','value'])||!approved.has(change.target)||!SURFACES[change.target].properties.includes(change.property))throw Error('A change is outside the approved appearance surfaces or properties.');
    const key=`${change.target}.${change.property}`;
    if(seen.has(key))throw Error('Do not repeat an appearance property.');seen.add(key);
    config.surfaces[change.target][change.property]=checkedValue(change.property,change.value);
  }
  return canonicalAppearance(config);
}
export function contrast(foreground,background){
  const luminance=hex=>{const channels=[1,3,5].map(i=>parseInt(hex.slice(i,i+2),16)/255).map(v=>v<=.04045?v/12.92:((v+.055)/1.055)**2.4);return channels[0]*.2126+channels[1]*.7152+channels[2]*.0722;};
  const a=luminance(foreground),b=luminance(background);return (Math.max(a,b)+.05)/(Math.min(a,b)+.05);
}
export function appearanceChecks(config,base,goals){
  config=JSON.parse(canonicalAppearance(config));base=JSON.parse(canonicalAppearance(base));goals=appearanceGoals(goals);
  const changes=appearanceDiff(base,config),approved=new Set(goals.map(goal=>goal.target));
  const row=(key,name,pass,actual,expected)=>({key,name,pass,actual,expected});
  const rows=[row('appearance:schema','Only approved appearance tokens changed',Object.keys(changes).every(target=>approved.has(target)),Object.keys(changes),'Approved surfaces only'),row('appearance:changed','The preview contains a real change',Object.keys(changes).length>0,Object.keys(changes),'At least one changed token')];
  for(const goal of goals){
    const values=config.surfaces[goal.target];
    const background=values.background||config.surfaces.expenseApp.background;
    const ratio=contrast(values.color,background);
    rows.push(row(`contrast:${goal.target}`,`${SURFACES[goal.target].label}: readable text`,ratio>=4.5,Math.round(ratio*100)/100,'Contrast at least 4.5:1'));
    if(goal.intent==='match-app'){
      const expected=matchProfile(goal.target,base),pass=Object.entries(expected).every(([property,value])=>values[property]===value);
      rows.push(row(`goal:${goal.target}`,`${SURFACES[goal.target].label}: matches the app style`,pass,values,expected));
    }else rows.push(row(`goal:${goal.target}`,`${SURFACES[goal.target].label}: requested surface changed`,Object.hasOwn(changes,goal.target),changes[goal.target]||{},'A visible token change on this surface'));
  }
  return rows;
}
export function appearanceCriteria(task){return [
  ...task.appearanceGoals.map(goal=>`${SURFACES[goal.target].label}: ${goal.intent==='match-app'?'match the existing app colors and corners':'apply the reviewed appearance change'}.`),
  'Keep text readable, and change only the reviewed surfaces.',
  'Keep receipt content, expense data, approval rules and paid history unchanged.',
  'Preview the result before accepting it.',
];}
export function appearanceEvidenceValid(task){
  try{
    if(canonicalAppearance(JSON.parse(task.candidate))!==task.candidate)return false;
    const checks=appearanceChecks(JSON.parse(task.candidate),JSON.parse(task.base),task.appearanceGoals);
    return checks.every(check=>check.pass&&task.evidence.rows.some(row=>row.key===check.key&&row.pass))&&task.evidence.rows.some(row=>row.key==='policy:unchanged'&&row.pass);
  }catch{return false;}
}
export function extraAppearanceChecks(value,goals){
  if(!value||typeof value.summary!=='string')throw Error('QA appearance summary must be a string.');
  if(!Array.isArray(value.cases))throw Error('QA appearance cases must be an array; use [] because required checks already ran.');
  if(value.cases.length>6)throw Error(`QA returned ${value.cases.length} additional appearance checks; maximum is 6.`);
  const allowed=new Set(goals.map(goal=>goal.target));
  return value.cases.map((item,index)=>{
    if(!object(item)||!allowed.has(item.target))throw Error(`QA appearance case ${index+1} has a missing or unapproved target. Allowed targets: ${[...allowed].join(', ')}.`);
    if(!['contrast','changed','match-app'].includes(item.check))throw Error(`QA appearance case ${index+1} has an unsupported check. Use contrast, changed or match-app.`);
    if(!keys(item,['target','check']))throw Error(`QA appearance case ${index+1} must contain only target and check, without copied test results.`);
    return {target:item.target,check:item.check};
  });
}
export function scoreExtraAppearance(checks,config,base){return checks.map((item,index)=>{
  const goal={target:item.target,intent:item.check==='match-app'?'match-app':'adjust'};
  const rows=appearanceChecks(config,base,[goal]);const row=rows.find(row=>row.key===`${item.check==='contrast'?'contrast':'goal'}:${item.target}`);
  return {...row,key:`qa:${index}`,name:`QA: ${row.name}`};
});}
function rgb(value){
  const numbers=value.match(/[\d.]+/g)?.map(Number);
  if(!numbers||numbers.length<3)return null;
  if(numbers.length>3&&numbers[3]===0)return null;
  if(numbers.length>3&&numbers[3]!==1)throw Error('Partially transparent appearance colors are unsupported.');
  return '#'+numbers.slice(0,3).map(v=>Math.round(v).toString(16).padStart(2,'0')).join('');
}
function background(node){for(let item=node;item;item=item.parentElement){const value=rgb(getComputedStyle(item).backgroundColor);if(value)return value;}return '#ffffff';}
export function captureAppearance(){
  const surfaces={};
  for(const [target,spec]of Object.entries(SURFACES)){
    const node=document.querySelector(spec.selector);if(!node)throw Error(`The ${spec.label} is unavailable.`);
    const style=getComputedStyle(node);surfaces[target]={};
    for(const property of spec.properties){
      const raw=style.getPropertyValue(CSS[property]);
      surfaces[target][property]=property==='background'?background(node):property==='color'?rgb(raw):property==='borderColor'?(rgb(raw)||'transparent'):parseFloat(raw);
    }
  }
  return JSON.parse(canonicalAppearance({version:1,surfaces}));
}
export function applyAppearance(overrides={}){
  overrides=validateOverrides(overrides);
  for(const [target,spec]of Object.entries(SURFACES))for(const node of document.querySelectorAll(spec.selector))for(const property of spec.properties){
    const value=overrides[target]?.[property];
    if(value===undefined)node.style.removeProperty(CSS[property]);else node.style.setProperty(CSS[property],typeof value==='number'?`${value}px`:value);
  }
}
export function auditRenderedAppearance(task,before={},propertyFilter=null){
  const current=JSON.parse(task.candidate),base=JSON.parse(task.base);
  const actualReadability=captureReadability(task.appearanceGoals);
  for(const goal of task.appearanceGoals)for(const [key,ratio]of Object.entries(actualReadability[goal.target])){const minimum=Math.min(4.5,before[goal.target]?.[key]??4.5);if(ratio+.02<minimum)throw Error(`${SURFACES[goal.target].label}: actual text contrast became unreadable.`);}
  for(const goal of task.appearanceGoals)for(const node of document.querySelectorAll(SURFACES[goal.target].selector)){
    const style=getComputedStyle(node),actualColor=rgb(style.color),actualBackground=background(node);
    if(contrast(actualColor,actualBackground)<4.5)throw Error(`${SURFACES[goal.target].label} text is not readable in the actual preview.`);
    for(const [property,value]of Object.entries(current.surfaces[goal.target])){
      if(propertyFilter&&!Object.hasOwn(propertyFilter[goal.target]||{},property))continue;
      const raw=style.getPropertyValue(CSS[property]);const actual=property==='background'?actualBackground:property==='color'?actualColor:property==='borderColor'?(rgb(raw)||'transparent'):parseFloat(raw);
      if(typeof value==='number'?Math.abs(actual-value)>.2:actual!==value)throw Error(`${SURFACES[goal.target].label}: the rendered ${property} does not match the tested configuration.`);
    }
    if(goal.intent==='match-app'&&!Object.entries(matchProfile(goal.target,base)).every(([key,value])=>current.surfaces[goal.target][key]===value))throw Error('The preview does not match the app style.');
  }
}
// Existing low-contrast decoration is not silently certified as accessible. A change
// must meet 4.5:1 for previously readable text, and cannot worsen any other text.
export function captureReadability(goals){
  const result={};
  for(const goal of goals){
    result[goal.target]={};
    for(const [rootIndex,root]of [...document.querySelectorAll(SURFACES[goal.target].selector)].entries()){
      const parentRow=root.closest('tr'),row=parentRow?`${parentRow.getAttribute('aria-label')}|${root.cellIndex}`:rootIndex;
      for(const node of [root,...root.querySelectorAll('*')]){
        const ownText=[...node.childNodes].filter(child=>child.nodeType===3).map(child=>child.textContent.trim()).join(' ');if(!ownText)continue;
        const style=getComputedStyle(node),color=rgb(style.color);if(!color)continue;
        const path=[];for(let item=node;item!==root;item=item.parentElement)path.unshift([...item.parentElement.children].indexOf(item));
        result[goal.target][`${row}|${path.join('.')}`]=contrast(color,background(node));
      }
    }
  }
  return result;
}

export function validateAppearanceState(state){
  if(!keys(state,['base','config','overrides']))throw Error('Saved appearance state is invalid.');
  const base=JSON.parse(canonicalAppearance(state.base)),config=JSON.parse(canonicalAppearance(state.config)),overrides=validateOverrides(state.overrides);
  const resolved=structuredClone(base);
  for(const [target,values]of Object.entries(overrides))Object.assign(resolved.surfaces[target],values);
  if(canonicalAppearance(resolved)!==canonicalAppearance(config))throw Error('Saved appearance settings do not match the accepted configuration.');
  for(const target of Object.keys(overrides)){const values=config.surfaces[target],bg=values.background||config.surfaces.expenseApp.background;if(contrast(values.color,bg)<4.5)throw Error('Saved appearance text is not readable.');}
  return {base,config,overrides};
}
