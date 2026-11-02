'use strict';
const $ = selector => document.querySelector(selector);
const params = new URLSearchParams(location.search);
const icons = {
  close:'<path d="m6 6 12 12M18 6 6 18"/>',
  download:'<path d="M12 3v12m-5-5 5 5 5-5M4 16v5h16v-5"/>',
  upload:'<path d="M12 16V4m-5 5 5-5 5 5M4 16v5h16v-5"/>',
  refresh:'<path d="M20 7v5h-5M4 17v-5h5M6.1 6.1a8 8 0 0 1 13.6 4M4.3 13.9a8 8 0 0 0 13.6 4"/>',
  arrow:'<path d="M4 12h16m-6-6 6 6-6 6"/>',
  check:'<path d="m5 12 4 4L19 6"/>',
  shield:'<path d="m12 3 8 3v6c0 4-5 8-8 9-3-1-8-5-8-9V6l8-3Z"/><path d="m8 12 3 3 5-6"/>',
  rocket:'<path d="M14 4c3-2 6-2 6-2s0 3-2 6l-7 9-5-5 8-8ZM7 11l-5 1 4-5 6-1M12 17l-1 5 5-4 1-6"/><circle cx="15" cy="7" r="1.5"/><path d="M6 16c-3 0-3 5-3 5s5 0 5-3"/>',
  branch:'<circle cx="6" cy="5" r="2"/><circle cx="6" cy="19" r="2"/><circle cx="18" cy="5" r="2"/><path d="M6 7v10M18 7v2c0 5-12 1-12 6"/>',
  commit:'<circle cx="12" cy="12" r="4"/><path d="M2 12h6m8 0h6"/>',
  home:'<path d="m3 11 9-8 9 8M5 9v12h14V9M9 21v-7h6v7"/>',
  file:'<path d="M6 3h8l4 4v14H6V3Zm8 0v5h4M9 12h6M9 16h6"/>',
  compare:'<path d="M8 4v16M16 4v16M4 8l4-4 4 4m0 8 4 4 4-4"/>',
  folder:'<path d="M3 6h7l2 3h9v11H3V6Z"/>',
  tag:'<path d="M3 3h8l10 10-8 8L3 11V3Z"/><circle cx="7" cy="7" r="1"/>',
  box:'<path d="m3 7 9-4 9 4-9 4-9-4Zm0 0v10l9 4 9-4V7M12 11v10"/>',
  external:'<path d="M14 3h7v7M21 3l-11 11M10 4H4v16h16v-6"/>',
  github:'<path d="M9 19c-5 2-5-2-7-2m14 4v-4c0-1-.3-2-1-2 4 0 7-2 7-6 0-2-1-3-2-4 0-1 0-2-.4-3-2 0-3 1-4 2a14 14 0 0 0-7 0C8 3 7 2 5 2c-.5 1-.5 2-.2 3C4 6 3 7 3 9c0 4 3 6 7 6-.7 0-1 1-1 2v4"/>',
  clock:'<circle cx="12" cy="12" r="9"/><path d="M12 7v6l4 2"/>',
  connection:'<path d="m4 4 16 16M5 10a11 11 0 0 1 3-2m4-1a11 11 0 0 1 8 3M8 14a6 6 0 0 1 3-1m2 0 3 1M12 18h.01"/>',
  spark:'<path d="m12 3 2.5 6.5L21 12l-6.5 2.5L12 21l-2.5-6.5L3 12l6.5-2.5L12 3Z"/>'
};
function icon(name){return `<svg viewBox="0 0 24 24" aria-hidden="true">${icons[name]||icons.file}</svg>`}
document.querySelectorAll('[data-icon]').forEach(el => el.innerHTML = icon(el.dataset.icon));
const commits=['Refine repository navigation','Remember the selected repository','Keep local changes visible during refresh','Add worktree quick actions','Improve keyboard navigation','Handle renamed files in history','Refresh remote branch information'];
$('#commit-rows').innerHTML=commits.map((s,i)=>`<div class="commit-row"><span class="graph-cell">●</span><span>${i===0?'<span class="branch-pill">main</span>':''}${s}</span><span>Contributor</span><span>${i<2?'Today':'Yesterday'}</span></div>`).join('');
let state='available', dialogOpen=true, longNotes=false, expanded=false, timers=[], returnFocus=null;
const known=new Set([...$('#scenario').options].map(o=>o.value));
const releaseStates=new Set(['available','downloading','verifying','ready','stopping']);
const busyStates=new Set(['checking','downloading','verifying','stopping']);
const initial=known.has(params.get('state'))?params.get('state'):'available';
if(params.has('embed'))document.body.classList.add('embed');
if(params.has('compact')){$('#stage').classList.add('compact');$('#compact').setAttribute('aria-pressed','true')}
function clearTimers(){timers.forEach(clearTimeout);timers=[]}
function later(fn,ms){timers.push(setTimeout(fn,ms))}
function button(text,action,kind=''){return `<button class="button ${kind}" data-action="${action}">${text}</button>`}
function source(){return `<div class="source-line">${icon('github')}<a href="https://github.com/JasXTD/Rocket/releases" target="_blank" rel="noopener noreferrer">JasXTD/Rocket ${icon('external')}</a><span>Published on GitHub</span></div>`}
function versions(){return `<div class="version-strip" aria-label="Version comparison"><div class="version-item"><small>Installed</small><strong>0.88.0</strong></div><div class="version-arrow">${icon('arrow')}</div><div class="version-item latest"><small>Available <span class="tiny-badge">Stable</span></small><strong>0.89.0</strong></div></div>`}
function releaseNotes(){return `<div class="release-meta"><span>Windows x64</span><span class="meta-divider">·</span><span>192 MB download</span><span class="meta-divider">·</span><span>Sample release</span></div><h3 class="notes-title">What’s new</h3><ul class="release-notes"><li><span class="note-icon">${icon('spark')}</span><div><strong>A more focused update experience</strong>Review what’s changed and install when you’re ready.</div></li><li><span class="note-icon">${icon('refresh')}</span><div><strong>Smoother everyday navigation</strong>Small improvements to repository switching and refresh feedback.</div></li><li><span class="note-icon">${icon('shield')}</span><div><strong>Reliability improvements</strong>Clearer recovery guidance and fixes for compact windows.</div></li></ul><button class="release-link" data-action="notes" aria-expanded="${expanded||longNotes}">${expanded||longNotes?'Hide full release notes':'Full release notes'} ${icon('arrow')}</button>${expanded||longNotes?`<div class="expanded-notes"><h3>Additional improvements</h3><p>These are sample notes for reviewing the layout, not a published release.</p>${Array.from({length:longNotes?18:3},(_,i)=>`<p>• ${['Keep selected files visible while changing repositories.','Handle long branch names and Unicode paths such as 資料/café.rs.','Retain a clear status while a background operation is finishing.'][i%3]}</p>`).join('')}<h3>Installation</h3><p>Download now and choose when to restart. Your previous installation is retained for recovery.</p></div>`:''}`}
function header(title,subtitle,plain=false){return `<header class="dialog-head"><div class="dialog-kicker">${icon('rocket')}Rocket updates</div><button class="close-button" data-action="close" aria-label="Close update window">${icon('close')}</button>${plain?'':`<h2 id="dialog-title">${title}</h2><p class="subtitle">${subtitle}</p>`}</header>`}
function footer(hint,actions,withSource=false){return `<footer class="dialog-footer">${hint?`<div class="footer-hint">${icon(state==='ready'?'shield':'clock')}<span>${hint}</span></div>`:''}<div class="footer-actions">${withSource?`<span class="footer-source">${icon('github')}GitHub · Stable releases</span>`:''}${actions}</div></footer>`}
function symbol(kind,type=''){return `<div class="state-hero"><div class="state-symbol ${type}">${icon(kind)}</div></div>`}
function chip(){return '<div class="installed-chip">Installed <strong>0.88.0</strong><span class="dot-divider">·</span>Stable</div>'}
function simpleState(){
  const data={checking:{visual:'<div class="state-hero"><div class="spinner" aria-hidden="true"></div></div>',title:'Checking for updates',body:'Looking for the latest Rocket release on GitHub.',caption:'You can cancel and keep using Rocket.',actions:button('Cancel check','cancel')},current:{visual:symbol('check','success'),title:'You’re up to date',body:'You’re running the latest published stable version of Rocket.',actions:button('Check again','check','quiet')+button('Done','close','primary')},error:{visual:symbol('connection','warning'),title:'Couldn’t check for updates',body:'We couldn’t reach GitHub. Check your connection and try again.',details:'<details class="error-detail"><summary>Connection details</summary><p>The update server could not be reached.<br><code>Connection timed out</code><br>Your current installation is unchanged.</p></details>',actions:button('Close','close','quiet')+button('Try again','check','primary')},unavailable:{visual:symbol('box','neutral'),title:'No public release yet',body:'No public stable release was found. The repository may still be private, or its first release hasn’t been published.',actions:button('Close','close','quiet')+button('Check again','check','primary')},cancelled:{visual:symbol('check','neutral'),title:'Update cancelled',body:'Your current installation is unchanged. You can check again whenever you’re ready.',actions:button('Close','close','quiet')+button('Check again','check','primary')}}[state];
  return header('','',true)+`<div class="dialog-body center-state"><div class="state-content" role="status">${data.visual}<h3 id="dialog-title">${data.title}</h3><p>${data.body}</p>${chip()}${data.caption?`<p class="check-caption">${data.caption}</p>`:''}${data.details||''}</div></div>`+footer('',data.actions,true);
}
function progressState(){const verify=state==='verifying',stop=state==='stopping';return header(stop?'Stopping download':verify?'Verifying your update':'Downloading Rocket 0.89.0',stop?'Waiting for the download to stop.':verify?'Checking the downloaded files before installation.':'Rocket will stay open while the update downloads.')+`<div class="dialog-body"><div class="progress-box" role="status"><div class="progress-label"><span>${icon(stop?'clock':verify?'shield':'download')} ${stop?'Stopping…':verify?'Verifying files…':'Downloading update…'}</span><span class="phase-label">${verify?'Step 2 of 3':'Step 1 of 3'}</span></div><div class="progress-track"><div class="progress-fill"></div></div><ol class="phase-list"><li class="${verify?'done':'active'}"><i>${verify?'✓':'1'}</i>Download</li><li class="${verify?'active':''}"><i>2</i>Verify</li><li><i>3</i>Ready</li></ol><div class="file-line">Rocket-0.89.0-windows-x64.zip · 192 MB</div></div>${source()}</div>`+footer('Installation starts only when you choose to restart.',stop?'<button class="button" disabled>Stopping…</button>':button('Cancel download','cancel'))}
function readyState(){return header('Ready when you are','Rocket 0.89.0 has been downloaded and verified.')+`<div class="dialog-body">${versions()}<div class="ready-detail">${icon('check')}<div><strong>Everything is ready to install</strong>Your previous version will be kept for recovery.</div></div>${source()}</div>`+footer('Restart saves your drafts and closes terminal sessions.',button('Later','close','quiet')+button('Install and restart '+icon('refresh'),'restart','primary'))}
function render(){
  const active=document.activeElement,inside=$('#update-dialog').contains(active),action=inside?active.dataset.action:null;
  $('#stage').dataset.state=state;$('#scenario').value=state==='stopping'?'downloading':state;
  document.querySelectorAll('[data-scenario]').forEach(el=>el.setAttribute('aria-pressed',String(el.dataset.scenario===state)));
  $('#notice-host').innerHTML=state==='notice'?`<section class="update-notice" aria-label="Update available" aria-live="polite"><button class="close-button" data-action="dismiss" aria-label="Dismiss update notice">${icon('close')}</button><div class="notice-heading"><div class="notice-icon">${icon('download')}</div><div><h2>Rocket 0.89.0 is available</h2><p>A new stable release is ready.<br>Update whenever it suits you.</p></div></div><div class="notice-actions">${button('View update '+icon('arrow'),'view','primary')}${button('Not now','dismiss','quiet')}</div></section>`:'';
  $('#modal-layer').hidden=!dialogOpen;$('#app-content').inert=dialogOpen;
  $('#status-update').hidden=!releaseStates.has(state)&&state!=='notice';
  $('#status-update').innerHTML=state==='ready'?'Update ready <span>↗</span>':'Update available <span>↗</span>';
  if(!dialogOpen)return;
  let html;
  if(state==='available')html=header('Rocket 0.89.0 is available','A new release, ready whenever you are.')+`<div class="dialog-body">${versions()}${releaseNotes()}${source()}</div>`+footer('Download now. Choose when to restart.',button('Later','close','quiet')+button(icon('download')+' Download update','download','primary'));
  else if(state==='ready')html=readyState();
  else if(['downloading','verifying','stopping'].includes(state))html=progressState();
  else html=simpleState();
  $('#update-dialog').innerHTML=html;
  if(inside)($('#update-dialog').querySelector(`[data-action="${action}"]`)||$('#update-dialog')).focus({preventScroll:true});
}
function setState(next,{advance=false,focus=true}={}){
  clearTimers();state=next;dialogOpen=next!=='notice';expanded=false;render();
  if(dialogOpen&&focus)$('#update-dialog').focus({preventScroll:true});
  if(advance&&next==='checking')later(()=>setState('available'),1700);
  if(advance&&next==='downloading')later(()=>{setState('verifying');later(()=>setState('ready'),2200)},3200);
}
function dismiss(){clearTimers();dialogOpen=false;if(busyStates.has(state))state='cancelled';if(state==='notice')state='available';render();(returnFocus||$('#help')).focus({preventScroll:true})}
function act(action){
  if(action==='view'){returnFocus=$('#status-update');setState('available')}
  else if(action==='dismiss'||action==='close')dismiss();
  else if(action==='check')setState('checking',{advance:true});
  else if(action==='download')setState('downloading',{advance:true});
  else if(action==='cancel'){if(state==='checking')setState('cancelled');else{setState('stopping');later(()=>setState('cancelled'),500)}}
  else if(action==='notes'){expanded=!(expanded||longNotes);longNotes=false;$('#long-notes').setAttribute('aria-pressed','false');render()}
  else if(action==='restart'){dismiss();$('#demo-feedback').textContent='Preview only: Install and restart selected. No application was restarted.';$('#demo-feedback').hidden=false;later(()=>$('#demo-feedback').hidden=true,4200)}
}
document.addEventListener('click',event=>{const target=event.target.closest('[data-action]');if(target)act(target.dataset.action)});
document.querySelectorAll('[data-scenario]').forEach(el=>el.addEventListener('click',()=>{returnFocus=el;setState(el.dataset.scenario)}));
$('#scenario').addEventListener('change',()=>{returnFocus=$('#scenario');setState($('#scenario').value)});
$('#help').addEventListener('click',()=>{returnFocus=$('#help');setState('checking',{advance:true})});
$('#status-update').addEventListener('click',()=>{returnFocus=$('#status-update');setState(state==='ready'?'ready':'available')});
$('#compact').addEventListener('click',()=>{const on=$('#stage').classList.toggle('compact');$('#compact').setAttribute('aria-pressed',String(on))});
$('#long-notes').addEventListener('click',()=>{longNotes=!longNotes;$('#long-notes').setAttribute('aria-pressed',String(longNotes));if(state!=='available')setState('available');else render()});
document.addEventListener('keydown',event=>{
  if(!dialogOpen)return;
  if(event.key==='Escape'){event.preventDefault();dismiss();return}
  if(event.key!=='Tab')return;
  const nodes=[...$('#update-dialog').querySelectorAll('button:not(:disabled),a,summary')].filter(el=>el.getClientRects().length);
  const first=nodes[0],last=nodes.at(-1),active=document.activeElement;
  if(event.shiftKey&&(active===first||active===$('#update-dialog'))){event.preventDefault();last?.focus()}
  else if(!event.shiftKey&&(active===last||active===$('#update-dialog'))){event.preventDefault();first?.focus()}
});
setState(initial,{focus:false});
