// Shared post-run assertions; no launches or filesystem mutations.
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict');
function checkLogs(root){
 const records=fs.readdirSync(root).filter(n=>n.startsWith('mcp-logs-')).flatMap(n=>fs.readdirSync(path.join(root,n)).filter(f=>/^rocket(?:\.\d+)?\.jsonl$/.test(f)).flatMap(f=>fs.readFileSync(path.join(root,n,f),'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
 assert(records.length);assert(records.every(r=>r.dropped_before===0));const text=JSON.stringify(records);for(const secret of ['PRIVATE_','版本 文.txt','rocket-ui-test/auto-bisect','PRIVATE_AUTO_OUTPUT'])assert(!text.includes(secret),'Diagnostic leak '+secret);
 for(const op of ['bisect-auto-preview','bisect-auto','bisect-tests','bisect-restore']){const starts=records.filter(r=>r.event==='request_start'&&r.operation===op);assert(starts.length,'Missing '+op);for(const s of starts)assert(records.some(r=>r.event==='request_end'&&r.session===s.session&&r.request===s.request),'Unpaired request');}
 for(const outcome of ['good','bad','skip','failed','timeout','changed','cancelled'])assert(records.some(r=>r.event==='bisect_test_end'&&r.outcome===outcome),'Missing outcome '+outcome);
 for(const step of ['run_test_command','run_reviewed_search'])assert(records.some(r=>r.step===step&&r.operation==='bisect-auto'),'Missing step '+step);
 return records.length;
}
module.exports={checkLogs};
