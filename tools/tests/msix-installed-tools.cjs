'use strict';
const assert=require('node:assert/strict');
const path=require('node:path').win32;

function installedTools(registrations,build){
  const packages=[registrations].flat().filter(Boolean);
  assert.equal(packages.length,1,'Expected one installed compatibility package');
  const pkg=packages[0];
  assert.match(build.identity,/^Rocket\.CompatibilityTest\.[a-f0-9]{12}$/);
  assert.equal(pkg.Name,build.identity);
  assert.equal(pkg.Publisher,'CN=Rocket Compatibility Test '+build.identity.split('.').at(-1));
  assert.equal(pkg.Version,build.packageVersion);
  assert.equal(pkg.IsDevelopmentMode,false,'Expected an installed MSIX');
  assert.match(pkg.InstallLocation,/^[A-Za-z]:[\\/]/);
  assert.equal(path.basename(pkg.InstallLocation),pkg.PackageFullName);
  const app=path.join(pkg.InstallLocation,'app');
  return {git:path.join(app,'git/2.56.1.windows.1/cmd/git.exe'),mcp:path.join(app,'rocket-mcp.exe')};
}
module.exports={installedTools};
