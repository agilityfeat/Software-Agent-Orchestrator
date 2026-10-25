import {policyFromPrompt} from './prompt-policy.mjs';
import {DEFAULT_APP_REQUEST,SURFACES,canonicalAppearance,appearanceGoals,matchProfile,normalizeAppearancePatch,appearanceChecks,appearanceCriteria,extraAppearanceChecks,scoreExtraAppearance,validateOverrides} from './presentation.mjs';
import {BASE_SOURCE,EXPENSES,checkOutput,criteria,requiredCases,score,digest,extractJSON,extractCode,validatePlan,extraCases,canAccept,fixtureSource} from './core.mjs';
const wait = ms => new Promise(resolve => setTimeout(resolve,ms));
export class Controller {
  constructor({model,execute,onChange=()=>{},delay=450}) { Object.assign(this,{model,execute,onChange,delay}); this.task=null; this.busy=false; this.epoch=0; }
  emit() { this.onChange(this.task); }
  event(role,text) { this.task.events.push({role,text,at:new Date().toISOString()}); this.emit(); }
  assert(epoch) { if (epoch !== this.epoch) throw Error('Run cancelled.'); }
  async plan({request,rule,mode,base=BASE_SOURCE,appearanceBase=null,appearanceOverrides={}}) {
    if (this.busy) throw Error('A run is already active.');
    this.busy=true; const epoch=++this.epoch;
    this.task={id:crypto.randomUUID(),mode,request,rule,base,status:'Active',phase:'Planner',attempt:0,plan:null,candidate:'',hash:'',evidence:null,versions:[],events:[],createdAt:new Date().toISOString()};
    this.emit();
    try {
      if(typeof request!=='string'||!request.trim()||request.length>1000)throw Error('Describe the app improvement you want, in up to 1,000 characters.');
      const capabilities=Object.fromEntries(Object.entries(SURFACES).map(([key,value])=>[key,{label:value.label,properties:value.properties}]));
      const prompt=rule?`Plan a small edit to evaluateExpense. Immutable requirements: ${JSON.stringify(criteria(rule))}. User context: ${request}. Return only JSON {"summary":"...","steps":["..."]}; maximum three steps.`:
        `Plan this app improvement: ${request}. Supported scopes: "appearance" for colors, typography, spacing and corners on these finite surfaces: ${JSON.stringify(capabilities)}; "policy" for manager approval and optional receipt thresholds explicitly in the request. App appearance baseline: ${appearanceBase?canonicalAppearance(appearanceBase):'unavailable'}. Receipt appearance means the dialog, its close button and View receipt trigger as a consistent experience. Return ONLY JSON {"scope":"appearance","summary":"brief plan","steps":["..."],"goals":[{"target":"receiptDialog","intent":"match-app"},{"target":"receiptButton","intent":"match-app"},{"target":"dialogButton","intent":"match-app"}]}. This is a schema example, not a default task: choose the appropriate supported targets for the actual request. For matching existing style use intent "match-app"; for other appearance adjustments use "adjust". For policy use scope "policy" and omit goals. Unsupported behavior, new features or repository changes must return scope "unsupported" with a clear explanation. Never turn an appearance request into expense rules. Maximum three steps. No Markdown.`;
      let raw;
      if(mode==='guided'){
        if(!rule){try{rule=policyFromPrompt(request);}catch{if(request!==DEFAULT_APP_REQUEST)throw Error('Guided example can demonstrate the receipt appearance request or an explicit expense policy. Choose Live for your own app improvement.');}}
        raw=JSON.stringify(rule?{scope:'policy',summary:'Add an expense approval policy without changing paid history.',steps:['Implement the reviewed thresholds.','Verify boundaries and paid expenses.','Present the tested candidate for human approval.']}:{scope:'appearance',summary:'Match the receipt experience to the expense app.',steps:['Update the receipt dialog and buttons.','Check the actual style, readability and unchanged expense rules.','Preview the change for your approval.'],goals:['receiptDialog','receiptButton','dialogButton'].map(target=>({target,intent:'match-app'}))});
      }else raw=await this.model.generate('Planner',prompt);
      this.assert(epoch);const value=extractJSON(raw);
      if(value.scope==='unsupported')throw Error(String(value.summary||'This improvement is outside the editable app surfaces.').slice(0,500));
      this.task.plan=validatePlan(value);
      if(rule||value.scope==='policy'){
        this.task.kind='policy';this.task.rule=rule||policyFromPrompt(request);
      }else if(value.scope==='appearance'){
        if(!appearanceBase)throw Error('The app appearance is unavailable. Reload and try again.');
        this.task.kind='appearance';this.task.appearanceGoals=appearanceGoals(value.goals);
        this.task.base=canonicalAppearance(appearanceBase);this.task.baseOverrides=validateOverrides(appearanceOverrides);
        const outputs=await this.execute(base,EXPENSES);this.assert(epoch);
        if(outputs.length!==EXPENSES.length||!outputs.every(checkOutput))throw Error('The current expense policy cannot be verified.');
        this.task.policySnapshot={source:base,hash:await digest(base),outputs};this.assert(epoch);
      }else throw Error('Planner must choose appearance, policy, or explain an unsupported change.');
      this.task.status='Ready';this.task.phase='Awaiting authorization';
      this.event('Planner',this.task.plan.summary);
    } catch(error) { if(epoch===this.epoch) this.block(error); throw error; }
    finally { if(epoch===this.epoch) {this.busy=false;this.emit();} }
    return this.task;
  }
  block(error) { this.task.status='Blocked'; this.task.phase='Needs attention'; this.event('Controller',String(error.message||error)); }
  async run() {
    if(this.task?.kind==='appearance')return this.runAppearance();
    if(this.busy || this.task?.status!=='Ready') throw Error('Approve a ready plan first.');
    this.busy=true; const epoch=++this.epoch; const t=this.task; t.status='Active';
    try {
      for(let attempt=0;attempt<3;attempt++) {
        this.assert(epoch); t.attempt=attempt+1; t.phase='Coder'; this.emit();
        const prompt=`Implement ONLY a plain JavaScript function evaluateExpense(expense), without export, imports or Markdown. Input: {amountCents:integer,status:'submitted'|'paid',receipt:boolean}. Output: {status:'eligible'|'manager_review'|'receipt_required'|'paid',reason:nonempty string <=300 chars,canApprove:boolean}. Money is integer cents. canApprove is true ONLY when eligible. Requirements: ${JSON.stringify(criteria(t.rule))}. Exact thresholds: ${JSON.stringify(t.rule)}. Context: ${t.request}. Existing source:\n${t.candidate||t.base}\n${t.evidence ? 'Failing checks: '+JSON.stringify(t.evidence.rows.filter(r=>!r.pass)) : ''}`;
        const raw=t.mode==='guided' ? fixtureSource(t.rule,attempt===0) : await this.model.generate('Coder',prompt);
        if(t.mode==='guided') await wait(this.delay);
        this.assert(epoch); t.candidate=extractCode(raw); t.hash=await digest(t.candidate); this.assert(epoch); t.evidence=null;
        this.event('Coder',`Candidate ${t.hash.slice(0,10)} · attempt ${t.attempt}/3`);
        t.phase='Tester';this.emit();
        const cases=requiredCases(t.rule);
        let rows;
        try { rows=score(cases,await this.execute(t.candidate,cases.map(c=>c.input)),t.rule); }
        catch(error) { rows=cases.map(c=>({...c,expected:'Valid output',actual:{error:String(error.message||error)},pass:false})); }
        this.assert(epoch);
        const qaPrompt=`You are QA. Review the immutable requirements: ${JSON.stringify(criteria(t.rule))}. The controller has ALREADY EXECUTED the following ${rows.length} protected test cases. These rows are evidence to review, NOT cases to return or repeat: ${JSON.stringify(rows)}. Return ONLY one JSON object with exactly two keys: "summary" and "cases". The default response shape is {"summary":"brief factual assessment of the executed results","cases":[]}. Keep cases empty unless you identify useful ADDITIONAL boundary inputs not already tested. Never copy the protected rows into cases. cases may contain at most 6 extra data-only inputs. Each extra input must have only these fields, for example {"amountCents":50001,"status":"submitted","receipt":false}. amountCents must be an integer from 0 to 100000000, status must be "submitted" or "paid", and receipt must be a boolean; do not include names, expected results, pass flags, nested input objects, or code. Never declare a pass unsupported by the executed results. No Markdown or executable code.`;
        const qa=t.mode==='guided' ? {summary:rows.every(r=>r.pass)?'Boundary and history checks passed.':'Exact threshold incorrectly requires review. Use >, not >=.',cases:[]} : extractJSON(await this.model.generate('Tester',qaPrompt));
        this.assert(epoch); const additional=extraCases(qa);
        if(additional.length) {
          try { rows.push(...score(additional,await this.execute(t.candidate,additional.map(c=>c.input)),t.rule)); }
          catch(error) {rows.push(...additional.map(c=>({...c,expected:'Valid output',actual:{error:String(error.message||error)},pass:false})));}
        }
        this.assert(epoch); t.evidence={hash:t.hash,rows,at:new Date().toISOString()};
        t.versions.push({source:t.candidate,rawResponse:raw,hash:t.hash,evidence:structuredClone(t.evidence)});
        this.event('Tester',`${rows.filter(r=>r.pass).length}/${rows.length} executed checks passed. ${qa.summary.slice(0,500)}`);
        if(rows.every(r=>r.pass)) {t.status='Review';t.phase='Human approval';this.emit();return t;}
        if(attempt===2) throw Error('Two repair cycles exhausted. Review the failing evidence or start a new task.');
        this.event('Controller','Returning actual failure evidence to Coder.');
      }
    } catch(error) { if(epoch===this.epoch) this.block(error); throw error; }
    finally {if(epoch===this.epoch){this.busy=false;this.emit();}}
  }
  async runAppearance(){
    if(this.busy||this.task?.status!=='Ready')throw Error('Approve a ready plan first.');
    this.busy=true;const epoch=++this.epoch,t=this.task;t.status='Active';
    try{
      for(let attempt=0;attempt<3;attempt++){
        this.assert(epoch);t.attempt=attempt+1;t.phase='Coder';this.emit();
        const base=JSON.parse(t.base),profiles=Object.fromEntries(t.appearanceGoals.filter(goal=>goal.intent==='match-app').map(goal=>[goal.target,matchProfile(goal.target,base)]));
        const prompt=`Implement this reviewed app appearance request: ${t.request}. Goals: ${JSON.stringify(t.appearanceGoals)}. Exact match-app profiles (must match every listed property): ${JSON.stringify(profiles)}. Complete current baseline: ${t.base}. Only these approved surfaces and properties may change: ${JSON.stringify(Object.fromEntries(t.appearanceGoals.map(goal=>[goal.target,SURFACES[goal.target].properties])))}. Return ONLY JSON {"changes":[{"target":"receiptDialog","property":"background","value":"#ffffff"}]}. The example is a shape, not a fixed answer. Colors must be six-digit hex; borderColor may also be "transparent". Numeric properties: fontSize 9..32, borderRadius 0..24, padding 4..32, paddingBlock 4..24, all numbers without px. At most 32 unique token changes. No selectors, CSS, HTML, JavaScript, summary or Markdown. Preserve business rules, receipt content and expense data. ${t.evidence?'Actual failing checks: '+JSON.stringify(t.evidence.rows.filter(row=>!row.pass)):''}`;
        let raw;
        if(t.mode==='guided'){
          const changes=t.appearanceGoals.flatMap(goal=>Object.entries(matchProfile(goal.target,base)).map(([property,value])=>({target:goal.target,property,value})));
          if(attempt===0){const change=changes.find(change=>change.target==='receiptDialog'&&change.property==='background');if(change)change.value='#111827';}
          raw=JSON.stringify({changes});await wait(this.delay);
        }else raw=await this.model.generate('Coder',prompt);
        this.assert(epoch);t.coderResponses??=[];t.coderResponses.push({attempt:t.attempt,rawResponse:String(raw)});let rows;
        try{t.candidate=normalizeAppearancePatch(extractJSON(raw),base,t.appearanceGoals);rows=appearanceChecks(JSON.parse(t.candidate),base,t.appearanceGoals);}
        catch(error){t.candidate=String(raw).slice(0,16000);rows=[{key:'appearance:invalid',name:'Valid appearance configuration',expected:'Approved appearance tokens only',actual:{error:String(error.message||error)},pass:false}];}
        t.hash=await digest(t.candidate);this.assert(epoch);t.evidence=null;
        this.event('Coder',`Appearance candidate ${t.hash.slice(0,10)} · attempt ${t.attempt}/3`);t.phase='Tester';this.emit();
        const outputs=await this.execute(t.policySnapshot.source,EXPENSES),policyHash=await digest(t.policySnapshot.source);this.assert(epoch);
        rows.push({key:'policy:unchanged',name:'Expense rules and all six displayed results are unchanged',expected:t.policySnapshot.outputs,actual:outputs,pass:policyHash===t.policySnapshot.hash&&JSON.stringify(outputs)===JSON.stringify(t.policySnapshot.outputs)});
        const qaPrompt=`You are QA for app appearance. Reviewed requirements: ${JSON.stringify(appearanceCriteria(t))}. The host already executed these checks: ${JSON.stringify(rows)}. Review them without inventing a pass. Return ONLY JSON {"summary":"brief factual assessment","cases":[]}. cases is empty by default. If useful, propose at most six ADDITIONAL data-only checks using ONLY {"target":"receiptDialog","check":"contrast"}. target must be one of ${JSON.stringify(t.appearanceGoals.map(goal=>goal.target))}; check must be contrast, changed or match-app. Do not repeat host rows or return code/Markdown.`;
        let qa,extras;
        if(t.mode==='guided'){qa={summary:rows.every(row=>row.pass)?'Appearance and expense preservation checks passed.':'The actual preview does not yet match the app style.',cases:[]};extras=[];}
        else{
          let correction='';
          for(let qaAttempt=0;qaAttempt<2;qaAttempt++){
            const rawQa=await this.model.generate('Tester',qaPrompt+' All required checks already ran. For this review use cases:[], not copied host results. '+correction);
            this.assert(epoch);t.qaResponses??=[];t.qaResponses.push({attempt:t.attempt,qaAttempt:qaAttempt+1,rawResponse:rawQa});
            try{qa=extractJSON(rawQa);extras=extraAppearanceChecks(qa,t.appearanceGoals);break;}
            catch(error){if(qaAttempt===1)throw error;correction=`Your previous response was rejected: ${String(error.message||error)} Return ONLY {"summary":"your factual assessment of the executed checks","cases":[]}.`;this.event('Tester','QA response format was invalid; requesting one correction from the model.');}
          }
        }
        this.assert(epoch);
        if(extras.length&&rows[0].key!=='appearance:invalid')rows.push(...scoreExtraAppearance(extras,JSON.parse(t.candidate),base));
        t.evidence={hash:t.hash,rows,at:new Date().toISOString()};t.versions.push({source:t.candidate,rawResponse:raw,hash:t.hash,evidence:structuredClone(t.evidence)});
        this.event('Tester',`${rows.filter(row=>row.pass).length}/${rows.length} executed checks passed. ${qa.summary.slice(0,500)}`);
        if(rows.every(row=>row.pass)){t.status='Review';t.phase='Human approval';this.emit();return t;}
        if(attempt===2)throw Error('Two repair cycles exhausted. Review the failing checks or start a new task.');
        this.event('Controller','Returning actual appearance failures to Coder.');
      }
    }catch(error){if(epoch===this.epoch)this.block(error);throw error;}
    finally{if(epoch===this.epoch){this.busy=false;this.emit();}}
  }
  async accept() {
    if(this.busy || !canAccept(this.task))throw Error('Only the exact passing candidate can be accepted.');
    const t=this.task,epoch=this.epoch,candidate=t.candidate,expectedHash=t.hash;
    const authorization=JSON.stringify({kind:t.kind,request:t.request,base:t.base,goals:t.appearanceGoals,overrides:t.baseOverrides,snapshot:t.policySnapshot});
    const snapshot=t.policySnapshot,source=snapshot?.source,sourceHash=snapshot?.hash,expectedOutputs=JSON.stringify(snapshot?.outputs);
    const hash=await digest(candidate);
    if(t.kind==='appearance'){
      const outputs=await this.execute(source,EXPENSES),actualSourceHash=await digest(source);
      this.assert(epoch);
      if(authorization!==JSON.stringify({kind:t.kind,request:t.request,base:t.base,goals:t.appearanceGoals,overrides:t.baseOverrides,snapshot:t.policySnapshot})||t!==this.task||candidate!==t.candidate||expectedHash!==t.hash||hash!==expectedHash||!canAccept(t)||t.policySnapshot!==snapshot||snapshot.source!==source||snapshot.hash!==sourceHash||JSON.stringify(snapshot.outputs)!==expectedOutputs||actualSourceHash!==sourceHash||JSON.stringify(outputs)!==expectedOutputs)throw Error('Appearance or expense policy changed; nothing accepted.');
    }else if(t!==this.task||candidate!==t.candidate||hash!==t.hash||!canAccept(t))throw Error('Candidate changed; re-test before acceptance.');
    t.status='Done';t.phase='Accepted locally';this.event('Human','Accepted the tested candidate. No GitHub publication or real payment.');
    return candidate;
  }
  cancel() {this.epoch++;this.model.cancel();this.busy=false;if(this.task){this.task.status='Blocked';this.task.phase='Cancelled';this.event('Human','Cancelled. Late results are ignored; model worker terminated.');}}
}
