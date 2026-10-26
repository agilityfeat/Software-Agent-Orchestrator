const props=new Set(['text','count','value','checked','attribute','visible','style']);
const operators=new Set(['includes','equals','matches','gt','gte']);
// Small regular-expression language: literal/class atoms, anchors, exact {N}
// repeats up to 100, and at most one variable repeat. No groups or backreferences.
export function safePattern(value){
  if(typeof value!=='string'||value.length>160)throw new Error('Use a short, simple check pattern.');
  let atom=false,variableRepeats=0;
  const escaped=(at)=>{
    const next=value[at+1];
    if(!next||/[1-9bBkKpP]/.test(next)||(/[a-zA-Z]/.test(next)&&!/[dDsSwWtrnfv]/.test(next)))throw new Error('Use literal characters or simple character classes in check patterns.');
    return at+1;
  };
  for(let i=0;i<value.length;i++){
    const char=value[i];
    if(char==='\\'){i=escaped(i);atom=true;continue;}
    if(char==='['){
      let end=i+1;
      for(;end<value.length;end++){if(value[end]==='\\'){end=escaped(end);continue;}if(value[end]===']')break;}
      if(end===value.length)throw new Error('Unclosed pattern character class.');
      i=end;atom=true;continue;
    }
    if(char==='('||char===')'||char==='|')throw new Error('Check patterns do not support groups or alternation.');
    if(char==='{'){
      const repeat=/^\{([0-9]{1,3})\}/.exec(value.slice(i));
      if(!atom||!repeat||Number(repeat[1])>100)throw new Error('Use exact {N} repetition with N between 0 and 100, without nesting or ranges.');
      i+=repeat[0].length-1;atom=false;continue;
    }
    if(char==='}')throw new Error('Invalid pattern repetition.');
    if(char==='*'||char==='+'||char==='?'){
      if(!atom||++variableRepeats>1)throw new Error('Patterns may contain only one variable repetition and no nested repetition.');
      atom=false;continue;
    }
    atom=char!=='^'&&char!=='$';
  }
  try{return new RegExp(value,'u');}catch{throw new Error('Invalid check pattern.');}
}
function object(x){return x&&typeof x==='object'&&!Array.isArray(x);}
function selector(x){if(typeof x!=='string'||!x.trim()||x.length>300)throw new Error('Each check needs a CSS selector.');return x;}
export function validateChecks(value){
  if(!Array.isArray(value)||!value.length||value.length>6)throw new Error('Provide between one and six browser checks.');
  return value.map(check=>{
    if(!object(check)||typeof check.name!=='string'||!check.name.trim()||check.name.length>160)throw new Error('Each browser check needs a short name.');
    const steps=check.steps??[];
    if(!Array.isArray(steps)||steps.length>8)throw new Error('Use at most eight actions in each check.');
    const normalizedSteps=steps.map(step=>{
      if(!object(step)||!['click','input','select','key','wait'].includes(step.action))throw new Error('Unknown browser action.');
      const out={action:step.action};
      if(step.action==='wait'){if(!Number.isInteger(step.ms)||step.ms<0||step.ms>1000)throw new Error('Wait actions must be between 0 and 1000 ms.');out.ms=step.ms;}
      else{out.selector=selector(step.selector);if(step.action==='input'||step.action==='select'){if(typeof step.value!=='string'||step.value.length>2000)throw new Error('Input actions need a bounded text value.');out.value=step.value;}if(step.action==='key'){if(!['Enter','Escape','Tab',' ','ArrowUp','ArrowDown','ArrowLeft','ArrowRight'].includes(step.key))throw new Error('Unsupported key.');out.key=step.key;}}
      return out;
    });
    if(!Array.isArray(check.assertions)||!check.assertions.length||check.assertions.length>12)throw new Error('Each browser check needs between one and twelve assertions.');
    const assertions=check.assertions.map(assertion=>{
      if(!object(assertion)||!props.has(assertion.property)||!operators.has(assertion.operator))throw new Error('Unknown browser assertion.');
      const out={selector:selector(assertion.selector),property:assertion.property,operator:assertion.operator,expected:assertion.expected};
      if(assertion.property==='attribute'||assertion.property==='style'){if(typeof assertion.attribute!=='string'||!/^[a-zA-Z_][\w:.-]{0,60}$/.test(assertion.attribute))throw new Error('Attribute and style checks need a property name.');out.attribute=assertion.attribute;}
      if(assertion.property==='count'){if(!Number.isInteger(assertion.expected)||assertion.expected<0||assertion.expected>10000||!['equals','gt','gte'].includes(assertion.operator))throw new Error('Count checks need a nonnegative integer.');}
      else if(assertion.property==='visible'||assertion.property==='checked'){if(typeof assertion.expected!=='boolean'||assertion.operator!=='equals')throw new Error('Boolean checks use equals true or false.');}
      else if(typeof assertion.expected!=='string'||assertion.expected.length>1000||!['equals','includes','matches'].includes(assertion.operator))throw new Error('Text checks use equals, includes or matches.');
      if(assertion.operator==='matches')safePattern(assertion.expected);
      if(assertion.all!==undefined){if(typeof assertion.all!=='boolean')throw new Error('all must be true or false.');out.all=assertion.all;}
      if(assertion.first!==undefined){if(typeof assertion.first!=='boolean')throw new Error('first must be true or false.');out.first=assertion.first;}
      if(assertion.all===true&&assertion.first===true)throw new Error('Choose all or first for a node assertion, not both.');
      return out;
    });
    return {name:check.name.trim(),steps:normalizedSteps,assertions};
  });
}
