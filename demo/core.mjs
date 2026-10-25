import {appearanceEvidenceValid} from './presentation.mjs';
// Deterministic contract and evidence. Models never own expected outcomes.
export const BASE_SOURCE = `function evaluateExpense(expense) {
  if (expense.status === 'paid') return {status: 'paid', reason: 'Already paid', canApprove: false};
  return {status: 'eligible', reason: 'Ready for approval', canApprove: true};
}`;
export const EXPENSES = [
  {id:'e1', merchant:'Atlas Air', person:'Alex Morgan', category:'Travel', amountCents:89000, receipt:true, status:'submitted', initials:'AA'},
  {id:'e2', merchant:'Corner Coffee', person:'Jamie Chen', category:'Meals', amountCents:4200, receipt:true, status:'submitted', initials:'CC'},
  {id:'e3', merchant:'Studio Supply', person:'Sam Rivera', category:'Equipment', amountCents:50000, receipt:false, status:'submitted', initials:'SS'},
  {id:'e4', merchant:'Harbor Hotel', person:'Alex Morgan', category:'Travel', amountCents:189000, receipt:true, status:'submitted', initials:'HH'},
  {id:'e5', merchant:'Cloud Hosting', person:'Taylor Reed', category:'Software', amountCents:250000, receipt:true, status:'paid', initials:'CH'},
  {id:'e6', merchant:'Design Tools', person:'Jamie Chen', category:'Software', amountCents:7600, receipt:false, status:'submitted', initials:'DT'},
];
export const money = cents => new Intl.NumberFormat('en-US', {style:'currency',currency:'USD'}).format(cents / 100);
export function policy(threshold, receipt = '') {
  const parse = value => {
    if (!/^\d{1,6}(\.\d{1,2})?$/.test(String(value))) throw Error('Use a positive dollar amount with at most two decimals.');
    const cents = Math.round(Number(value) * 100);
    if (cents < 1 || cents > 99999999) throw Error('Amount must be between $0.01 and $999,999.99.');
    return cents;
  };
  return {thresholdCents:parse(threshold), receiptCents:receipt === '' ? null : parse(receipt)};
}
export function expected(expense, rule) {
  if (expense.status === 'paid') return 'paid';
  if (rule.receiptCents !== null && expense.amountCents > rule.receiptCents && !expense.receipt) return 'receipt_required';
  return expense.amountCents > rule.thresholdCents ? 'manager_review' : 'eligible';
}
export function criteria(rule) {
  return [
    `Submitted expenses above ${money(rule.thresholdCents)} need manager review; exactly the threshold remains eligible.`,
    'Paid expenses stay paid, with approval disabled.',
    'Every result includes a clear explanation; approval is available only for eligible expenses.',
    ...(rule.receiptCents === null ? ['No receipt requirement is enabled. Missing receipts must not change the result: only the manager approval threshold applies, and paid expenses stay paid.'] : [`Missing receipts above ${money(rule.receiptCents)} take priority over manager review.`]),
  ];
}
export function requiredCases(rule) {
  const expense = (amountCents, extra = {}) => ({id:'test',merchant:'Synthetic merchant',amountCents,status:'submitted',receipt:true,...extra});
  const t = rule.thresholdCents;
  const cases = [
    {name:`Below threshold · ${money(t - 1)}`, input:expense(t - 1)},
    {name:`Exact boundary · ${money(t)}`, input:expense(t)},
    {name:`Above threshold · ${money(t + 1)}`, input:expense(t + 1)},
    {name:'Large submitted expense', input:expense(189000)},
    {name:'Paid expense is unchanged', input:expense(250000,{status:'paid'})},
  ];
  if (rule.receiptCents === null) {
    cases.push({name:'No receipt rule · below threshold',input:expense(t - 1,{receipt:false})},
      {name:'No receipt rule · exact boundary',input:expense(t,{receipt:false})},
      {name:'No receipt rule · above threshold',input:expense(t + 1,{receipt:false})},
      {name:'Paid without receipt stays paid',input:expense(250000,{status:'paid',receipt:false})});
  } else {
    const r = rule.receiptCents;
    cases.push({name:'Receipt boundary',input:expense(r,{receipt:false})},
      {name:'Missing receipt above boundary',input:expense(r + 1,{receipt:false})},
      {name:'Paid without receipt stays paid',input:expense(250000,{status:'paid',receipt:false})});
  }
  return cases;
}
export function checkOutput(value) {
  return value && ['eligible','manager_review','receipt_required','paid'].includes(value.status) &&
    typeof value.reason === 'string' && value.reason.trim().length > 0 && value.reason.length <= 300 &&
    typeof value.canApprove === 'boolean';
}
export function score(cases, outputs, rule) {
  if (outputs.length !== cases.length) throw Error('Runner returned an incomplete result set.');
  return cases.map((test, i) => {
    const actual = outputs[i], wanted = expected(test.input, rule);
    return {...test, expected:wanted, actual, pass:checkOutput(actual) && actual.status === wanted && actual.canApprove === (wanted === 'eligible')};
  });
}
export async function digest(source) {
  return [...new Uint8Array(await crypto.subtle.digest('SHA-256',new TextEncoder().encode(source)))].map(x=>x.toString(16).padStart(2,'0')).join('');
}
export function extractJSON(text) {
  const clean = String(text).trim().replace(/^```(?:json)?\s*/i,'').replace(/\s*```$/,'');
  return JSON.parse(clean);
}
export function extractCode(text) {
  const clean = String(text).trim().replace(/^```(?:javascript|js)?\s*/i,'').replace(/\s*```$/,'');
  if (clean.length > 16000 || !/function\s+evaluateExpense\s*\(/.test(clean)) throw Error('Coder must return one evaluateExpense function, at most 16 KB.');
  return clean;
}
export function validatePlan(value) {
  if (!value || typeof value.summary !== 'string' || !Array.isArray(value.steps) || value.steps.length < 1 || value.steps.length > 5 || value.steps.some(s=>typeof s!=='string' || s.length>400)) throw Error('Planner returned an invalid plan.');
  return {summary:value.summary.slice(0,500),steps:value.steps};
}
export function extraCases(value) {
  if(!value || typeof value.summary!=='string')throw Error('QA summary must be a string describing the executed results.');
  if(!Array.isArray(value.cases))throw Error('QA cases must be an array of additional inputs; use [] when none are needed.');
  if(value.cases.length>6)throw Error(`QA returned ${value.cases.length} additional cases; the maximum is 6. Do not repeat the protected test rows.`);
  return value.cases.map((item,i)=> {
    if (!item || !Number.isSafeInteger(item.amountCents) || item.amountCents < 0 || item.amountCents > 100000000 || !['submitted','paid'].includes(item.status) || typeof item.receipt!=='boolean') throw Error(`QA case ${i+1} must contain an integer amountCents from 0 to 100000000, status submitted or paid, and a boolean receipt.`);
    return {name:`QA case ${i+1}`,input:{id:`qa-${i}`,merchant:'Synthetic QA case',amountCents:item.amountCents,status:item.status,receipt:item.receipt}};
  });
}
export function canAccept(task) {
  if(!(task && task.mode !== 'recorded' && task.status === 'Review' && task.candidate && task.evidence?.hash === task.hash && Array.isArray(task.evidence?.rows) && task.evidence.rows.every(x=>x.pass)))return false;
  if(task.kind==='appearance')return appearanceEvidenceValid(task);
  if(!task.rule)return false;
  // Persisted evidence must cover today's immutable cases, not only an old count.
  return requiredCases(task.rule).every(test=>task.evidence.rows.some(row=>
    row.input?.amountCents===test.input.amountCents && row.input?.status===test.input.status && row.input?.receipt===test.input.receipt &&
    score([test],[row.actual],task.rule)[0].pass));
}
export function fixtureSource(rule, broken = false) {
  return `function evaluateExpense(expense) {
  if (expense.status === 'paid') return {status:'paid', reason:'Already paid', canApprove:false};
  ${rule.receiptCents === null ? '' : `if (!expense.receipt && expense.amountCents > ${rule.receiptCents}) return {status:'receipt_required', reason:'A receipt is required', canApprove:false};`}
  if (expense.amountCents ${broken ? '>=' : '>'} ${rule.thresholdCents}) return {status:'manager_review', reason:'Amount exceeds the manager approval threshold', canApprove:false};
  return {status:'eligible', reason:'Within the approval policy', canApprove:true};
}`;
}
export function baselineOutputs() {
  return EXPENSES.map(e => e.status === 'paid' ? {status:'paid',reason:'Already paid',canApprove:false} : {status:'eligible',reason:'Ready for approval',canApprove:true});
}
