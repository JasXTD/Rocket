// Runs through connected MCP tool adapters and owned fixture helpers.
async function runMcpTransport(root, mode) {
  const report = { passed:false, mode, root, via:"MCP tools; actual Rocket widgets", phases:[], cleanup:"pending" };
  const records=[]; let cursor=load("networkLogCursor")??0; let state; let tab; let phase="open"; let session;
  const require = (ok,msg) => { if(!ok) throw Error(msg); };
  async function call(name,args={}) {
    const r=await tools["mcp__rocket_ui__"+name](args);
    if(r.isError) throw Error(r.content.filter(c=>c.type==="text").map(c=>c.text).join("\n"));
    return r.structuredContent??r;
  }
  async function helper(action) {
    let r=await tools.exec_command({cmd:"node .\\work\\mcp-github-fixture.cjs "+action+" '"+root.replaceAll("'","''")+"'",workdir:"C:\\Users\\Tommy\\Desktop\\Rocket",login:false,yield_time_ms:1000,max_output_tokens:1800});
    let output=r.output;
    while(r.session_id) { r=await tools.write_stdin({session_id:r.session_id,chars:"",yield_time_ms:1000,max_output_tokens:1800}); output+=r.output; }
    require(r.exit_code===0,"Fixture helper "+action+" failed: "+output);
    const v=JSON.parse(output.trim()); require(v.passed,"Fixture helper check did not pass"); return v;
  }
  async function settled() {
    const deadline=Date.now()+90000;
    do { state=await call("rocket_wait",{timeout_ms:10000}); if(!state.busy&&!state.input_pending) return; } while(Date.now()<deadline);
    throw Error("UI workers did not settle; operation was not retried");
  }
  async function readLogs() {
    for(;;) {
      const b=await call("rocket_logs",{after_seq:cursor,limit:1000});
      require(!b.retention_gap,"Diagnostic retention gap");
      if(session) require(session===b.session,"Diagnostic session changed"); session=b.session;
      require(b.next_seq>=cursor,"Diagnostic sequence moved backwards");
      records.push(...b.records);cursor=b.next_seq;store("networkLogCursor",cursor);
      if(!b.more) break;
    }
  }
  async function capture(label) {
    const r=await tools.mcp__rocket_ui__rocket_capture({});
    require(!r.isError,"Window capture failed");
    const im=r.content.find(c=>c.type==="image");require(im,"Window image missing");
    report.captures??=[];report.captures.push(label);image(im);
  }
  async function click(target) { await call("rocket_click",{target,expected_tab:tab});state=await call("rocket_state"); }
  async function operation(label,operation,expected="",upstream=false,pullKind="") {
    phase=label;
    state=await call("rocket_state");require(!state.busy&&!state.input_pending&&!state.dialog,"UI not ready before "+label);
    const count=state.activities.length;const logStart=records.length;
    await click("toolbar-"+operation);require(state.dialog?.kind===operation,"Wrong operation dialog");
    if(operation==="push") {
      await call("rocket_type",{target:"push-destination",text:"rocket-ui-test/"+root.split("\\").at(-1),expected_tab:tab});
      state=await call("rocket_state");
      if(state.dialog.flag!==upstream) await click("push-upstream");
      require(state.dialog.second==="rocket-ui-test/"+root.split("\\").at(-1)&&state.dialog.flag===upstream,"Wrong push destination or upstream option");
    }
    await click("dialog-submit");await settled();
    require(state.activities.length===count+1,"Missing/duplicate activity");
    const a=state.activities.at(-1);require(a.operation===operation,"Wrong activity operation");
    const details=a.details.join("\n");
    require(!/github_pat_|gh[pousr]_[A-Za-z0-9]{16,}|Authorization:\s*(Basic|Bearer)\s+\S+|PRIVATE KEY/i.test(details),"Secret-shaped UI result; suppressed");
    const failure=details.match(/\[([a-z_]+)\]/)?.[1]??"";
    report.phases.push({label,operation,expected,command_succeeded:a.command_succeeded,summary:a.summary,details:a.details,elapsed_ms:a.elapsed_ms,failure_code:failure});
    if(expected) require(!a.command_succeeded&&new RegExp("^(?:"+expected+")$").test(failure)&&details.includes("Next: "),"Wrong rejection or missing guidance for "+label+" (observed "+failure+")");
    else require(a.command_succeeded&&a.details.length===0,"Operation did not report clean success: "+label);
    require(state.dropped_events===0,"Dropped UI log events");
    require(!state.tabs.find(t=>t.id===tab).error,"Repository refresh failed");
    await readLogs();
    const phaseLogs=records.slice(logStart);
    require(phaseLogs.filter(r=>r.event==="request_start"&&r.operation===operation).length===1,"Missing/duplicate backend request for "+label);
    const cmd=operation==="pull"?"fetch":operation;
    require(phaseLogs.filter(r=>r.event==="git_start"&&r.operation===cmd).length===(failure==="pull_dirty_worktree"?0:1),"Unexpected command count/retry for "+label);
    if(failure) require(phaseLogs.some(r=>r.failure_code===failure),"Displayed failure missing from diagnostics");
    if(pullKind) {
      const outcomes=phaseLogs.filter(r=>r.event==="pull_outcome");
      require(outcomes.length===1&&outcomes[0].update_kind===pullKind&&outcomes[0].verification==="ok"&&outcomes[0].fetch_outcome==="succeeded","Pull phase outcome mismatch");
    }
    if(label==="fetch-authentication-unavailable"||label==="pull-dirty-worktree") await capture(mode+"-"+label);
    report.phases.at(-1).verified=true;
    notify({mode,phase:label,passed:true,operation,outcome:failure||"success"});
  }
  try {
    await call("rocket_open",{path:root+"\\ROCKET_PRIVATE_client"});await settled();tab=state.active_tab;
    require(state.window_count===1,"Expected one persistent window");
    report.pid=state.pid;report.window_count=state.window_count;report.tab=tab;
    await readLogs();await helper("before-push");
    await operation("push-new-branch","push","",true);await helper("verify-new-push");
    await helper("auth-off");
    const auth=mode==="https"?"credentials_unavailable|authentication_rejected":"ssh_authentication_rejected";
    for(const op of ["fetch","pull","push"]) { await operation(op+"-authentication-unavailable",op,auth);await helper("verify-unchanged"); }
    await helper("auth-on");
    await operation("fetch-up-to-date","fetch");await helper("verify-unchanged");
    await helper("publish-advance");await operation("fetch-new-commit","fetch");await helper("verify-fetch");
    await operation("pull-fast-forward","pull","",false,"fast_forward");await helper("verify-pull");
    await operation("pull-up-to-date","pull","",false,"up_to_date");await helper("verify-unchanged");
    await helper("local-advance");await click("refresh");await settled();await readLogs();
    await operation("push-fast-forward","push");await helper("verify-local-push");
    await operation("push-up-to-date","push");await helper("verify-unchanged");
    await helper("diverge");await click("refresh");await settled();await readLogs();
    await operation("push-fetch-first","push","push_fetch_first");await helper("verify-unchanged");
    await operation("fetch-diverged","fetch");await helper("verify-diverged-fetch");
    await operation("push-non-fast-forward","push","push_non_fast_forward");await helper("verify-unchanged");
    await operation("pull-diverged","pull","pull_diverged");await helper("verify-unchanged");
    await helper("dirty");await click("refresh");await settled();await readLogs();
    await operation("pull-dirty-worktree","pull","pull_dirty_worktree");await helper("verify-unchanged");
    report.operations_passed=true;
  } catch(e) { report.failure_phase=phase;report.error=e.message;notify({mode,failure_phase:phase,error:e.message}); }
  finally {
    try { await settled();const c=await helper("cleanup");report.cleanup=c.cleanup;report.original_refs_and_default_head_unchanged=true; }
    catch(e) { report.cleanup_error=e.message; }
    try {
      await readLogs();require(records.length>0&&records.every(r=>r.schema===1&&r.dropped_before===0),"Invalid or dropped logs");
      const payload=JSON.stringify(records);
      require(!/ROCKET_PRIVATE_|rocket-ui-test|refs\/heads\/|refs\/remotes\/|github\.com|rocket-auth-test|github_pat_|gh[pousr]_[A-Za-z0-9]{16,}|PRIVATE KEY/.test(payload),"Private metadata or credential pattern in logs");
      for(const start of records.filter(r=>["request_start","step_start","git_start"].includes(r.event)))
        require(records.filter(r=>r.event===start.event.replace("_start","_end")&&r.id===start.id&&r.session===start.session).length===1,"Unpaired diagnostic span");
      report.logs={records:records.length,session,dropped:0,paired:true,privacy_scan_passed:true,no_automatic_retries:report.phases.every(p=>p.verified)};
      const p=root+"\\mcp-diagnostics.json";
      await tools.apply_patch("*** Begin Patch\n*** Add File: "+p+"\n"+JSON.stringify(records,null,2).split("\n").map(l=>"+"+l).join("\n")+"\n*** End Patch");
    } catch(e) { report.log_error=e.message; }
    report.passed=!!report.operations_passed&&report.cleanup==="confirmed-absent"&&!!report.logs;
    store(mode+"NetworkReport",report);
    await tools.apply_patch("*** Begin Patch\n*** Add File: "+root+"\\mcp-result.json\n"+JSON.stringify(report,null,2).split("\n").map(l=>"+"+l).join("\n")+"\n*** End Patch");
  }
  text(report);
  return report;
}
if (typeof module !== 'undefined') module.exports = runMcpTransport;
