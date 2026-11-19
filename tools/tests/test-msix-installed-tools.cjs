'use strict';
const test=require('node:test'),assert=require('node:assert/strict'),path=require('node:path').win32;
const {installedTools}=require('./msix-installed-tools.cjs');
const identity='Rocket.CompatibilityTest.012345abcdef',version='1.2.3.0';
const fullName=identity+'_'+version+'_x64__test';
const registration={Name:identity,Publisher:'CN=Rocket Compatibility Test 012345abcdef',Version:version,
  InstallLocation:path.join('D:/WindowsApps',fullName),PackageFullName:fullName,IsDevelopmentMode:false};
const build={identity,packageVersion:version,get payload(){throw Error('Read a staging path from the build record')}};

test('Git and MCP come from the matching installed package on its registered volume',()=>{
  const tools=installedTools([registration],build);
  assert.equal(tools.git,path.join(registration.InstallLocation,'app/git/2.56.1.windows.1/cmd/git.exe'));
  assert.equal(tools.mcp,path.join(registration.InstallLocation,'app/rocket-mcp.exe'));
});
test('absent, ambiguous and mismatching registrations do not yield executable paths',()=>{
  for(const registrationList of [[],[registration,registration],...[{Publisher:'wrong'},{Version:'1.0.0.0'},
    {Name:'different'},{IsDevelopmentMode:true},{InstallLocation:'relative'},{PackageFullName:'unrelated'}].map(change=>[{...registration,...change}])]){
    assert.throws(()=>installedTools(registrationList,build));
  }
});
