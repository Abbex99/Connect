import {createHmac} from 'node:crypto';
import {verifyRegistration,validSlug,validId} from '../lib/directory-protocol.mjs';
import {register,lookup,byOwner,byServerName} from '../lib/directory-store.mjs';
export const config={api:{bodyParser:false}};
async function readBody(req){
 let text='';
 if(req.body!==undefined)text=typeof req.body==='string'?req.body:JSON.stringify(req.body);
 else{const chunks=[];let size=0;for await(const chunk of req){size+=chunk.length;if(size>16384)throw Object.assign(new Error(),{status:413});chunks.push(chunk);}text=Buffer.concat(chunks).toString('utf8');}
 if(Buffer.byteLength(text)>16384)throw Object.assign(new Error(),{status:413});
 try{return JSON.parse(text);}catch{throw Object.assign(new Error(),{status:400});}
}
export default async function handler(req,res){
 res.setHeader('Cache-Control','no-store');res.setHeader('Content-Type','application/json; charset=utf-8');
 res.setHeader('X-Content-Type-Options','nosniff');
 const send=(status,data)=>{res.statusCode=status;res.end(data===undefined?'':JSON.stringify(data));};
 try{
  const url=new URL(req.url,'https://connect.invalid');
  const route=url.searchParams.get('route')||url.pathname.replace(/^\//,'');
  if(route==='health'&&req.method==='GET')return send(200,{service:'connect-directory',version:3});
  let m;
  if((m=/^v([123])\/register$/.exec(route))){
   if(req.method!=='POST'){res.setHeader('Allow','POST');return send(405,{error:'Använd POST.'});}
   if(!req.headers['content-type']?.startsWith('application/json'))return send(415,{error:'JSON krävs.'});
   const value=verifyRegistration(Number(m[1]),await readBody(req));
   // Vercel overwrites this header. Never persist raw IP addresses.
   const ip=process.env.VERCEL?req.headers['x-vercel-forwarded-for']||'unknown':req.socket?.remoteAddress||'local';
   const bucket=createHmac('sha256',process.env.DATABASE_URL||'local').update(String(ip)).digest('hex');
   const status=await register(value,bucket);
   if(status===429)res.setHeader('Retry-After','60');
   return send(status,status===204?undefined:{error:status===409?'Namnet tillhör en annan identitet eller registreringen är äldre.':'Adressboken kan inte ta emot registreringen just nu.'});
  }
  if(req.method!=='GET'){res.setHeader('Allow','GET');return send(405,{error:'Använd GET.'});}
  if((m=/^v([12])\/servers\/([^/]+)\/([^/]+)$/.exec(route))&&validSlug(m[2])&&validSlug(m[3])){
   const record=await lookup('name:'+m[2]+'/'+m[3],Number(m[1]));
   return record?send(200,record):send(404,{error:'Servern är offline eller saknas i adressboken.'});
  }
  if((m=/^v3\/servers\/([^/]+)$/.exec(route))&&validId(m[1])){
   const record=await lookup('id:'+m[1],3);return record?send(200,record):send(404,{error:'Servern är offline eller har inte registrerats ännu.'});
  }
  if((m=/^v3\/names\/([^/]+)$/.exec(route))&&validSlug(m[1])){
   const record=await byServerName(m[1]);return record?send(200,record):send(404,{error:'Servern är offline eller namnet är inte registrerat.'});
  }
  if((m=/^v1\/users\/([^/]+)$/.exec(route))&&validSlug(m[1]))return send(200,await byOwner(m[1]));
  send(404,{error:'Adressen finns inte.'});
 }catch(e){send(e.status||503,{error:e.status?'Ogiltig serverregistrering.':'Adressboken är tillfälligt otillgänglig. Försök igen.'});}
}
