import {policy} from './core.mjs';

export const DEFAULT_REQUEST='Require manager approval for expenses above $500 and explain why. Preserve already paid expenses.';
const EXAMPLE='Try “Require manager approval for expenses above $500.” You can also add “Require receipts above $75.”';
const SCOPE='This demo supports manager approval and optional receipt rules for expenses. '+EXAMPLE;
const MANAGER=/\b(?:manager(?:'s)?\s+(?:approval|review)|approval\s+(?:from|by)\s+(?:(?:a|the)\s+)?manager)\b/gi;
const RECEIPT=/\breceipts?\b/gi;
const COMPARISON=/\b(?:above|over|more than|exceeds?|exceeding)\s+(?:USD\s*|\$)?([+-]?\d[\d,]*(?:\.\d+)?)(?:\s*(?:dollars?|USD))?/gi;
const NUMBER=/\d[\d,]*(?:\.\d+)?/g;
const POLICY_WORDS=new Set('add a an the please require requires required requiring need needs needed should must have manager review approval for all submitted expense expenses purchase purchases amount amounts that are is be to than also only'.split(' '));
const CONTEXT_WORDS=new Set('please also preserve keep leave already paid expense expenses unchanged history stay remain and explain why provide give a an clear reason reasons explanation explanations for every each result'.split(' '));
const unsupportedSymbols=text=>/[^a-z\s.,:;!?'"-]/i.test(text);
const words=text=>text.toLowerCase().replace(/[^a-z]+/g,' ').trim().split(/\s+/).filter(Boolean);

// A deliberately bounded reader: uncertain intent is rejected before any agent runs.
// The model receives the resulting immutable rules, but never chooses their values.
export function policyFromPrompt(request){
  if(typeof request!=='string'||!request.trim())throw Error('Describe the expense policy you want. '+EXAMPLE);
  if(request.length>1000)throw Error('Please keep the request under 1,000 characters.');
  const text=request.replace(/[’‘]/g,"'").replace(/[–—]/g,'-');
  if(/[%‰]/.test(text)||[...text.matchAll(/\p{Sc}/gu)].some(match=>match[0]!=='$'))throw Error('Use dollar amounts for this demo; percentages and other currencies are not supported.');
  if(/\b(?:at least|at or above|or more|or above|greater than or equal|no less than)\b|>=|≥/i.test(text))throw Error('This demo supports strictly above a limit; the exact amount stays eligible. Use “above” only if that matches the change you want.');
  if(/\bbetween\b|\bfrom\s+\$?\d[^;.!?]*\bto\s+\$?\d/i.test(text))throw Error('Ranges are not supported yet. Give one amount for manager approval and, if needed, one separate amount for receipts.');
  let approval=null,receipt=null;
  const clauses=text.split(/\s*[,;!?\n]+\s*(?!\d)|\.\s+|\band\b|\bbut\b|\bthen\b/i).filter(clause=>clause.trim());
  for(const clause of clauses){
    const managers=[...clause.matchAll(MANAGER)],receipts=[...clause.matchAll(RECEIPT)];
    const numbers=[...clause.matchAll(NUMBER)];
    if(!managers.length&&!receipts.length){
      if(numbers.length||unsupportedSymbols(clause)||words(clause).some(word=>!CONTEXT_WORDS.has(word)))throw Error(SCOPE);
      continue;
    }
    if(/\b(?:not|no|never|without|unless|except|disable|remove|don'?t)\b/i.test(clause))throw Error('This demo cannot turn rules off or apply exceptions. Describe the rule you want for all submitted expenses. '+EXAMPLE);
    if(Boolean(managers.length)===Boolean(receipts.length))throw Error('Give the manager approval rule and the receipt rule in separate sentences. '+EXAMPLE);
    const comparisons=[...clause.matchAll(COMPARISON)];
    if(comparisons.length!==1||numbers.length!==1)throw Error('Use one clear amount strictly above a limit for each rule. '+EXAMPLE);
    const amount=comparisons[0][1];
    if(!/^(?:\d{1,6}|\d{1,3}(?:,\d{3}){1,2})(?:\.\d{1,2})?$/.test(amount))throw Error('Use a dollar amount from $0.01 to $999,999.99, with at most two decimal places.');
    const remaining=clause.replace(MANAGER,'').replace(RECEIPT,'').replace(COMPARISON,'');
    if(unsupportedSymbols(remaining)||words(remaining).some(word=>!POLICY_WORDS.has(word)))throw Error(SCOPE);
    let cents;
    try{cents=policy(amount.replaceAll(',','')).thresholdCents;}
    catch{throw Error('Use a dollar amount from $0.01 to $999,999.99, with at most two decimal places.');}
    if(managers.length){
      if(approval!==null&&approval!==cents)throw Error('The request gives different manager approval amounts. Keep one clear amount.');
      approval=cents;
    }else{
      if(receipt!==null&&receipt!==cents)throw Error('The request gives different receipt amounts. Keep one clear amount.');
      receipt=cents;
    }
  }
  if(approval===null)throw Error('Include the manager approval amount in your request. '+EXAMPLE);
  return {thresholdCents:approval,receiptCents:receipt};
}
