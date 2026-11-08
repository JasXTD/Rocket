'use strict';
const fs = require('node:fs'), path = require('node:path'), assert = require('node:assert/strict');
const packages = require('../prototypes/macos-lua-wasm/approved-crates.json');
const [vendor, output] = process.argv.slice(2);
assert(vendor && output && path.isAbsolute(vendor) && path.isAbsolute(output));
let notice = 'Rocket Mac Lua helper: Wasmtime/Pulley 50.0.1, Lua 5.5.1 and Rust bindings.\n\n';
for (const p of packages) {
  const directory = path.join(vendor, p.name + '-' + p.version);
  let licenseDirectory = directory;
  let names = fs.readdirSync(directory).filter(n => /^(LICENSE|LICENCE|COPYING|NOTICE)([.\-_]|$)/i.test(n) && fs.lstatSync(path.join(directory,n)).isFile());
  if (!names.length) {
    // Some monorepo subcrates omit the workspace LICENSE from their archive.
    // Reuse it only from the same approved revision and declared license.
    const vcs = JSON.parse(fs.readFileSync(path.join(directory,'.cargo_vcs_info.json'))).git.sha1;
    const license = fs.readFileSync(path.join(directory,'Cargo.toml'),'utf8').match(/^license\s*=\s*"([^"]+)"/m)?.[1];
    for (const candidate of packages) {
      const sibling = path.join(vendor,candidate.name+'-'+candidate.version);
      const metadata = path.join(sibling,'.cargo_vcs_info.json');
      if (!fs.existsSync(metadata) || JSON.parse(fs.readFileSync(metadata)).git.sha1 !== vcs) continue;
      if (fs.readFileSync(path.join(sibling,'Cargo.toml'),'utf8').match(/^license\s*=\s*"([^"]+)"/m)?.[1] !== license) continue;
      const found=fs.readdirSync(sibling).filter(n=>/^LICENSE([.\-_]|$)/i.test(n)&&fs.lstatSync(path.join(sibling,n)).isFile());
      if(found.length) { licenseDirectory=sibling; names=found; break; }
    }
    assert(names.length, 'Missing matching upstream license for ' + p.name);
  }
  notice += `\n=== ${p.name} ${p.version} (https://crates.io/crates/${p.name}/${p.version}) ===\n`;
  for (const name of names.sort()) { const file = path.join(licenseDirectory,name); assert(!fs.lstatSync(file).isSymbolicLink()); notice += '\n'+name+'\n'+fs.readFileSync(file,'utf8')+'\n'; }
}
fs.writeFileSync(output, notice, { flag: 'wx', mode: 0o644 });
