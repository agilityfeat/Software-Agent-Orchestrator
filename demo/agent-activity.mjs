// These states describe the controller's current work, never a separate model.
const ROLES=[{role:'Planner',label:'Planning',glyph:'M8 8h8M8 12h8M8 16h5'},
  {role:'Coder',label:'Building',glyph:'m7 16 1-4 7-7 4 4-7 7-5 1Zm6-9 4 4'},
  {role:'Tester',label:'Checking',glyph:'m6 12 4 4 8-8'}];
export function agentStates(task){
  const current=ROLES.findIndex(item=>item.role===task?.phase);
  return ROLES.map((item,index)=>{
    let state='waiting',status='Waiting';
    if(task?.mode==='recorded'){state='recorded';status='Recorded';}
    else if(task?.status==='Blocked'){state='paused';status=task.phase==='Cancelled'?'Stopped':'Paused';}
    else if(['Review','Done'].includes(task?.status)){state='finished';status='Finished';}
    else if(task?.status==='Ready'){
      if(index===0){state='finished';status='Finished';}
      else if(index===1){state='next';status='Up next';}
    } else if(task?.status==='Active'&&current>=0){
      if(index===current){state='working';status='Working now';}
      else if(index<current){state='finished';status='Finished';}
      else if(index===current+1){state='next';status='Up next';}
    }
    return {...item,state,status};
  });
}
function element(tag,className,content){
  const node=document.createElement(tag);node.className=className;
  if(content)node.textContent=content;
  return node;
}
function vector(tag,attributes){
  const node=document.createElementNS('http://www.w3.org/2000/svg',tag);
  for(const [key,value]of Object.entries(attributes))node.setAttribute(key,value);
  return node;
}
function icon(glyph){
  const frame=element('span','agent-icon');frame.setAttribute('aria-hidden','true');
  const bot=vector('svg',{viewBox:'0 0 32 32',fill:'none',stroke:'currentColor','stroke-width':'1.8','stroke-linecap':'round','stroke-linejoin':'round',class:'agent-bot'});
  bot.append(vector('path',{d:'M16 5V2m-2 0h4M6 14H3v7h3m20-7h3v7h-3'}),
    vector('rect',{x:'6',y:'8',width:'20',height:'18',rx:'6'}),
    vector('path',{d:'M11 15v3m10-3v3',class:'agent-eyes'}),
    vector('path',{d:'M12 22h8'}));
  const badge=element('span','agent-role-badge');
  const mark=vector('svg',{viewBox:'0 0 24 24',fill:'none',stroke:'currentColor','stroke-width':'2.2','stroke-linecap':'round','stroke-linejoin':'round'});
  mark.append(vector('path',{d:glyph}));badge.append(mark);frame.append(bot,badge);
  return frame;
}
export function renderAgents(container,task){
  const states=agentStates(task),signature=JSON.stringify(states);
  // Unrelated renders must not restart an icon's working animation or announcement.
  if(container.dataset.agentSignature===signature)return;
  container.dataset.agentSignature=signature;
  container.setAttribute('role','list');container.setAttribute('aria-label','Agent team');
  container.setAttribute('aria-live','polite');
  container.replaceChildren(...states.map(({role,label,glyph,state,status})=>{
    const card=element('div',`agent${state==='working'?' active':''}`);
    card.dataset.role=role;card.dataset.state=state;
    card.setAttribute('role','listitem');card.setAttribute('aria-label',`${label} agent: ${status}`);
    if(state==='working')card.setAttribute('aria-current','step');
    const copy=element('span','agent-copy'),title=element('strong','agent-name',label);
    const detail=element('span','agent-status',status);
    copy.append(title,detail);card.append(icon(glyph),copy);return card;
  }));
}
