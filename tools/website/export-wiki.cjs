'use strict';
// Export reviewed first-party Markdown only. No network, Git writes or credentials.
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict');
const project=path.resolve(__dirname,'../..'),source=path.join(project,'docs/wiki');
const repository='https://github.com/JasXTD/Rocket';
function exportWiki(destination){
  destination=path.resolve(destination);assert(!fs.existsSync(destination),'Choose a new export folder; existing pages are never overwritten');
  const names=fs.readdirSync(source).filter(n=>n.endsWith('.md')).sort();assert(names.includes('Home.md'));
  const entries=new Map();
  function link(value){
    if(!value||value.startsWith('#')||/^https?:\/\//.test(value))return value;
    assert(!/^[a-z]+:|^[/\\]/i.test(value),'Refuse non-repository link: '+value);
    const [file,anchor]=value.split('#'),target=path.resolve(source,file),relative=path.relative(project,target).replaceAll('\\','/');
    assert(relative&&!relative.startsWith('../')&&!path.isAbsolute(relative),'Link escapes repository: '+value);
    assert(fs.statSync(target).isFile(),'Missing linked file: '+value);
    const suffix=anchor?'#'+anchor:'';
    if(path.dirname(target)===source&&names.includes(path.basename(target)))return encodeURIComponent(path.basename(target,'.md'))+suffix;
    const image=/\.(?:png|webp|jpe?g|svg|gif)$/i.test(file);
    return repository+(image?'/raw/refs/heads/main/':'/blob/main/')+relative.split('/').map(encodeURIComponent).join('/')+suffix;
  }
  for(const name of names){
    const file=path.join(source,name);assert(fs.lstatSync(file).isFile()&&!fs.lstatSync(file).isSymbolicLink());
    let text=fs.readFileSync(file,'utf8');
    text=text.replace(/(!?\[[^\]\n]*\]\()([^\s)]+)(\))/g,(_,a,url,b)=>a+link(url)+b);
    text=text.replace(/\b(src|srcset)="([^"]+)"/g,(_,attribute,url)=>attribute+'="'+link(url)+'"');
    assert(!/\]\(\.\.?\//.test(text),'Unconverted Markdown link');entries.set(name,text);
  }
  entries.set('_Sidebar.md','# Rocket\n\n'+names.map(n=>'- ['+n.slice(0,-3).replaceAll('-',' ')+']('+encodeURIComponent(n.slice(0,-3))+')').join('\n')+'\n');
  entries.set('_Footer.md','Maintained in [docs/wiki]('+repository+'/tree/main/docs/wiki). [Downloads]('+repository+'/releases/tag/v0.92.1) · [Report an issue]('+repository+'/issues).\n');
  fs.mkdirSync(destination,{recursive:true});for(const [name,text]of entries)fs.writeFileSync(path.join(destination,name),text,{flag:'wx'});
  return {source_pages:names.length,exported_files:entries.size,destination};
}
module.exports={exportWiki};
if(require.main===module){try{assert(process.argv.length===3,'Usage: node tools/website/export-wiki.cjs NEW_DIRECTORY');console.log(JSON.stringify(exportWiki(process.argv[2])));}catch(e){console.error(e.message);process.exitCode=1;}}
