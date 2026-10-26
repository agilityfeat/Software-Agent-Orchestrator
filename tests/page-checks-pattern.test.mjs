import test from 'node:test';
import assert from 'node:assert/strict';
import {safePattern,validateChecks} from '../demo/page-checks.mjs';
test('fixed repetition supports normal bounded formatted values',()=>{
  const pattern='^[0-9]{4}-[0-9]{2}-[0-9]{2}$';
  const date=safePattern(pattern);
  assert.equal(date.test('2026-10-25'),true);
  assert.equal(date.test('2026-1-25'),false);
  assert.ok(safePattern('^[A-Z]{2}[0-9]{6}$').test('AB123456'));
  assert.ok(safePattern('^a{100}$').test('a'.repeat(100)));
  assert.equal(validateChecks([{name:'Formatted value',assertions:[{selector:'.value',property:'text',operator:'matches',expected:pattern}]}])[0].assertions[0].expected,pattern);
});
test('bounded exact repeats cannot hide nested, ranged or excessive repetition',()=>{
  for(const pattern of ['a{101}','a{1000}','a{1,4}','a{1,}','a{-1}','a{1.5}','a{2}{3}','a{2}*','a*{2}','{2}a','(a{2})+','a{2}?','a?b?','a+a*','^'+'a?'.repeat(35)+'a'.repeat(35)+'$'])assert.throws(()=>safePattern(pattern),pattern);
});
test('escaped atoms and character classes preserve the variable repetition budget',()=>{
  assert.ok(safePattern('^\\d{4}-\\d{2}-\\d{2}$').test('2026-10-25'));
  assert.ok(safePattern('^[*+?]{3}$').test('*+?'));
  assert.ok(safePattern('^item\\?{2}$').test('item??'));
  // Each pair of backslashes is one literal atom, so both following stars count.
  assert.throws(()=>safePattern(String.raw`\\*a\\*`));
  assert.throws(()=>safePattern('\\d*\\w+'));
});
