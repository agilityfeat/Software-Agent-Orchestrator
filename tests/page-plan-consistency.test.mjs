// Generic contract regressions; synthetic model responses are NON_ACCEPTANCE for inference.
import test from 'node:test';
import assert from 'node:assert/strict';
import {modelChecks,validatePlan,CHECK_SEMANTICS} from '../demo/page-contract.mjs';
import {Controller} from '../demo/page-controller.mjs';
const state=(expected,extra={})=>({name:'Declared state',selector:'#feature',property:'visible',operator:'equals',expected,...extra});
const plan=checks=>({summary:'Add an interactive feature.',steps:['Build the requested interaction.'],checks});
test('contradictory single observations need explicit scenario actions before approval',()=>{
 for(const selector of ['#feature','body','html',':root'])assert.throws(()=>modelChecks([state(false,{selector}),state(true,{selector})]),/contradict.*explicit click\/actions/);
 for(const flag of [{first:true},{all:true}])assert.throws(()=>modelChecks([state(false,{selector:'.items',...flag}),state(true,{selector:'.items',...flag})]),/contradict/);
 assert.throws(()=>modelChecks([state(1,{selector:'.items',property:'count'}),state(2,{selector:'.items',property:'count'})]),/contradict/);
});
test('different actions, properties, targets and heterogeneous any-node values remain supported',()=>{
 for(const checks of [
  [state(false),state(true,{click:'#toggle'})],
  [state(false),state(true,{actions:[{action:'click',selector:'#toggle'}]})],
  [state(false),state(true,{selector:'#other'})],
  [state(false),state(true,{property:'checked'})],
  [state(false,{selector:'.items'}),state(true,{selector:'.items'})],
  [state('Alpha',{selector:'.items',property:'text'}),state('Beta',{selector:'.items',property:'text'})],
  [state(false,{selector:'.items',first:true}),state(true,{selector:'.items',all:true})],
  [state(false),state(false)],
 ])assert.equal(modelChecks(checks).length,2);
 assert.equal(validatePlan(plan([state(true)])).checkSemantics,CHECK_SEMANTICS);
 assert.equal(CHECK_SEMANTICS,'any-node-isolated-v2');
});
test('Planner receives one explicit action correction and invalid checks never reach the browser',async()=>{
 let generations=0,executions=0,correction='';
 const controller=new Controller({model:{cancel(){},async generate(role,prompt){generations++;correction=prompt;return JSON.stringify(plan(generations===1?[state(false),state(true)]:[state(false),state(true,{click:'#toggle'})]));}},execute:async()=>{executions++;return {rows:[],errors:[]};}});
 await controller.plan({request:'Add an interactive feature.',mode:'live',base:{html:'<main>Initial app</main>',css:'',js:''}});
 assert.equal(generations,2);assert.equal(executions,1);assert.match(correction,/Supply explicit click\/actions/);assert.equal(controller.task.status,'Ready');assert.equal(controller.task.plan.checkSemantics,CHECK_SEMANTICS);
});
