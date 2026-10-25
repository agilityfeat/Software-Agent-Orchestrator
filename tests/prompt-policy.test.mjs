import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {DEFAULT_APP_REQUEST} from '../demo/presentation.mjs';
import {DEFAULT_REQUEST,policyFromPrompt} from '../demo/prompt-policy.mjs';

for(const [request,thresholdCents,receiptCents]of [
  [DEFAULT_REQUEST,50000,null],
  ['Require manager approval only above $500.',50000,null],
  ['Expenses above $750 need manager review. Keep paid expenses unchanged.',75000,null],
  ['Require manager approval for expenses over 600 dollars.',60000,null],
  ['Require approval from a manager for expenses more than USD 500.25.',50025,null],
  ['Expenses that exceed $1,000 need manager approval.',100000,null],
  ['Manager approval above $500 and receipts over $75.',50000,7500],
  ['Require receipts for expenses above $75.25. Expenses over $1,000.50 need manager review.',100050,7525],
  ['Manager approval above $0.01. Require receipts above $999,999.99.',1,99999999],
  ['Manager review above $500. Require manager approval above $500.',50000,null],
])test(`prompt derives explicit policy: ${request}`,()=>assert.deepEqual(policyFromPrompt(request),{thresholdCents,receiptCents}));

for(const request of [
  '', 'Add manager approval for high-value expenses.', 'Require receipts above $75.',
  'Require manager approval above $500 or $750.',
  'Manager approval above $500. Manager review above $600.',
  'Manager approval above $500. Receipts above $75. Receipts over $100.',
  'Do not require manager approval above $500.',
  'Manager approval above $500 without a receipt.',
  'Manager approval at least $500.', 'Manager approval for $500 or more.',
  'Manager approval between $500 and $1,000.', 'Manager review from $500 to $1,000.',
  'Manager approval above $500 only for travel.',
  'Manager approval above $500 and pay all paid expenses again.',
  'Require manager approval above $500 and ½.', 'Require manager approval above $500 and ².',
  'Manager approval above $500. Change the page color.',
  'Manager approval above $1k.', 'Manager approval above $1e3.',
  'Manager approval above $1,00.', 'Manager approval above $500.001.',
  'Manager approval above $0.', 'Manager approval above $-500.',
  'Manager approval above 50%.', 'Manager approval above 500¢.', 'Manager approval above $500°.',
  'Manager approval above $500+.', 'Manager approval above ¥500.',
  'Manager approval above €500.', 'Manager approval above $500 euros.',
  'Manager approval above $1,000,000.',
])test(`uncertain prompt cannot silently infer a rule: ${request}`,()=>assert.throws(()=>policyFromPrompt(request)));

test('the form contains one generic app request without policy parameter controls',async()=>{
  const html=await readFile(new URL('../demo/index.html',import.meta.url),'utf8');
  const form=html.match(/<form id="request-form">([\s\S]*?)<\/form>/)[1];
  assert.equal(form.match(/<textarea[^>]*>([^<]*)<\/textarea>/)[1],DEFAULT_APP_REQUEST);
  assert.doesNotMatch(form,/<input|rule-fields|evaluateExpense/);
});
