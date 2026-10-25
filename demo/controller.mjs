import {BASE_SOURCE,criteria,requiredCases,score,digest,extractJSON,extractCode,validatePlan,extraCases,canAccept,fixtureSource} from './core.mjs';
const wait = ms => new Promise(resolve => setTimeout(resolve,ms));
export class Controller {
  constructor({model,execute,onChange=()=>{},delay=450}) { Object.assign(this,{model,execute,onChange,delay}); this.task=null; this.busy=false; this.epoch=0; }
  emit() { this.onChange(this.task); }
  event(role,text) { this.task.events.push({role,text,at:new Date().toISOString()}); this.emit(); }
  assert(epoch) { if (epoch !== this.epoch) throw Error('Run cancelled.'); }
  async plan({request,rule,mode,base=BASE_SOURCE}) {
    if (this.busy) throw Error('A run is already active.');
    this.busy=true; const epoch=++this.epoch;
    this.task={id:crypto.randomUUID(),mode,request,rule,base,status:'Active',phase:'Planner',attempt:0,plan:null,candidate:'',hash:'',evidence:null,versions:[],events:[],createdAt:new Date().toISOString()};
    this.emit();
    try {
      const prompt=`Plan a small edit to evaluateExpense. The immutable requirements are ${JSON.stringify(criteria(rule))}. User context: ${request}. Return only JSON {"summary":"...","steps":["..."]}; maximum three steps. Do not introduce unrelated scope.`;
      const raw= mode==='guided' ? JSON.stringify({summary:'Add an expense approval policy without changing paid history.',steps:['Implement the reviewed thresholds.','Verify boundaries and paid expenses.','Present the tested candidate for human approval.']}) : await this.model.generate('Planner',prompt);
      this.assert(epoch); this.task.plan=validatePlan(extractJSON(raw)); this.task.status='Ready'; this.task.phase='Awaiting authorization';
      this.event('Planner',this.task.plan.summary);
    } catch(error) { if(epoch===this.epoch) this.block(error); throw error; }
    finally { if(epoch===this.epoch) {this.busy=false;this.emit();} }
    return this.task;
  }
  block(error) { this.task.status='Blocked'; this.task.phase='Needs attention'; this.event('Controller',String(error.message||error)); }
  async run() {
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
        const qaPrompt=`You are QA. Review these immutable requirements: ${JSON.stringify(criteria(t.rule))}. Test results: ${JSON.stringify(rows)}. Return ONLY JSON {"summary":"brief factual assessment","cases":[{"amountCents":integer,"status":"submitted or paid","receipt":boolean}]}. Propose zero to six extra data-only boundary cases. Never declare a pass unsupported by the runner. No executable code.`;
        const qa=t.mode==='guided' ? {summary:rows.every(r=>r.pass)?'Boundary and history checks passed.':'Exact threshold incorrectly requires review. Use >, not >=.',cases:[]} : extractJSON(await this.model.generate('Tester',qaPrompt));
        this.assert(epoch); const additional=extraCases(qa);
        if(additional.length) {
          try { rows.push(...score(additional,await this.execute(t.candidate,additional.map(c=>c.input)),t.rule)); }
          catch(error) {rows.push(...additional.map(c=>({...c,expected:'Valid output',actual:{error:String(error.message||error)},pass:false})));}
        }
        this.assert(epoch); t.evidence={hash:t.hash,rows,at:new Date().toISOString()};
        t.versions.push({source:t.candidate,hash:t.hash,evidence:structuredClone(t.evidence)});
        this.event('Tester',`${rows.filter(r=>r.pass).length}/${rows.length} executed checks passed. ${qa.summary.slice(0,500)}`);
        if(rows.every(r=>r.pass)) {t.status='Review';t.phase='Human approval';this.emit();return t;}
        if(attempt===2) throw Error('Two repair cycles exhausted. Review the failing evidence or start a new task.');
        this.event('Controller','Returning actual failure evidence to Coder.');
      }
    } catch(error) { if(epoch===this.epoch) this.block(error); throw error; }
    finally {if(epoch===this.epoch){this.busy=false;this.emit();}}
  }
  async accept() {
    if(this.busy || !canAccept(this.task)) throw Error('Only the exact passing candidate can be accepted.');
    const t=this.task, hash=await digest(t.candidate);
    if(t!==this.task || hash!==t.hash || !canAccept(t)) throw Error('Candidate changed; re-test before acceptance.');
    t.status='Done';t.phase='Accepted locally';this.event('Human','Accepted the tested candidate. No GitHub publication or real payment.');
    return t.candidate;
  }
  cancel() {this.epoch++;this.model.cancel();this.busy=false;if(this.task){this.task.status='Blocked';this.task.phase='Cancelled';this.event('Human','Cancelled. Late results are ignored; model worker terminated.');}}
}
