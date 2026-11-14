'use strict';
const test=require('node:test'),assert=require('node:assert/strict');
const {create}=require('./fixtures/hosting-other-inline.cjs');

test('Azure fixture enforces configured versions and scopes the 6.0 preview to PR statuses',async()=>{
 const token='disposable-fixture',fixture=create('azure-server',token);
 const port=await fixture.listen(),prefix=`http://127.0.0.1:${port}/azure-server/tfs/Fixture/team/_apis/git/repositories/sample`;
 const headers={Authorization:'Basic '+Buffer.from(':'+token).toString('base64')};
 try{
  for(const configured of ['6.0','7.0','7.1']){
   fixture.state.apiVersion=configured;
   for(const [route,expected]of [['/pullrequests/42/statuses',configured==='6.0'?'6.0-preview.1':configured],['/pullrequests/42',configured],['/commits/'+fixture.state.head+'/statuses',configured]]){
    const response=await fetch(prefix+route+'?api-version='+expected,{headers});assert.equal(response.status,200);await response.arrayBuffer();assert.equal(fixture.state.error,null);
    const wrong=expected===configured?'6.0-preview.1':configured;
    const rejected=await fetch(prefix+route+'?api-version='+wrong,{headers});assert.equal(rejected.status,500);await rejected.arrayBuffer();assert.match(fixture.state.error,/incorrect configured API version/);fixture.state.error=null;
   }
  }
 }finally{fixture.close();}
});
