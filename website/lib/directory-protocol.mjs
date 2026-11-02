import {createPublicKey,verify} from 'node:crypto';
const fail=()=>{throw Object.assign(new Error('Ogiltig eller utgången serverregistrering.'),{status:400});};
export const validSlug=x=>typeof x==='string'&&/^[a-z0-9_-]{1,32}$/.test(x);
export const validId=x=>typeof x==='string'&&/^[a-f0-9]{64}$/.test(x);
const stamp=x=>{if(!Number.isSafeInteger(x)||x<0)fail();return x;};
const name=x=>{if(typeof x!=='string'||!x.trim()||Buffer.byteLength(x)>96||/[\x00-\x09\x0b-\x1f\x7f]/.test(x))fail();return x;};
function address(x){
 if(!x||!validId(x.id)||!Array.isArray(x.addrs)||x.addrs.length>64)fail();
 const addrs=x.addrs.map(a=>{
  if(!a||Object.keys(a).length!==1)fail();
  if(typeof a.Relay==='string'){
   let u;try{u=new URL(a.Relay);}catch{fail();}
   if(u.protocol!=='https:'||u.username||u.password||u.hash||a.Relay.length>2048)fail();
   return {Relay:a.Relay};
  }
  if(typeof a.Ip==='string'&&a.Ip.length<=64&&/^(?:[0-9.]+|\[[a-fA-F0-9:]+\]):[0-9]{1,5}$/.test(a.Ip))return {Ip:a.Ip};
  fail();
 });
 const result={id:x.id,addrs};if(Buffer.byteLength(JSON.stringify(result))>8192)fail();return result;
}
function signature(id,domain,value,sig){
 if(!validId(id)||typeof sig!=='string'||!/^[A-Za-z0-9_-]{86}$/.test(sig))fail();
 try{
  const key=createPublicKey({key:Buffer.concat([Buffer.from('302a300506032b6570032100','hex'),Buffer.from(id,'hex')]),format:'der',type:'spki'});
  if(!verify(null,Buffer.from(domain+'\0'+JSON.stringify(value)),key,Buffer.from(sig,'base64url')))fail();
 }catch{fail();}
}
export function verifyRegistration(version,input,now=Math.floor(Date.now()/1000)){
 if(![1,2,3].includes(version)||!input||Buffer.byteLength(JSON.stringify(input))>16384)fail();
 let record,owner,slug,authority,server,issued,delegated=0;
 if(version===2){
  const d=input.delegation;if(!d||!validSlug(d.owner)||!validSlug(d.slug)||!validId(d.server))fail();
  const delegation={owner:d.owner,slug:d.slug,authority:d.authority,server:d.server,issued_at:stamp(d.issued_at),signature:d.signature};
  if(delegation.issued_at>now+30)fail();
  signature(d.authority,'connect/server-delegation/1',[d.owner,d.slug,d.authority,d.server,d.issued_at],d.signature);
  record={delegation,name:name(input.name),address:address(input.address),issued_at:stamp(input.issued_at),signature:input.signature};
  if(record.address.id!==d.server)fail();
  signature(d.server,'connect/server-registration/2',[delegation,record.name,record.address,record.issued_at],input.signature);
  ({owner,slug,authority,server}=delegation);issued=record.issued_at;delegated=d.issued_at;
 }else{
  const r=input.record;if(!r||!validSlug(r.owner)||!validSlug(r.slug))fail();
  const value={owner:r.owner,slug:r.slug,name:name(r.name),address:address(r.address),issued_at:stamp(r.issued_at)};
  signature(value.address.id,version===3?'connect/server-location/3':'serein-directory-registration-v1',value,input.signature);
  record={record:value,signature:input.signature};owner=r.owner;slug=r.slug;authority=server=value.address.id;issued=value.issued_at;
 }
 if(issued>now+30||now-issued>180)fail();
 if(version===3&&input.access!==undefined){
  const a=input.access;
  if(!a||a.server!==server||a.issued_at!==issued||!['open','password','invite'].includes(a.mode))fail();
  const access={server,issued_at:issued,mode:a.mode,salt:a.salt,nonce:a.nonce,ciphertext:a.ciphertext,signature:a.signature};
  if(a.mode==='password'){
   if(!/^[A-Za-z0-9_-]{22}$/.test(a.salt)||!/^[A-Za-z0-9_-]{16}$/.test(a.nonce)||!/^[A-Za-z0-9_-]{107}$/.test(a.ciphertext))fail();
  }else if(a.salt!==''||a.nonce!==''||a.ciphertext!=='')fail();
  signature(server,'connect/server-access/1',[server,issued,a.mode,a.salt,a.nonce,a.ciphertext],a.signature);
  record.access=access;
 }
 return {version,record,owner,slug,authority,server,issued,delegated,key:version===3?'id:'+server:'name:'+owner+'/'+slug};
}
