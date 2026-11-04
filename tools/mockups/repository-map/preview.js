/* Local, synthetic design data only. No repository, filesystem or network APIs. */
(() => {
  'use strict';
  const $ = (id) => document.getElementById(id);
  const esc = (value) => String(value).replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
  const icon = (name, cls = '') => `<svg class="icon ${cls}" aria-hidden="true"><use href="#i-${name}"/></svg>`;
  const colors = {cyan:'#22c3ee', violet:'#a78bfa', green:'#34d399', amber:'#f2b35b', teal:'#5cc6d4'};
  const uiFiles = ['crates/rocket-ui/src/app.rs', 'crates/rocket-ui/src/backend.rs', 'crates/rocket-ui/src/pages.rs'];
  const docFile = 'docs/資料/概要.md';
  const codeNodes = [
    {id:'mcp', name:'rocket-mcp', sub:'Local control server', x:35,y:95, files:14, type:'crate', path:'crates/rocket-mcp', desc:'Connects local automation to Rocket’s application and repository operations.'},
    {id:'cli', name:'rocket-cli', sub:'Command-line interface', x:35,y:280, files:9, type:'crate', path:'crates/rocket-cli', desc:'Exposes repository operations as commands and structured output.'},
    {id:'ui', name:'rocket-ui', sub:'Desktop application', x:315,y:95, files:138, type:'crate', path:'crates/rocket-ui', desc:'Coordinates repository views, background operations and the desktop workspace.'},
    {id:'integrations', name:'rocket-integrations', sub:'Extension host', x:315,y:315, files:18, type:'crate', path:'crates/rocket-integrations', desc:'Hosts reviewed integrations and their declarative interface components.'},
    {id:'git', name:'rocket-git', sub:'Repository engine', x:610,y:220, files:126, type:'crate', path:'crates/rocket-git', desc:'Runs Git operations with verification, cancellation and structured diagnostics.'},
    {id:'terminal', name:'rocket-terminal', sub:'Terminal runtime', x:610,y:45, files:12, type:'crate', path:'crates/rocket-terminal', desc:'Manages the embedded shell session, terminal rendering and process lifecycle.'},
    {id:'spell', name:'rocket-spell', sub:'Commit message helpers', x:610,y:390, files:5, type:'crate', path:'crates/rocket-spell', desc:'Provides local language helpers for commit message editing.'}
  ].map(n => ({...n,w:180,h:68,icon:'box',color:colors.cyan}));
  const codeEdges = [['mcp','ui'],['mcp','git'],['cli','git'],['ui','git'],['ui','terminal'],['ui','integrations'],['ui','spell']].map(([from,to])=>({from,to}));
  const fileNodes = [
    {id:'app-file',name:'app.rs',sub:'Application state',x:35,y:170,path:uiFiles[0],desc:'Coordinates the application and routes user actions to background work.'},
    {id:'pages-file',name:'pages.rs',sub:'View rendering',x:315,y:70,path:uiFiles[2],desc:'Renders repository views and their controls.'},
    {id:'backend-file',name:'backend.rs',sub:'Worker requests',x:315,y:270,path:uiFiles[1],desc:'Carries repository requests and results across the application boundary.'},
    {id:'repo-file',name:'repository.rs',sub:'Repository operations',x:610,y:270,path:'crates/rocket-git/src/repository.rs',desc:'Provides the repository operations used by the desktop and command-line clients.'},
    {id:'style-file',name:'style.rs',sub:'Shared UI styling',x:610,y:70,path:'crates/rocket-ui/src/style.rs',desc:'Defines common interface colors, dimensions and text styles.'},
    {id:'unicode-file',name:'概要.md',sub:'資料 / documentation',x:315,y:440,path:docFile,desc:'A sample Unicode path. The preview keeps its original spelling in navigation and details.'}
  ].map(n=>({...n,w:180,h:68,type:'file',icon:'file',color:n.id==='unicode-file'?colors.violet:colors.cyan}));
  const fileEdges = [['app-file','pages-file'],['app-file','backend-file'],['pages-file','style-file'],['backend-file','repo-file']].map(([from,to])=>({from,to}));
  const commits = [
    {id:'c9',hash:'9b4e7ac',name:'Keep refresh responsive',author:'Maya Chen',person:'maya',date:'Today · 10:42',x:45,y:25,parents:['c7'],files:[uiFiles[1],'crates/rocket-git/src/runner.rs'],refs:['main','origin/main'],lane:'main'},
    {id:'c7',hash:'7c3a2d1',name:'Merge cancel feedback',author:'Alex Kim',person:'alex',date:'Yesterday · 16:20',x:45,y:145,parents:['c5','c6'],files:[uiFiles[0],uiFiles[1]],refs:[],lane:'main'},
    {id:'c5',hash:'52de8f0',name:'Explain repository status',author:'林安',person:'lin',date:'Nov 2 · 14:06',x:45,y:265,parents:['c2'],files:[docFile,uiFiles[2]],refs:['v0.89.0'],lane:'main'},
    {id:'c2',hash:'2f8a01b',name:'Prepare shared view state',author:'Alex Kim',person:'alex',date:'Nov 1 · 11:15',x:45,y:385,parents:['c1'],files:[uiFiles[0],uiFiles[2]],refs:[],lane:'main'},
    {id:'c1',hash:'1a72b9e',name:'Set up workspace modules',author:'Alex Kim',person:'alex',date:'Oct 31 · 09:30',x:45,y:505,parents:[],files:['Cargo.toml',uiFiles[0]],refs:[],lane:'main'},
    {id:'c6',hash:'6f0b9c2',name:'Keep Stop feedback visible',author:'Maya Chen',person:'maya',date:'Nov 2 · 15:12',x:340,y:205,parents:['c4'],files:[uiFiles[0],uiFiles[1]],refs:['fix/cancel'],lane:'fix'},
    {id:'c4',hash:'4e8bc03',name:'Record cancellation steps',author:'Maya Chen',person:'maya',date:'Nov 1 · 17:32',x:340,y:325,parents:['c2'],files:['crates/rocket-git/src/runner.rs',uiFiles[1]],refs:[],lane:'fix'},
    {id:'c8',hash:'8d6a3b4',name:'Explore connected areas',author:'林安',person:'lin',date:'Yesterday · 13:10',x:635,y:85,parents:['c3'],files:[uiFiles[2],docFile],refs:['feature/map-lab'],lane:'feature'},
    {id:'c3',hash:'3cb9a21',name:'Sketch the map navigation',author:'Noah Patel',person:'noah',date:'Nov 1 · 14:43',x:635,y:325,parents:['c2'],files:[uiFiles[2],'crates/rocket-ui/src/style.rs'],refs:[],lane:'feature'}
  ].map(n=>({...n,w:235,h:65,type:'commit',icon:'history',sub:n.hash,color:n.lane==='main'?colors.cyan:n.lane==='fix'?colors.green:colors.violet}));
  const people = [
    {id:'alex',name:'Alex Kim',initials:'AK',x:55,y:35,commits:18,files:14,color:colors.cyan,desc:'Contributions across application state and repository operations.'},
    {id:'maya',name:'Maya Chen',initials:'MC',x:55,y:155,commits:14,files:11,color:colors.violet,desc:'Contributions to background work, cancellation and Git verification.'},
    {id:'noah',name:'Noah Patel',initials:'NP',x:55,y:275,commits:9,files:8,color:colors.green,desc:'Contributions to the desktop interface and terminal experience.'},
    {id:'lin',name:'林安',initials:'林',x:55,y:395,commits:7,files:6,color:colors.amber,desc:'Contributions to repository explanations, localization and documentation.'}
  ].map(n=>({...n,w:180,h:68,type:'person',icon:'people',sub:`${n.commits} commits · ${n.files} files`}));
  const areas = [
    {id:'area-ui',name:'Desktop UI',sub:'crates/rocket-ui',x:585,y:35,files:22,type:'area',path:'crates/rocket-ui',color:colors.cyan,icon:'folder',w:190,h:68},
    {id:'area-git',name:'Git backend',sub:'crates/rocket-git',x:585,y:155,files:12,type:'area',path:'crates/rocket-git',color:colors.violet,icon:'folder',w:190,h:68},
    {id:'area-terminal',name:'Terminal',sub:'crates/rocket-terminal',x:585,y:275,files:5,type:'area',path:'crates/rocket-terminal',color:colors.green,icon:'folder',w:190,h:68},
    {id:'area-docs',name:'Documentation',sub:'docs / 資料',x:585,y:395,files:8,type:'area',path:'docs/資料',color:colors.amber,icon:'folder',w:190,h:68}
  ];
  const contributions = [
    ['alex','area-ui',12],['alex','area-git',6],['maya','area-ui',6],['maya','area-git',8],['noah','area-ui',5],['noah','area-terminal',4],['lin','area-ui',2],['lin','area-docs',5]
  ].map(([from,to,weight])=>({from,to,weight}));
  const sharedEdges = [['alex','maya',3],['alex','noah',2],['alex','lin',1],['maya','noah',1],['noah','lin',2]].map(([from,to,weight])=>({from,to,weight}));
  const metadata = {
    code:{label:'Code',context:'Workspace dependencies',search:'Find a crate or file',scopeLabel:'Snapshot',scopes:[['main','main · HEAD'],['release','v0.89.0']],detailLabel:'Show',details:[['crates','Crates'],['files','Files · rocket-ui']],hint:'Select a crate to explore its dependencies.',loading:'Building the code map',loadingBody:'Reading the sample workspace manifest and linking declared dependencies.',empty:'No tracked files at this snapshot',emptyBody:'Code connections appear after the repository has tracked source files.',error:'The code map could not be read',errorBody:'The sample snapshot changed while its files were being read. Refresh to try again.'},
    history:{label:'Git history',context:'Commit ancestry',search:'Find a commit, author or path',scopeLabel:'Reachable from',scopes:[['all','All branches'],['main','main'],['feature','feature/map-lab']],detailLabel:'Layout',details:[['graph','Commit graph'],['merges','Merge connections']],hint:'Arrows follow parent → child. Scope never switches your checkout.',loading:'Reading commit connections',loadingBody:'Following parents and attaching branch labels in the sample history.',empty:'No commits yet',emptyBody:'The history map will connect commits and branches after the first commit.',error:'History could not be loaded',errorBody:'The sample refs changed during the read. Refresh to rebuild a consistent view.'},
    people:{label:'People',context:'Contributions and shared files',search:'Find a contributor or area',scopeLabel:'Period',scopes:[['month','Last 30 days'],['week','Last 7 days']],detailLabel:'Connections',details:[['areas','Changed areas'],['shared','Shared files']],hint:'Shared-file links show overlapping changes, not proof of collaboration.',loading:'Linking contributors and files',loadingBody:'Grouping the sample commit authors and their changed paths.',empty:'No contributor activity',emptyBody:'Choose a period with commits to see who changed each part of the repository.',error:'Contributor activity is unavailable',errorBody:'The sample history read was interrupted. Refresh to try again.'}
  };
  const defaults = () => ({code:{scope:'main',detail:'crates',query:'',selected:'ui'},history:{scope:'all',detail:'graph',query:'',selected:'c7'},people:{scope:'month',detail:'areas',query:'',selected:'alex'}});
  let prefs = defaults();
  let view = 'code', scenario = 'ready', detailOpen = true, compact = false;
  let graphData = {nodes:[],edges:[]}, camera={x:0,y:0,scale:1}, overview=false;
  let generation=0, pendingTimer=null, drag=null, isDragged=false, layoutFrame=null;
  let lastDialogFocus=null;
  if (new URLSearchParams(location.search).has('embed')) document.body.classList.add('embed');
  const current = () => prefs[view];
  const announce = text => { $('announcement').textContent=text; };
  const status = text => { $('status-message').textContent=text; };

  function dataset() {
    const p=current();
    let nodes,edges,labels=[];
    if(view==='code') {
      nodes=(p.detail==='crates'?codeNodes:fileNodes).map(n=>({...n}));
      edges=(p.detail==='crates'?codeEdges:fileEdges).map(e=>({...e}));
      if(p.scope==='release'&&p.detail==='crates') nodes=nodes.filter(n=>!['mcp','spell'].includes(n.id));
      labels=p.detail==='crates'?[{x:35,y:63,text:'INTERFACES'},{x:315,y:63,text:'APPLICATION'},{x:610,y:17,text:'SUPPORTING CRATES'}]:[{x:35,y:42,text:'ROCKET-UI · FILE CONNECTIONS'}];
    } else if(view==='history') {
      const allowed=p.scope==='main'?['c1','c2','c4','c5','c6','c7','c9']:p.scope==='feature'?['c1','c2','c3','c8']:commits.map(n=>n.id);
      nodes=commits.filter(n=>allowed.includes(n.id)).map(n=>({...n}));
      edges=nodes.flatMap(n=>n.parents.map(from=>({from,to:n.id,color:n.color})));
      labels=[];
    } else if(p.detail==='shared') {
      nodes=people.map((n,i)=>({...n,x:[95,565,95,565][i],y:[70,70,355,355][i]}));
      edges=sharedEdges.map(e=>({...e}));
      labels=[{x:95,y:32,text:'CONTRIBUTORS · SHARED FILES'}];
      if(p.scope==='week') nodes=nodes.filter(n=>n.id!=='lin');
    } else {
      nodes=[...people,...areas].map(n=>({...n}));
      edges=contributions.map(e=>({...e}));
      labels=[{x:55,y:10,text:'CONTRIBUTORS'},{x:585,y:10,text:'CHANGED AREAS'}];
      if(p.scope==='week') nodes=nodes.filter(n=>!['lin','area-docs'].includes(n.id));
    }
    if(view==='people'&&p.scope==='week') {
      nodes=nodes.map(n=>n.type==='person'?{...n,commits:Math.max(2,Math.round(n.commits/3)),files:Math.max(2,Math.round(n.files/2)),sub:`${Math.max(2,Math.round(n.commits/3))} commits · ${Math.max(2,Math.round(n.files/2))} files`}:{...n,files:Math.max(1,Math.round(n.files/2))});
      edges=edges.map(e=>({...e,weight:Math.max(1,Math.round(e.weight/(p.detail==='shared'?2:3)))}));
    }
    const ids=new Set(nodes.map(n=>n.id));
    edges=edges.filter(e=>ids.has(e.from)&&ids.has(e.to));
    const q=p.query.trim().toLocaleLowerCase();
    const matches=new Set(nodes.filter(n=>!q||[n.name,n.sub,n.path,n.author,n.hash,...(n.files instanceof Array?n.files:[]),...(n.refs||[])].filter(Boolean).join(' ').toLocaleLowerCase().includes(q)).map(n=>n.id));
    const related=new Set(matches);
    if(q) for(const e of edges) { if(matches.has(e.from)) related.add(e.to); if(matches.has(e.to)) related.add(e.from); }
    return {nodes,edges,labels,matches,related,query:q};
  }

  function nodeMarkup(n) {
    const p=current(), selected=n.id===p.selected;
    const selectedEdges=graphData.edges.filter(e=>e.from===p.selected||e.to===p.selected);
    const neighbors=new Set(selectedEdges.flatMap(e=>[e.from,e.to]));
    const mergeContext=view==='history'&&p.detail==='merges'&&n.parents.length<2&&!graphData.edges.some(e=>e.from===n.id&&(commits.find(c=>c.id===e.to)?.parents.length>1));
    const classes=['map-node',n.type==='commit'?'history-node':'',n.type==='person'?'person-node':'',selected?'selected':'',neighbors.has(n.id)?'related':'',graphData.query&&!graphData.related.has(n.id)||mergeContext?'dim':'',graphData.query&&graphData.matches.has(n.id)?'match':''].filter(Boolean).join(' ');
    const attrs=`class="${classes}" data-node="${n.id}" transform="translate(${n.x} ${n.y})" role="button" tabindex="0" aria-pressed="${selected}" aria-label="${esc(n.name)}${n.type==='commit'?`, ${n.hash}, ${esc(n.author)}`:''}. Show details"`;
    if(n.type==='commit') {
      return `<g ${attrs}><title>${esc(n.hash+' · '+n.name+' · '+n.author)}</title><rect class="node-bg" x="19" y="0" width="${n.w-19}" height="65" rx="6"/><circle class="commit-dot" cx="0" cy="32" r="6" stroke="${n.color}"/><text class="node-title" x="31" y="22" font-size="11">${esc(n.name)}</text><text class="node-sub" x="31" y="43">${n.hash} · ${esc(n.author)}</text>${n.refs.length?`<g transform="translate(29 -18)"><rect class="commit-ref" x="0" y="0" rx="3" width="${Math.min(210,n.refs.join(' · ').length*5.1+12)}" height="16"/><text class="ref-text" x="6" y="11">${esc(n.refs.join(' · '))}</text></g>`:''}</g>`;
    }
    const avatar=n.type==='person'?`<circle class="node-avatar" cx="29" cy="33" r="17"/><text class="avatar-label" x="29" y="38" text-anchor="middle">${n.initials}</text>`:`<rect x="13" y="17" width="30" height="30" rx="7" fill="#17354b"/><svg class="icon node-icon" x="20" y="24" width="16" height="16" color="${n.color}"><use href="#i-${n.icon}"/></svg>`;
    return `<g ${attrs}><title>${esc(n.path||n.name)}</title><rect class="node-bg" width="${n.w}" height="${n.h}" rx="8"/>${avatar}<text class="node-title" x="54" y="29">${esc(n.name)}</text><text class="node-sub" x="54" y="46">${esc(n.sub)}</text>${selected?`<circle cx="${n.w-10}" cy="10" r="2.3" fill="${colors.cyan}"/>`:''}</g>`;
  }

  function edgeMarkup(e) {
    const a=graphData.nodes.find(n=>n.id===e.from), b=graphData.nodes.find(n=>n.id===e.to);
    if(!a||!b)return '';
    const active=e.from===current().selected||e.to===current().selected;
    const dim=graphData.query&&!(graphData.matches.has(e.from)||graphData.matches.has(e.to));
    const lane=view==='history'?` lane-${b.lane}`:'';
    const weight=e.weight?` weight-${Math.min(4,Math.ceil(e.weight/(current().detail==='shared'?1:3)))}`:'';
    const cls=`map-edge${lane}${weight}${active?' active':''}${dim?' dim':''}`;
    let path, labelX, labelY;
    if(view==='history') {
      const x1=a.x,y1=a.y+32,x2=b.x,y2=b.y+32;
      path=`M${x1} ${y1} C${x1} ${(y1+y2)/2} ${x2} ${(y1+y2)/2} ${x2} ${y2+7}`;
    } else if(view==='people'&&current().detail==='shared') {
      const dx=b.x-a.x, dy=b.y-a.y;
      const x1=a.x+(Math.abs(dx)>Math.abs(dy)?(dx>0?a.w:0):a.w/2),y1=a.y+(Math.abs(dx)>Math.abs(dy)?a.h/2:(dy>0?a.h:0));
      const x2=b.x+(Math.abs(dx)>Math.abs(dy)?(dx>0?0:b.w):b.w/2),y2=b.y+(Math.abs(dx)>Math.abs(dy)?b.h/2:(dy>0?0:b.h));
      path=`M${x1} ${y1} L${x2} ${y2}`;labelX=(x1+x2)/2;labelY=(y1+y2)/2;
    } else if(Math.abs(a.x-b.x)<30) {
      const x1=a.x+a.w/2,y1=a.y+a.h,x2=b.x+b.w/2,y2=b.y;
      path=`M${x1} ${y1} C${x1} ${y1+55} ${x2} ${y2-55} ${x2} ${y2}`;
    } else {
      const x1=a.x+a.w,y1=a.y+a.h/2,x2=b.x,y2=b.y+b.h/2;
      const mid=(x1+x2)/2;
      path=`M${x1} ${y1} C${mid} ${y1} ${mid} ${y2} ${x2} ${y2}`;
      labelX=mid;labelY=(y1+y2)/2;
    }
    const edge=`<path class="${cls}" d="${path}"${view!=='people'?' marker-end="url(#arrow)"':''}/>`;
    return edge+(e.weight&&active?`<g class="edge-weight"><rect class="edge-bubble" x="${labelX-16}" y="${labelY-8}" width="32" height="16" rx="5"/><text class="edge-label" x="${labelX}" y="${labelY+3}" text-anchor="middle">${e.weight}</text></g>`:'');
  }

  function renderGraph(refit=false) {
    graphData=dataset();
    if(!graphData.nodes.some(n=>n.id===current().selected))current().selected=graphData.nodes[0]?.id||null;
    $('graph-world').innerHTML=graphData.labels.map(l=>`<text class="canvas-section-label" x="${l.x}" y="${l.y}">${l.text}</text>`).join('')+graphData.edges.map(edgeMarkup).join('')+graphData.nodes.map(nodeMarkup).join('');
    $('graph').setAttribute('aria-label',`Interactive ${metadata[view].label} map. Select a node for details. Drag empty space to pan; scroll to zoom; F fits the map.`);
    const p=current();
    $('graph-eyebrow').textContent=view==='code'?(p.detail==='crates'?'WORKSPACE CRATES':'FILES · ROCKET-UI'):view==='history'?'COMMIT ANCESTRY':p.detail==='shared'?'SHARED FILES':'CONTRIBUTIONS';
    $('graph-badge').textContent='Sample · '+(view==='code'?(p.scope==='main'?'main':'v0.89.0'):view==='history'?(p.scope==='all'?'3 branches':p.scope==='main'?'main':'feature/map-lab'):p.scope==='month'?'30 days':'7 days');
    const copy=view==='code'?`<strong>${p.detail==='crates'?'Declared dependencies':'Illustrative import connections'}</strong> · arrows point to the dependency`:view==='history'?`<strong>${p.detail==='merges'?'Merge context highlighted':'Read-only ancestry'}</strong> · parent → child`:`<strong>${p.detail==='shared'?'Files touched by both authors':'Commits touching each area'}</strong> · ${p.scope==='month'?'last 30':'last 7'} days`;
    $('context-copy').innerHTML=copy;
    $('context-count').textContent=graphData.query?`${graphData.matches.size} matches · context retained`:`${graphData.nodes.length} ${view==='history'?'commits':'nodes'} · ${graphData.edges.length} links`;
    $('legend').innerHTML=view==='code'?'<span><i class="directed"></i>Depends on</span><span><b></b>Selected connection</span>':view==='history'?'<span><i class="directed"></i>Parent → child</span><span><b></b>main <b class="violet"></b>feature</span>':`<span><i></i>${p.detail==='shared'?'Shared changed files':'Changed area'}</span><span><i></i>Thicker = ${p.detail==='shared'?'more files':'more commits'}</span>`;
    $('map-hint-text').textContent=view==='code'&&p.detail==='files'?'Illustrative file connections; unsupported languages would remain unlinked.':metadata[view].hint;
    renderInspector();
    renderState();
    if(refit)requestAnimationFrame(layoutMap);
    else updateCamera();
  }

  function bounds() {
    const ns=graphData.nodes;
    return {left:Math.min(...ns.map(n=>n.x))-15,top:Math.min(...ns.map(n=>n.y))-30,right:Math.max(...ns.map(n=>n.x+n.w))+15,bottom:Math.max(...ns.map(n=>n.y+n.h))+15};
  }
  function fitMap() {overview=true;layoutMap();}
  function layoutMap() {
    if(!graphData.nodes.length)return;
    const r=$('graph').getBoundingClientRect(), b=bounds();
    if(r.width<1||r.height<1)return;
    const sx=Math.max(80,r.width-38)/(b.right-b.left),sy=Math.max(60,r.height-110)/(b.bottom-b.top);
    camera.scale=Math.max(.14,Math.min(sx,sy,1.2));
    if(compact&&!overview&&camera.scale<.8) {focusNode();return;}
    camera.x=(r.width-(b.right-b.left)*camera.scale)/2-b.left*camera.scale;
    camera.y=48+(r.height-108-(b.bottom-b.top)*camera.scale)/2-b.top*camera.scale;
    updateCamera();
  }
  function focusNode() {
    const n=graphData.nodes.find(n=>n.id===current().selected)||graphData.nodes[0];
    if(!n)return;
    overview=false;
    const r=$('graph').getBoundingClientRect();camera.scale=Math.max(.9,Math.min(1.1,camera.scale));
    camera.x=r.width/2-(n.x+n.w/2)*camera.scale;
    camera.y=48+(r.height-108)/2-(n.y+n.h/2)*camera.scale;
    updateCamera();
  }
  function updateCamera() {
    $('graph-world').setAttribute('transform',`translate(${camera.x.toFixed(2)} ${camera.y.toFixed(2)}) scale(${camera.scale.toFixed(4)})`);
    $('zoom-label').textContent=`${Math.round(camera.scale*100)}%`;
    $('zoom-out').disabled=camera.scale<=.1401;
    $('zoom-in').disabled=camera.scale>=2.5;
    $('focus-node').disabled=!current().selected;
    if(compact)$('map-hint-text').textContent=overview?'Overview · use Focus or zoom in to read a node.':'Drag to explore · Fit shows the whole map.';
  }
  function zoom(factor,cx,cy) {
    const rect=$('graph').getBoundingClientRect();
    const px=cx??rect.width/2,py=cy??rect.height/2;
    const next=Math.max(.14,Math.min(2.5,camera.scale*factor)),ratio=next/camera.scale;
    camera.x=px-(px-camera.x)*ratio;camera.y=py-(py-camera.y)*ratio;camera.scale=next;updateCamera();
  }
  function selectNode(id,open=true) {
    if(scenario!=='ready'||!graphData.nodes.some(n=>n.id===id))return;
    current().selected=id;
    if(open) {detailOpen=true;syncDetails();}
    renderGraph();
    if(compact&&!overview)focusNode();
    const n=graphData.nodes.find(n=>n.id===id);
    announce(`${n.name} selected. Details updated.`);
  }
  function relatedButton(n,extra='') {
    return `<button data-select="${n.id}">${icon(n.icon)}<span class="related-name">${esc(n.name)}</span>${extra?`<small>${esc(extra)}</small>`:''}${icon('chevron')}</button>`;
  }
  function action(label,name,action,data='') {
    return `<button data-action="${action}"${data?` data-value="${esc(data)}"`:''}>${icon(name)}${label}${icon('arrow')}</button>`;
  }
  function evidenceNote(text) {return `<div class="evidence-note">${text}</div>`;}
  function renderInspector() {
    const n=graphData.nodes.find(n=>n.id===current().selected);
    if(!n||scenario==='empty') {
      $('inspector-content').innerHTML=`<div class="inspector-placeholder">${icon('map')}<p>${scenario==='empty'?'Details appear when the map has data.':'Select a node to see its connections and supporting records.'}</p></div>`;return;
    }
    const heading=`<div class="selection-icon">${n.type==='person'?`<span class="initials">${n.initials}</span>`:icon(n.icon)}<div><span class="eyebrow">${n.type==='commit'?'COMMIT':n.type==='person'?'CONTRIBUTOR':n.type==='area'?'CHANGED AREA':n.type==='file'?'SOURCE / FILE':'WORKSPACE CRATE'}</span><h2>${esc(n.type==='commit'?n.hash:n.name)}</h2></div></div>`;
    let html=heading;
    if(view==='code') {
      const outgoing=graphData.edges.filter(e=>e.from===n.id).map(e=>graphData.nodes.find(v=>v.id===e.to));
      const incoming=graphData.edges.filter(e=>e.to===n.id).map(e=>graphData.nodes.find(v=>v.id===e.from));
      html+=`<div class="inspector-path">${esc(n.path)}</div><p class="inspector-description">${esc(n.desc)}</p><div class="inspector-stats"><div><strong>${outgoing.length}</strong><span>direct dependencies</span></div><div><strong>${incoming.length}</strong><span>direct consumers</span></div></div>`;
      html+=`<div class="inspector-actions">${n.id==='ui'?action('Explore files','folder','files'):''}${action('View history','history','history-path',n.path)}${action('See contributors','people','people-path',n.id)}</div>`;
      html+=`<div class="inspector-section"><div class="section-heading"><h3>Depends on</h3><span>${outgoing.length} direct</span></div><div class="related-list">${outgoing.map(v=>relatedButton(v)).join('')||'<p class="inspector-description">No dependencies in this sample.</p>'}</div></div>`;
      if(incoming.length)html+=`<div class="inspector-section"><h3>Used by</h3><div class="related-list">${incoming.map(v=>relatedButton(v)).join('')}</div></div>`;
      html+=evidenceNote('<strong>Connection source</strong><br>Sample manifest / import records. Containment and dependencies are separate. File counts and links are illustrative.');
    } else if(view==='history') {
      html+=`<p class="inspector-description">${esc(n.name)}</p><dl class="detail-fields"><div><dt>Author</dt><dd>${esc(n.author)}</dd></div><div><dt>Committed</dt><dd>${n.date}</dd></div><div><dt>Parents</dt><dd>${n.parents.length||'Root commit'}</dd></div><div><dt>Refs</dt><dd>${esc(n.refs.join(', ')||'No branch or tag tip')}</dd></div></dl>`;
      html+=`<div class="inspector-actions">${action('Inspect changes','file','commit-evidence',n.id)}${action('Locate in Code','code','locate-code',n.files[0])}${action('View contributor','people','person',n.person)}</div>`;
      html+=`<div class="inspector-section"><h3>${n.parents.length>1?'Merge parents':'Parent commit'}</h3><div class="related-list">${n.parents.map(id=>commits.find(c=>c.id===id)).map(c=>relatedButton(c,c.hash)).join('')||'<p class="inspector-description">This is the first sample commit.</p>'}</div></div>`;
      html+=`<div class="inspector-section"><div class="section-heading"><h3>Changed files</h3><span>${n.files.length}</span></div><div class="related-list">${n.files.map(path=>`<button data-action="file-evidence" data-value="${esc(path)}">${icon('file')}<span class="related-name" title="${esc(path)}">${esc(path.split('/').pop())}</span>${icon('chevron')}</button>`).join('')}</div></div>`;
      html+=evidenceNote('<strong>Git facts only</strong><br>Parent IDs define these connections. Viewing a branch does not switch or modify the checkout.');
    } else if(n.type==='person') {
      const edges=graphData.edges.filter(e=>e.from===n.id||e.to===n.id);
      html+=`<p class="inspector-description">${esc(n.desc)}</p><div class="inspector-stats"><div><strong>${n.commits}</strong><span>commits in period</span></div><div><strong>${n.files}</strong><span>files changed</span></div></div>`;
      html+=`<div class="inspector-actions">${action('Show commits','history','person-history',n.name)}${action('Inspect changed files','file','person-evidence',n.id)}</div>`;
      html+=`<div class="inspector-section"><div class="section-heading"><h3>${current().detail==='shared'?'Shared work areas':'Changed areas'}</h3><span>${edges.length} connections</span></div><div class="related-list">${edges.map(e=>relatedButton(graphData.nodes.find(v=>v.id===(e.from===n.id?e.to:e.from)),`${e.weight} ${current().detail==='shared'?'files':'commits'}`)).join('')}</div></div>`;
      html+=`<div class="inspector-section"><h3>Recent sample commits</h3>${commits.filter(c=>c.person===n.id).slice(0,2).map(c=>`<div class="mini-commit"><code>${c.hash}</code><span>${esc(c.name)}</span></div>`).join('')}</div>`;
      html+=evidenceNote('<strong>Identity & interpretation</strong><br>Author aliases would use the repository’s .mailmap. Shared files indicate overlapping changes, not direct collaboration or a performance score.');
    } else {
      const edges=graphData.edges.filter(e=>e.to===n.id);
      html+=`<div class="inspector-path">${esc(n.path)}</div><p class="inspector-description">People who changed files in this area during the selected period.</p><div class="inspector-stats"><div><strong>${edges.length}</strong><span>contributors</span></div><div><strong>${n.files}</strong><span>changed files</span></div></div><div class="inspector-section"><h3>Contributors</h3><div class="related-list">${edges.map(e=>relatedButton(graphData.nodes.find(v=>v.id===e.from),`${e.weight} commits`)).join('')}</div></div><div class="inspector-actions">${action('View area history','history','history-path',n.path)}${action('Locate in Code','code','locate-code',n.path)}</div>`;
      html+=evidenceNote('Counts describe the synthetic sample only. A commit that touches two areas can appear in both.');
    }
    $('inspector-content').innerHTML=html;
    $('inspector-content').inert=scenario!=='ready';
  }

  function syncDetails() {
    $('graph-and-details').classList.toggle('details-hidden',!detailOpen);
    $('map-panel').classList.toggle('drawer-mode',compact);
    $('toggle-details').setAttribute('aria-expanded',String(detailOpen));
    $('drawer-backdrop').hidden=!(compact&&detailOpen);
    $('inspector').setAttribute('aria-hidden',String(!detailOpen));
    $('inspector').inert=!detailOpen;
  }
  function closeDetails(restore=false) {
    detailOpen=false;syncDetails();
    if(!compact)requestAnimationFrame(layoutMap);
    if(restore)$('toggle-details').focus();
  }
  function setView(next,{preserveQuery=true}={}) {
    if(!metadata[next])return;
    cancelPending();
    view=next;
    overview=false;
    if(!preserveQuery)current().query='';
    const m=metadata[view];
    document.querySelectorAll('[data-view]').forEach(button=>{const active=button.dataset.view===view;button.setAttribute('aria-selected',String(active));button.tabIndex=active?0:-1;});
    $('map-panel').setAttribute('aria-labelledby',`tab-${view}`);
    $('view-context').textContent=m.context;
    $('map-search').value=current().query;
    $('map-search').placeholder=m.search;
    $('clear-search').hidden=!current().query;
    $('scope-label').textContent=m.scopeLabel;$('detail-label').textContent=m.detailLabel;
    $('map-scope').innerHTML=m.scopes.map(([value,label])=>`<option value="${value}">${label}</option>`).join('');
    $('map-detail').innerHTML=m.details.map(([value,label])=>`<option value="${value}">${label}</option>`).join('');
    $('map-scope').value=current().scope;$('map-detail').value=current().detail;
    if(compact)detailOpen=false;
    syncDetails();renderGraph(true);
    status(`Sample ${m.label.toLocaleLowerCase()} map · no repository connected`);
  }
  function cancelPending() {generation++;if(pendingTimer!==null){clearTimeout(pendingTimer);pendingTimer=null;}}
  function setScenario(next) {
    cancelPending();scenario=next;
    $('scenario').value=next;$('app').dataset.state=next;
    if(compact&&next!=='ready')closeDetails();
    renderInspector();renderState();
    if(next==='ready')status('Sample data · no repository connected');
    else if(next==='stopped')status('Stopped · previous map retained · no repository changed');
    else if(next==='loading')status('Building sample map · Stop is available');
    else if(next==='empty')status('Empty sample repository · no data to display');
    else status('Sample read failed · details and retry are available');
  }
  function simulateRefresh() {
    setScenario('loading');
    const ticket=generation;
    pendingTimer=setTimeout(()=>{if(ticket!==generation)return;pendingTimer=null;setScenario('ready');renderGraph(true);announce('Sample map refreshed.');},1200);
  }
  function renderState() {
    const m=metadata[view], layer=$('state-layer');
    const noResults=scenario==='ready'&&graphData.query&&!graphData.matches.size;
    $('context-count').textContent=scenario==='empty'?`0 ${view==='history'?'commits':'nodes'} · 0 links`:scenario==='loading'?'Previous map retained':graphData.query?`${graphData.matches.size} matches · context retained`:`${graphData.nodes.length} ${view==='history'?'commits':'nodes'} · ${graphData.edges.length} links`;
    $('refresh-map').disabled=scenario==='loading';
    $('graph').inert=scenario!=='ready'||noResults;
    if(scenario==='ready'&&!noResults){layer.hidden=true;layer.innerHTML='';return;}
    layer.hidden=false;layer.className=`state-layer state-${scenario}${noResults?' no-results':''}`;
    let heading,body,kind,actions,extra='';
    if(noResults){kind='search';heading='No matching nodes';body=`No names, paths or sample records match “${current().query}”. The current map is kept behind this search.`;actions='<button class="primary" data-state-action="clear-search">Clear search</button>';}
    else if(scenario==='loading') {kind='map';heading=m.loading;body=m.loadingBody;actions=`<button data-state-action="stop">${icon('stop')}Stop</button><button class="quiet" data-state-action="finish">Finish simulation</button>`;extra='<div class="state-progress"><span></span></div><span class="state-footnote">Preview simulation · no files are being read</span>';}
    else if(scenario==='empty') {kind=view==='people'?'people':view==='history'?'history':'folder';heading=m.empty;body=m.emptyBody;actions='<button class="primary" data-state-action="sample">Load sample repository</button>';}
    else if(scenario==='error') {kind='warn';heading=m.error;body=m.errorBody;actions=`<button class="primary" data-state-action="retry">${icon('refresh')}Retry</button><button data-state-action="error-details">View details</button>`;extra='<span class="state-code">MAP_SNAPSHOT_CHANGED · simulated</span>';}
    else {kind='stop';heading='Map loading stopped';body='Your previous map is still available. No repository files or checkout state were changed.';actions='<button class="primary" data-state-action="previous">Show previous map</button><button data-state-action="retry">Retry</button>';}
    layer.innerHTML=`<div class="state-card"><div class="state-icon">${icon(kind)}</div><h2>${esc(heading)}</h2><p>${esc(body)}</p>${extra}<div class="state-actions">${actions}</div></div>`;
  }

  function openEvidence(title,body) {
    lastDialogFocus=document.activeElement;
    $('dialog-title').textContent=title;$('dialog-body').innerHTML=body;
    $('evidence-dialog').showModal();$('close-dialog').focus();
  }
  function fileEvidence(paths) {
    return `<div class="file-evidence">${paths.map((path,i)=>`<div>${icon('file')}<span>${esc(path)}</span><small>+${[18,7,12,4][i%4]} −${[4,2,3,1][i%4]}</small></div>`).join('')}</div>`;
  }
  function closeEvidence() {$('evidence-dialog').close();if(lastDialogFocus?.isConnected)lastDialogFocus.focus();}
  function handleAction(action,value) {
    if(scenario!=='ready')return;
    if(action==='files'){current().detail='files';current().selected='app-file';current().query='';setView('code');}
    else if(action==='history-path'){prefs.history.query=value;prefs.history.scope='all';setView('history');}
    else if(action==='person-history'){prefs.history.query=value;prefs.history.scope='all';setView('history');}
    else if(action==='people-path'){
      const areasByNode={ui:'area-ui','app-file':'area-ui','pages-file':'area-ui','backend-file':'area-ui','style-file':'area-ui',git:'area-git','repo-file':'area-git',terminal:'area-terminal','unicode-file':'area-docs'};
      const area=areasByNode[value];
      prefs.people.detail='areas';prefs.people.query=area?'':codeNodes.find(n=>n.id===value)?.path||value;prefs.people.scope='month';prefs.people.selected=area||'alex';setView('people');detailOpen=Boolean(area);syncDetails();
    } else if(action==='person'){prefs.people.query='';prefs.people.scope='month';prefs.people.detail='areas';prefs.people.selected=value;setView('people');detailOpen=true;syncDetails();}
    else if(action==='locate-code'){
      prefs.code.scope='main';prefs.code.query='';
      const file=fileNodes.find(n=>n.path===value),crate=codeNodes.find(n=>n.path===value||value.startsWith(n.path+'/'));
      prefs.code.detail=file?'files':'crates';prefs.code.selected=file?.id||crate?.id||'ui';setView('code');detailOpen=true;syncDetails();
    } else if(action==='commit-evidence'){
      const c=commits.find(c=>c.id===value);openEvidence(c.name,`<p class="dialog-intro">${c.hash} · ${esc(c.author)} · ${c.date}<br>Changed paths in this sample commit.</p>${fileEvidence(c.files)}`);
    } else if(action==='file-evidence'){
      openEvidence(value.split('/').pop(),`<p class="dialog-intro">Selected path, preserved as Unicode. A real implementation would open the existing commit diff viewer.</p>${fileEvidence([value])}`);
    } else if(action==='person-evidence'){
      const person=people.find(n=>n.id===value),files=[...new Set(commits.filter(c=>c.person===value).flatMap(c=>c.files))];
      openEvidence(`${person.name} · changed files`,`<p class="dialog-intro">Evidence from the visible sample commits. Shared-file relationships do not imply that two people worked together directly.</p>${fileEvidence(files)}`);
    }
  }

  document.querySelectorAll('[data-view]').forEach(button=>button.addEventListener('click',()=>setView(button.dataset.view)));
  document.querySelector('.view-tabs').addEventListener('keydown',event=>{
    const views=['code','history','people'];let next=views.indexOf(view);
    if(event.key==='ArrowRight')next=(next+1)%3;else if(event.key==='ArrowLeft')next=(next+2)%3;else if(event.key==='Home')next=0;else if(event.key==='End')next=2;else return;
    event.preventDefault();setView(views[next]);$(`tab-${views[next]}`).focus();
  });
  $('map-search').addEventListener('input',()=>{
    current().query=$('map-search').value;$('clear-search').hidden=!current().query;renderGraph();
    if(compact&&graphData.query&&graphData.matches.size){current().selected=[...graphData.matches][0];renderGraph();focusNode();}
  });
  function clearSearch() {current().query='';$('map-search').value='';$('clear-search').hidden=true;renderGraph();$('map-search').focus();}
  $('clear-search').addEventListener('click',clearSearch);
  $('map-scope').addEventListener('change',()=>{current().scope=$('map-scope').value;renderGraph(true);announce('Map scope changed. Checkout is unchanged.');});
  $('map-detail').addEventListener('change',()=>{current().detail=$('map-detail').value;current().query='';$('map-search').value='';$('clear-search').hidden=true;renderGraph(true);});
  $('toggle-details').addEventListener('click',()=>{detailOpen=!detailOpen;syncDetails();if(!compact)requestAnimationFrame(layoutMap);});
  $('close-details').addEventListener('click',()=>closeDetails(true));
  $('drawer-backdrop').addEventListener('click',()=>closeDetails(true));
  $('inspector-content').addEventListener('click',event=>{
    const select=event.target.closest('[data-select]');if(select){selectNode(select.dataset.select);return;}
    const action=event.target.closest('[data-action]');if(action)handleAction(action.dataset.action,action.dataset.value);
  });
  $('graph-world').addEventListener('click',event=>{if(isDragged){isDragged=false;return;}const node=event.target.closest('[data-node]');if(node)selectNode(node.dataset.node);});
  $('graph-world').addEventListener('keydown',event=>{const node=event.target.closest('[data-node]');if(node&&['Enter',' '].includes(event.key)){event.preventDefault();selectNode(node.dataset.node);requestAnimationFrame(()=>document.querySelector(`[data-node="${node.dataset.node}"]`)?.focus());}});
  $('graph-world').addEventListener('focusin',event=>{
    const node=event.target.closest('[data-node]');if(!node||!node.matches(':focus-visible'))return;
    const r=node.getBoundingClientRect(),canvas=$('graph').getBoundingClientRect();
    if(r.left<canvas.left||r.right>canvas.right||r.top<canvas.top+40||r.bottom>canvas.bottom-50){current().selected=node.dataset.node;focusNode();renderInspector();}
  });
  $('graph').addEventListener('pointerdown',event=>{
    isDragged=false;
    if(event.button!==0||event.target.closest('[data-node]')||scenario!=='ready')return;
    drag={id:event.pointerId,x:event.clientX,y:event.clientY,originX:camera.x,originY:camera.y};isDragged=false;$('graph').setPointerCapture(event.pointerId);$('graph').classList.add('dragging');
  });
  $('graph').addEventListener('pointermove',event=>{if(!drag||event.pointerId!==drag.id)return;const dx=event.clientX-drag.x,dy=event.clientY-drag.y;if(Math.abs(dx)+Math.abs(dy)>3)isDragged=true;camera.x=drag.originX+dx;camera.y=drag.originY+dy;updateCamera();});
  function endDrag(event){if(!drag||event.pointerId!==drag.id)return;if($('graph').hasPointerCapture(drag.id))$('graph').releasePointerCapture(drag.id);drag=null;$('graph').classList.remove('dragging');}
  $('graph').addEventListener('pointerup',endDrag);$('graph').addEventListener('pointercancel',endDrag);
  $('graph').addEventListener('wheel',event=>{if(scenario!=='ready')return;event.preventDefault();const r=$('graph').getBoundingClientRect();zoom(Math.exp(-event.deltaY*.0015),event.clientX-r.left,event.clientY-r.top);},{passive:false});
  $('graph').addEventListener('keydown',event=>{
    if(event.key==='Escape'){closeDetails(true);return;}
    if(['f','F'].includes(event.key)){event.preventDefault();fitMap();return;}
    if(['+','='].includes(event.key)){event.preventDefault();zoom(1.2);return;}
    if(event.key==='-'){event.preventDefault();zoom(1/1.2);return;}
    if(!event.target.closest('[data-node]')&&event.key.startsWith('Arrow')){event.preventDefault();camera.x+=event.key==='ArrowLeft'?30:event.key==='ArrowRight'?-30:0;camera.y+=event.key==='ArrowUp'?30:event.key==='ArrowDown'?-30:0;updateCamera();}
  });
  $('zoom-in').addEventListener('click',()=>zoom(1.25));$('zoom-out').addEventListener('click',()=>zoom(.8));$('fit-map').addEventListener('click',fitMap);$('focus-node').addEventListener('click',focusNode);
  $('scenario').addEventListener('change',()=>setScenario($('scenario').value));
  $('refresh-map').addEventListener('click',simulateRefresh);
  $('state-layer').addEventListener('click',event=>{
    const a=event.target.closest('[data-state-action]')?.dataset.stateAction;if(!a)return;
    if(a==='stop'){setScenario('stopped');announce('Stopped. Previous map retained.');}
    else if(['finish','previous','sample'].includes(a)){setScenario('ready');renderGraph(true);}
    else if(a==='retry')simulateRefresh();
    else if(a==='clear-search')clearSearch();
    else if(a==='error-details')openEvidence('Map read details','<p class="dialog-intro">This is a simulated error state for reviewing the recovery flow.</p><dl class="detail-fields"><div><dt>Operation</dt><dd>Read map snapshot</dd></div><div><dt>Result</dt><dd>MAP_SNAPSHOT_CHANGED</dd></div><div><dt>Repository changes</dt><dd>None</dd></div><div><dt>Next step</dt><dd>Refresh to rebuild the map</dd></div></dl>');
  });
  $('window-size').addEventListener('change',()=>{$('app').dataset.size=$('window-size').value;});
  $('reset-preview').addEventListener('click',()=>{prefs=defaults();setScenario('ready');detailOpen=!compact;setView('code');announce('Preview reset.');});
  $('close-dialog').addEventListener('click',closeEvidence);$('done-dialog').addEventListener('click',closeEvidence);
  $('evidence-dialog').addEventListener('cancel',event=>{event.preventDefault();closeEvidence();});
  document.addEventListener('keydown',event=>{
    if(event.key==='Escape'&&!$('evidence-dialog').open&&detailOpen){closeDetails(true);}
  });
  const resize=new ResizeObserver(()=>{
    cancelAnimationFrame(layoutFrame);
    layoutFrame=requestAnimationFrame(()=>{
      const nextCompact=$('app').getBoundingClientRect().width<=1000;
      if(nextCompact!==compact){compact=nextCompact;detailOpen=!compact;overview=false;syncDetails();}
      layoutMap();
    });
  });
  resize.observe($('app'));resize.observe($('graph-area'));
  compact=$('app').getBoundingClientRect().width<=1000;detailOpen=!compact;
  setView('code');
})();
