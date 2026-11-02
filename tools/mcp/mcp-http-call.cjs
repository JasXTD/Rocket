// Direct localhost client: node tools/mcp/mcp-http-call.cjs PROFILE TOOL [ARGUMENTS_JSON_FILE]
const fs = require('node:fs');
const { RocketHttpClient } = require('./mcp-http-client.cjs');
(async () => {
  const [profile, name, argumentsFile] = process.argv.slice(2);
  if (!profile || !name) throw Error('Provide the local profile and tool name');
  const client = new RocketHttpClient(profile);
  if (name === '--shutdown') {
    const reply = await client.raw('POST', '/shutdown');
    if (reply.status !== 202) throw Error(reply.text);
    console.log('Local MCP service accepted shutdown.');
    return;
  }
  await client.initialize();
  const result = await client.tool(name, argumentsFile ? JSON.parse(fs.readFileSync(argumentsFile, 'utf8')) : {});
  console.log(JSON.stringify(result, null, 2));
})().catch(error => { console.error(error.message); process.exitCode = 1; });
