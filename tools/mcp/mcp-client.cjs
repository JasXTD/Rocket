const { spawn, spawnSync } = require('node:child_process');
const { once } = require('node:events');

// Small stdio MCP client for local acceptance. Keeps exactly one server process.
class RocketMcpClient {
  constructor(executable, root, env = process.env, options = {}) {
    if (env.ROCKET_MCP_HTTP_PROFILE) return new (require('./mcp-http-client.cjs').RocketHttpClient)(env.ROCKET_MCP_HTTP_PROFILE);
    const ssh = options.windowsSshPublicKey ? ['--windows-ssh-agent', options.windowsSshPublicKey] : [];
    this.child = spawn(executable, ['--test-root', root, ...ssh], { env, windowsHide:true, stdio:['pipe','pipe','pipe'] });
    this.pending = new Map(); this.nextId = 0; this.calls = 0; this.stderr = ''; this.buffer = ''; this.exited = false;
    this.child.stderr.on('data', b => { this.stderr = (this.stderr + b.toString()).slice(-65536); });
    this.child.stdout.setEncoding('utf8');
    this.child.stdout.on('data', chunk => {
      this.buffer += chunk;
      if (this.buffer.length > 16 * 1024 * 1024) { this.fail(new Error('MCP output exceeded its bound')); return; }
      for (;;) {
        const end = this.buffer.indexOf('\n'); if (end < 0) break;
        const line = this.buffer.slice(0,end); this.buffer = this.buffer.slice(end+1);
        let value; try { value = JSON.parse(line); } catch { this.fail(new Error('Non-JSON output on MCP stdout')); return; }
        const request = this.pending.get(value.id); if (!request) continue;
        this.pending.delete(value.id); clearTimeout(request.timer);
        if (value.error) request.reject(new Error(value.error.message)); else request.resolve(value.result);
      }
    });
    this.child.on('error', e => this.fail(e));
    this.child.on('exit', (code, signal) => { this.exited = true; this.exitCode = code; this.fail(new Error(`MCP process exited (${code ?? signal})`)); });
    this.child.stdin.on('error', e => this.fail(e));
  }
  fail(error) { for (const p of this.pending.values()) {clearTimeout(p.timer);p.reject(error);} this.pending.clear(); }
  notify(method, params = {}) { this.child.stdin.write(JSON.stringify({jsonrpc:'2.0',method,params})+'\n'); }
  request(method, params = {}) {
    const id = ++this.nextId;
    return new Promise((resolve,reject) => {
      const timer = setTimeout(() => { this.pending.delete(id); this.notify('notifications/cancelled',{requestId:id}); reject(new Error(`MCP ${method} timed out; inspect state before retrying`)); },25000);
      this.pending.set(id,{resolve,reject,timer});
      this.child.stdin.write(JSON.stringify({jsonrpc:'2.0',id,method,params})+'\n');
    });
  }
  async initialize() {
    const info = await this.request('initialize',{protocolVersion:'2025-11-25',capabilities:{},clientInfo:{name:'Rocket acceptance',version:'1'}});
    if (info.protocolVersion !== '2025-11-25') throw Error('Unexpected MCP protocol version');
    this.notify('notifications/initialized'); return info;
  }
  async tool(name,args = {}) {
    this.calls++;
    const result = await this.request('tools/call',{name,arguments:args});
    if (result.isError) throw Error(result.content.map(c=>c.text || '').join('\n'));
    return result.structuredContent ?? result;
  }
  async finish() {
    if (this.exited) {
      if (this.exitCode !== 0) throw Error('MCP exited unsuccessfully');
      return;
    }
    this.child.stdin.end();
    let timer;
    try {
      await Promise.race([once(this.child,'exit'), new Promise((_,reject) => { timer=setTimeout(()=>reject(Error('MCP shutdown timed out')),30000); })]);
    } finally { clearTimeout(timer); }
    if (this.exitCode !== 0) throw Error('MCP exited unsuccessfully');
  }
  terminateOwnedProcess() {
    if (!this.exited && this.child.pid) spawnSync('taskkill.exe',['/PID',String(this.child.pid),'/T','/F'],{windowsHide:true,stdio:'ignore'});
  }
}
module.exports = { RocketMcpClient };
