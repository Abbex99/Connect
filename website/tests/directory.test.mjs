import test from 'node:test';
import assert from 'node:assert/strict';
import { generateKeyPairSync, sign } from 'node:crypto';
import { readFileSync, existsSync } from 'node:fs';
import { verifyRegistration } from '../lib/directory-protocol.mjs';

export function fixture(owner='test', issued=Math.floor(Date.now()/1000),version=1) {
  const {privateKey,publicKey}=generateKeyPairSync('ed25519');
  const id=publicKey.export({type:'spki',format:'der'}).subarray(-32).toString('hex');
  const record={owner,slug:version===3?owner:'room',name:'Test Å 🎮',address:{id,addrs:[{Relay:'https://euc1-1.relay.n0.iroh.link./'}]},issued_at:issued};
  const domain=version===3?'connect/server-location/3':'serein-directory-registration-v1';
  const signed={record,signature:sign(null,Buffer.from(domain+'\0'+JSON.stringify(record)),privateKey).toString('base64url')};
  return {signed,privateKey,id};
}
export function withAccess(f){
 const a={server:f.id,issued_at:f.signed.record.issued_at,mode:'open',salt:'',nonce:'',ciphertext:''};
 a.signature=sign(null,Buffer.from('connect/server-access/1\0'+JSON.stringify([a.server,a.issued_at,a.mode,a.salt,a.nonce,a.ciphertext])),f.privateKey).toString('base64url');
 f.signed.access=a;return f;
}
test('valid signatures, timestamp boundaries and tamper rejection',()=>{
 const {signed,id}=fixture(); const now=signed.record.issued_at;
 assert.equal(verifyRegistration(1,signed,now).server,id);
 assert.throws(()=>verifyRegistration(3,signed,now));
 assert.throws(()=>verifyRegistration(1,{...signed,record:{...signed.record,name:'Fake'}},now));
 assert.throws(()=>verifyRegistration(1,signed,now+181));
 assert.throws(()=>verifyRegistration(1,signed,now-31));
 assert.throws(()=>verifyRegistration(1,{...signed,signature:'invalid'},now));
 assert.throws(()=>verifyRegistration(1,{...signed,record:{...signed.record,owner:'../bad'}},now));
 assert.throws(()=>verifyRegistration(1,{...signed,padding:'x'.repeat(17000)},now));
});
test('automatic server locations cannot claim a personal namespace by replay',()=>{
 const {signed,id}=fixture('server',undefined,3);
 assert.equal(verifyRegistration(3,signed).key,'id:'+id);
 assert.throws(()=>verifyRegistration(1,signed));
});
test('canonical field order does not depend on JSON object order',()=>{
 const {signed}=fixture();
 const reordered={signature:signed.signature,record:Object.fromEntries(Object.entries(signed.record).reverse())};
 assert.equal(verifyRegistration(1,reordered).server,signed.record.address.id);
});
test('Rust-produced v1 and v2 vectors verify, forged delegation rejected',()=>{
 const path=new URL('./fixtures/rust-directory.json',import.meta.url);
 assert.ok(existsSync(path),'Generate the Rust directory_vector fixture first');
 const vectors=JSON.parse(readFileSync(path,'utf8'));
 assert.equal(verifyRegistration(1,vectors.v1,vectors.v1.record.issued_at).server,vectors.v1.record.address.id);
 assert.equal(verifyRegistration(2,vectors.v2,vectors.v2.issued_at).server,vectors.v2.address.id);
 assert.equal(verifyRegistration(3,vectors.v3,vectors.v3.record.issued_at).server,vectors.v3.record.address.id);
 assert.throws(()=>verifyRegistration(1,vectors.v3,vectors.v3.record.issued_at));
 const tampered=structuredClone(vectors.v2);tampered.delegation.authority=tampered.delegation.server;
 assert.throws(()=>verifyRegistration(2,tampered,vectors.v2.issued_at));
});
