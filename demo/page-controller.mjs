import {canonicalBundle,validateBundle,applyEdits,validatePlan,qaCases,canAccept,digest,extractJSON,CHECK_HELP,DEFAULT_REQUEST,CHECK_SEMANTICS,validateConsistentChecks} from './page-contract.mjs';
import {GUIDED_PLAN,guidedDateEdit} from './page-baseline.mjs';
export class Controller{
  constructor({model,execute,onChange=()=>{},delay=250}){Object.assign(this,{model,execute,onChange,delay});this.task=null;this.busy=false;this.epoch=0;this.abort=null;}
  emit(){this.onChange(this.task);}
  event(role,text){this.task.events.push({role,text,at:new Date().toISOString()});this.emit();}
  assert(epoch,t=this.task){if(epoch!==this.epoch||t!==this.task)throw Error('Run cancelled or replaced.');}
  begin(){this.abort?.abort();this.abort=new AbortController();this.busy=true;return ++this.epoch;}
  block(error){if(!this.task)return;this.task.status='Blocked';this.task.phase='Needs attention';this.event('Controller',String(error.message||error));}
  async plan({request,mode,base}){
    if(this.busy)throw Error('A run is already active.');if(!['live','guided'].includes(mode))throw Error('Recorded runs are read-only.');const validBase=validateBundle(base),epoch=this.begin();
    const t=this.task={id:crypto.randomUUID(),kind:'workspace',mode,model:mode==='live'?structuredClone(this.model.identity||{name:'Local model (identity unavailable)'}):{name:'Scripted Guided example'},request,base:validBase,status:'Active',phase:'Planner',attempt:0,plan:null,candidate:null,hash:'',evidence:null,versions:[],events:[],plannerResponses:[],coderResponses:[],qaResponses:[],createdAt:new Date().toISOString()};this.emit();
    try{
      if(typeof request!=='string'||!request.trim()||request.length>1000)throw Error('Describe the app change in up to 1,000 characters.');
      if(mode==='guided'&&request!==DEFAULT_REQUEST)throw Error('Guided is a scripted date-column example. Choose Live to build your own app change.');
      const files=['html','css','js'].map(file=>`--- BEGIN ${file} ---\n${t.base[file]}\n--- END ${file} ---`).join('\n');
      const prompt=`Plan this single-page app change: ${request}\nInspect the actual current files first. Reuse existing properties, data and functions; do NOT propose adding fields or logic already present. A feature may need only to expose existing data in the rendered UI. You may change any app HTML, CSS or JavaScript, add features, interactions or data. No preset categories. Return ONLY JSON {"summary":"short plan","steps":["up to three steps"],"checks":[...]}. Propose 1–2 short flat browser checks for the requested rendered outcome, not JavaScript-source/data existence. For static visible text prefer body/text/includes; avoid invented descendant chains. Freeze them for human approval before coding. ${CHECK_HELP}\nCurrent complete files (literal text):\n${files}`;
      let correction='';
      for(let attempt=0;attempt<2;attempt++){
        this.assert(epoch,t);const raw=mode==='guided'?JSON.stringify(GUIDED_PLAN):await this.model.generate('Planner',prompt+correction);this.assert(epoch,t);t.plannerResponses.push(raw);
        try{t.plan=validatePlan(extractJSON(raw));if(t.plan.checks.length>4)throw Error('Use at most four approved browser checks.');const baseline=await this.execute(t.base,t.plan.checks,{signal:this.abort.signal});this.assert(epoch,t);t.baseline=baseline;
          break;
        }catch(error){this.assert(epoch,t);if(attempt===1||mode==='guided')throw error;correction=`\nPrevious rejected plan (data only): ${raw.slice(0,6000)}\nThe response above is INVALID: ${error.message}. Return a corrected complete plan. Use visible/equals/false for initially hidden elements, and visible/equals/true with an explicit click or actions list for the shown state. A style check MUST include attribute (for example attribute:"display", expected:"none"). To verify removal, count a precise selector with expected 0; to count cells in one row use tbody tr:first-child td. There is no children property or not-includes operator. Do not copy the invalid assertion. Every interactive check must contain its own actions. Return JSON only.`;this.event('Planner','Requesting one correction to the plan’s browser checks.');}
      }
      t.planHash=await digest(JSON.stringify(t.plan));t.baseHash=await digest(canonicalBundle(t.base));this.assert(epoch,t);t.status='Ready';t.phase='Awaiting authorization';this.event('Planner',t.plan.summary);return t;
    }catch(error){if(epoch===this.epoch)this.block(error);throw error;}finally{if(epoch===this.epoch){this.busy=false;this.emit();}}
  }
  async executeCandidate(t,bundle,checks,{signal=this.abort?.signal}={}){
    const result=await this.execute(bundle,checks,{signal});
    if(!Array.isArray(result?.errors)||!Array.isArray(result.rows)||result.rows.length<checks.length||result.rows.length>checks.length+1||checks.some((check,index)=>result.rows[index]?.name!==check.name||typeof result.rows[index]?.pass!=='boolean'||JSON.stringify(result.rows[index]?.expected)!==JSON.stringify(check.assertions)))throw Error('The browser did not return every requested check. Nothing can be accepted.');
    const rows=[{name:'App loads without runtime errors',expected:'No runtime errors',actual:result.errors||[],pass:Array.isArray(result.errors)&&result.errors.length===0},...(result.rows||[])];
    const changed=canonicalBundle(bundle)!==canonicalBundle(t.base);
    rows.push({name:'App source changes from the accepted version',expected:'Changed HTML, CSS or JavaScript',actual:changed,pass:changed});
    return {rows,errors:result.errors||[],observations:result.observations||null};
  }
  async run(){
    if(this.busy||this.task?.status!=='Ready'||this.task.mode==='recorded')throw Error('Approve a ready Live or Guided plan first.');const epoch=this.begin(),t=this.task;t.status='Active';
    try{
      if(t.plan.checkSemantics!==CHECK_SEMANTICS)throw Error('Browser check rules changed. Create a new plan.');
      if(await digest(JSON.stringify(t.plan))!==t.planHash||await digest(canonicalBundle(t.base))!==t.baseHash)throw Error('The approved plan or app changed. Create a new plan.');this.assert(epoch,t);
      for(let attempt=0;attempt<3;attempt++){
        this.assert(epoch,t);t.attempt=attempt+1;t.phase='Coder';this.emit();
        const current=t.candidate||t.base;
        const files=['html','css','js'].map(file=>`--- BEGIN ${file} ---\n${current[file]}\n--- END ${file} ---`).join('\n');
        const prompt=`Make a small, exact patch to this app. Return ONLY valid JSON {"edits":[{"file":"html","find":"<h1>Overview</h1>","replace":"<h1>Dashboard</h1>"}]}. This unrelated example shows the format, not the requested change. Use the SHORTEST UNIQUE existing substring, usually one line. Aim for 1–4 small edits. For a local change NEVER copy whole arrays, functions or files. Reuse existing data; skip fields that already exist. Never emit identical find/replace. Every edit needs file, find AND replace, and all JSON must close. Match the exact approved DOM selectors and positions; never alter the checks. For a genuine whole-app rewrite, {"files":{"html":"complete replacement"}} is allowed.
Request: ${t.request}
Approved plan: ${JSON.stringify(t.plan)}
Current files (literal text, not escaped JSON):
${files}
${t.evidence?'Actual failures: '+JSON.stringify(t.evidence.rows.filter(row=>!row.pass)):''}\n${t.qaSummary?'Independent Tester observations (verify against actual failures; never change approved checks): '+t.qaSummary.slice(0,1000):''}
Return a SHORT complete JSON patch. No Markdown or explanation.`;
        const raw=t.mode==='guided'?JSON.stringify(guidedDateEdit(current)):await this.model.generate('Coder',prompt,{strategy:'fast-initial',repair:attempt>0});this.assert(epoch,t);t.coderResponses.push(raw);
        let bundle,diagnostic;
        try{bundle=applyEdits(current,extractJSON(raw));}catch(error){diagnostic=error.message;}
        if(diagnostic){t.evidence={hash:'',rows:[{name:'Apply exact source edits',expected:'Valid unique edits',actual:diagnostic,pass:false}]};this.event('Coder',diagnostic);if(attempt===2)throw Error('Two repair cycles exhausted. Review the edit failure.');continue;}
        t.candidate=bundle;t.hash=await digest(canonicalBundle(bundle));this.assert(epoch,t);
        if(t.hash===t.baseHash)throw Error('The candidate is unchanged. Nothing to accept.');
        this.event('Coder',`Full app candidate ${t.hash.slice(0,10)} · attempt ${t.attempt}/3`);t.phase='Tester';this.emit();
        let result=await this.executeCandidate(t,bundle,t.plan.checks);this.assert(epoch,t);
        t.checks=t.plan.checks;const initialChecksHash=await digest(JSON.stringify(t.checks));this.assert(epoch,t);
        t.evidence={hash:t.hash,checksHash:initialChecksHash,rows:result.rows,observations:result.observations,at:new Date().toISOString()};
        const version={bundle:structuredClone(bundle),rawResponse:raw,hash:t.hash,checks:structuredClone(t.checks),evidence:structuredClone(t.evidence)};t.versions.push(version);this.emit();
        const remaining=6-t.plan.checks.length;
        const qaPrompt=`Independently review this app change: ${t.request}. Approved plan/checks: ${JSON.stringify(t.plan)}. Complete candidate files: ${canonicalBundle(bundle)}. Actual browser result: ${JSON.stringify({...result,observations:result.observations?{text:String(result.observations.text||'').slice(0,1400),html:String(result.observations.html||'').slice(0,1400),errors:result.observations.errors}:null})}. Return ONLY {"summary":"brief factual assessment","cases":[]}. The parent executes all checks; never return pass flags or executable tests. Use cases:[] when existing failures already explain the problem. Do not repeat approved checks or copy actual results into cases. You may add at most ${remaining} useful additional browser checks, or use cases:[] when existing checks cover it. ${CHECK_HELP}`;
        let qa,extras,correction='';
        for(let qaAttempt=0;qaAttempt<2;qaAttempt++){
          this.assert(epoch,t);const response=t.mode==='guided'?JSON.stringify({summary:'The browser checked the date column and displayed dates.',cases:[]}):await this.model.generate('Tester',qaPrompt+correction);this.assert(epoch,t);t.qaResponses.push(response);
          try{qa=extractJSON(response);extras=qaCases(qa,remaining);break;}catch(error){if(qaAttempt===1)throw error;correction=`\nPrevious response was rejected: ${error.message}. Return only {"summary":"your assessment","cases":[]}. All required checks already ran; empty cases is correct. Do not repeat rows or omit mandatory assertion fields.`;this.event('Tester','Requesting one correction to QA’s check format.');}
        }
        t.qaSummary=qa.summary.slice(0,1000);
        const checks=validateConsistentChecks([...t.plan.checks,...extras]);if(extras.length)result=await this.executeCandidate(t,bundle,checks);this.assert(epoch,t);
        t.checks=checks;const checksHash=await digest(JSON.stringify(checks));this.assert(epoch,t);
        t.evidence={hash:t.hash,checksHash,rows:result.rows,observations:result.observations,at:new Date().toISOString()};version.checks=structuredClone(checks);version.evidence=structuredClone(t.evidence);
        this.event('Tester',`${result.rows.filter(row=>row.pass).length}/${result.rows.length} executed browser checks passed. ${qa.summary}`);
        if(result.rows.every(row=>row.pass)){t.status='Review';t.phase='Human approval';this.emit();return t;}
        if(attempt===2)throw Error('Two repair cycles exhausted. Review the failing browser checks.');this.event('Controller','Returning actual browser failures to Coder.');
      }
    }catch(error){if(epoch===this.epoch)this.block(error);throw error;}finally{if(epoch===this.epoch){this.busy=false;this.emit();}}
  }
  async verify(t=this.task){
    if(t.plan?.checkSemantics!==CHECK_SEMANTICS)throw Error('Browser check rules changed. Create a new plan.');
    this.abort?.abort();this.abort=new AbortController();const signal=this.abort.signal;
    const epoch=this.epoch,authorization=JSON.stringify({request:t.request,base:t.base,plan:t.plan,checks:t.checks,hash:t.hash,planHash:t.planHash,baseHash:t.baseHash,evidenceHash:t.evidence?.hash,checksHash:t.evidence?.checksHash}),candidate=canonicalBundle(t.candidate),checks=validateConsistentChecks(t.checks);
    const [hash,checksHash,planHash,baseHash]=await Promise.all([digest(candidate),digest(JSON.stringify(checks)),digest(JSON.stringify(t.plan)),digest(canonicalBundle(t.base))]);
    this.assert(epoch,t);if(hash!==t.hash||checksHash!==t.evidence?.checksHash||planHash!==t.planHash||baseHash!==t.baseHash||JSON.stringify(checks.slice(0,t.plan.checks.length))!==JSON.stringify(t.plan.checks))throw Error('Saved source or checks changed. Create a fresh plan.');
    const result=await this.executeCandidate(t,JSON.parse(candidate),checks,{signal});
    this.assert(epoch,t);if(authorization!==JSON.stringify({request:t.request,base:t.base,plan:t.plan,checks:t.checks,hash:t.hash,planHash:t.planHash,baseHash:t.baseHash,evidenceHash:t.evidence?.hash,checksHash:t.evidence?.checksHash})||candidate!==canonicalBundle(t.candidate)||result.rows.some(row=>!row.pass))throw Error('The current candidate no longer passes its reviewed browser checks.');return result;
  }
  async accept({base}={}){
    if(this.busy||!canAccept(this.task))throw Error('Only the exact passing candidate can be accepted.');const t=this.task,epoch=this.epoch,source=canonicalBundle(t.candidate),accepted=canonicalBundle(base);
    if(accepted!==canonicalBundle(t.base))throw Error('The accepted app changed. Create a fresh plan.');this.busy=true;this.emit();
    try{await this.verify(t);this.assert(epoch,t);if(!canAccept(t)||source!==canonicalBundle(t.candidate)||accepted!==canonicalBundle(base))throw Error('Candidate changed; nothing accepted.');t.status='Done';t.phase='Accepted locally';this.event('Human','Accepted the tested full app locally.');return JSON.parse(source);}finally{if(epoch===this.epoch){this.busy=false;this.emit();}}
  }
  restart(){const stopModel=this.busy||this.model.pending||!this.model.ready;this.epoch++;this.abort?.abort();this.busy=false;this.task=null;if(stopModel)this.model.cancel();this.emit();return Boolean(this.model.ready);}
  cancel(){this.epoch++;this.abort?.abort();this.model.cancel();this.busy=false;if(this.task){this.task.status='Blocked';this.task.phase='Cancelled';this.event('Human','Stopped. Late model and browser results are ignored.');}}
}
