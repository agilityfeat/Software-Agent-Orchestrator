import test from 'node:test';
import assert from 'node:assert/strict';
import {agentStates} from '../demo/agent-activity.mjs';

const states=task=>agentStates(task).map(({state,status})=>({state,status}));
test('agent cards describe the three steps in plain language',()=>{
  const cards=agentStates();
  assert.deepEqual(cards.map(card=>card.label),['Planning','Building','Checking']);
  assert.ok(cards.every(card=>card.state==='waiting'&&card.status==='Waiting'));
  assert.equal(new Set(cards.map(card=>card.glyph)).size,3);
});
for(const [phase,index]of ['Planner','Coder','Tester'].map((role,index)=>[role,index])){
  test(`only the actual ${phase} phase can activate its icon`,()=>{
    const cards=agentStates({status:'Active',phase,mode:'live'});
    assert.equal(cards.filter(card=>card.state==='working').length,1);
    assert.equal(cards[index].status,'Working now');
    assert.ok(cards.slice(0,index).every(card=>card.state==='finished'));
    assert.ok(cards.slice(index+1).every(card=>card.state!=='working'));
    assert.deepEqual(states({status:'Active',phase,mode:'guided'}),states({status:'Active',phase,mode:'live'}));
  });
}
test('authorization, review and acceptance never animate an agent',()=>{
  assert.deepEqual(agentStates({status:'Ready'}).map(card=>card.status),['Finished','Up next','Waiting']);
  for(const status of ['Review','Done'])assert.ok(agentStates({status}).every(card=>card.status==='Finished'));
});
test('blocked, cancelled, recorded and unknown phases cannot appear to be working',()=>{
  assert.ok(agentStates({status:'Blocked',phase:'Coder'}).every(card=>card.status==='Paused'));
  assert.ok(agentStates({status:'Blocked',phase:'Cancelled'}).every(card=>card.status==='Stopped'));
  assert.ok(agentStates({status:'Active',phase:'Planner',mode:'recorded'}).every(card=>card.status==='Recorded'));
  assert.ok(agentStates({status:'Active',phase:'Unknown'}).every(card=>card.status==='Waiting'));
});
