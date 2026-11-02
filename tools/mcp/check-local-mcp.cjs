const path = require('node:path');
const { RocketHttpClient } = require('./mcp-http-client.cjs');
const profile = path.join(process.env.LOCALAPPDATA, 'Rocket', 'state', 'mcp');
let phase = 'profile';
(async () => {
  const client = new RocketHttpClient(profile);
  try {
    phase = 'health';
    const response = await client.raw('GET', '/health');
    if (response.status !== 200 || !response.text.startsWith('rocket-mcp\n')) throw Error('The configured port did not return authenticated Rocket health');
    phase = 'initialize';
    const initialized = await client.initialize();
    phase = 'state';
    const state = await client.tool('rocket_state');
    console.log(JSON.stringify({ endpoint: 'http://127.0.0.1:' + client.port + '/mcp', version: initialized.serverInfo.version, window: state.window, pid: state.pid }));
  } catch (error) {
    if (error.code === 'ECONNREFUSED') process.exitCode = 2;
    else throw error;
  } finally {
    await client.finish();
  }
})().catch(error => { console.error(phase + ': ' + error.message); process.exitCode = 1; });
