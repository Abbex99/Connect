const $=id=>document.getElementById(id);
const parts=location.pathname.split('/').filter(Boolean),friendly=parts[0]==='s';
const name=parts[1],fragment=location.hash.slice(1);
const validId=x=>/^[a-f0-9]{64}$/.test(x||'');
const bytes=x=>Uint8Array.from(atob(x.replace(/-/g,'+').replace(/_/g,'/')),c=>c.charCodeAt(0));
let invite='',current;
async function verifiedSignature(id,signature,domain,value){
 const key=await crypto.subtle.importKey('raw',Uint8Array.from(id.match(/../g),x=>parseInt(x,16)),{name:'Ed25519'},false,['verify']);
 return crypto.subtle.verify('Ed25519',key,bytes(signature),new TextEncoder().encode(domain+'\0'+JSON.stringify(value)));
}
function prepare(token){
 const r=current.record,encoded=new TextEncoder().encode(JSON.stringify({version:2,address:r.address,token,alias:'@'+r.owner+'/'+r.slug,name:r.name}));
 invite='connect://join/'+btoa(String.fromCharCode(...encoded)).replace(/\+/g,'-').replace(/\//g,'_').replace(/=+$/,'');
 $('status').textContent='Redo att ansluta i Connect.';$('invite').value=invite;$('password-box').hidden=true;$('join').hidden=false;
}
async function load(){
 $('join').hidden=true;$('password-box').hidden=true;$('retry').hidden=true;$('status').textContent='Hämtar servern…';
 if(!(friendly?/^[a-z0-9_-]{1,32}$/.test(name||''):validId(name))||(!friendly&&!/^([a-fA-F0-9]{64})?$/.test(fragment))){$('status').textContent='Adressen verkar ofullständig. Be din vän skicka serveradressen igen.';return;}
 try{
  const response=await fetch('/v3/'+(friendly?'names/':'servers/')+name,{cache:'no-store',signal:AbortSignal.timeout(10000),referrerPolicy:'no-referrer'});
  if(!response.ok)throw Error(response.status===404?'Servern är offline eller har inte hunnit registreras. Be värden kontrollera serverpanelen.':'Adressboken kan inte nås just nu. Försök igen.');
  const text=await response.text();if(text.length>16384)throw Error('Serverns svar är ogiltigt.');
  const value=JSON.parse(text),r=value.record;
  if(!r||!validId(r.address?.id)||(!friendly&&r.address.id!==name)||!/^[a-z0-9_-]{1,32}$/.test(r.owner)||!/^[a-z0-9_-]{1,32}$/.test(r.slug)||typeof r.name!=='string'||r.name.length>96||!Array.isArray(r.address.addrs)||!Number.isSafeInteger(r.issued_at)||r.issued_at>Date.now()/1000+30||r.issued_at<Date.now()/1000-180)throw Error('Serveradressen är ogiltig eller har gått ut.');
  const canonical={owner:r.owner,slug:r.slug,name:r.name,address:{id:r.address.id,addrs:r.address.addrs},issued_at:r.issued_at};
  if(!await verifiedSignature(r.address.id,value.signature,'connect/server-location/3',canonical))throw Error('Serverns signatur kunde inte verifieras.');
  current={record:canonical,access:value.access};$('server-name').textContent=r.name;
  if(!friendly&&fragment){prepare(fragment);return;}
  const a=value.access;
  if(!a||a.server!==r.address.id||a.issued_at!==r.issued_at||!['open','password','invite'].includes(a.mode)||!await verifiedSignature(a.server,a.signature,'connect/server-access/1',[a.server,a.issued_at,a.mode,a.salt,a.nonce,a.ciphertext]))throw Error('Serverns åtkomstinställningar kunde inte verifieras.');
  if(a.mode==='open')prepare('');
  else if(a.mode==='password'){$('status').textContent='Skriv serverns lösenord för att fortsätta.';$('password-box').hidden=false;$('password').focus();}
  else $('status').textContent='Den här servern använder privata inbjudningar. Be ägaren om en inbjudan eller att aktivera ett serverlösenord.';
 }catch(e){$('status').textContent=e.name==='TimeoutError'?'Det tog för lång tid att hämta servern. Försök igen.':e.message||'Kunde inte hämta servern.';$('retry').hidden=false;}
}
$('unlock').onclick=async()=>{
 $('unlock').disabled=true;
 try{
  const a=current.access,password=$('password').value;
  const material=await crypto.subtle.importKey('raw',new TextEncoder().encode(password),'PBKDF2',false,['deriveKey']);
  const key=await crypto.subtle.deriveKey({name:'PBKDF2',hash:'SHA-256',iterations:600000,salt:bytes(a.salt)},material,{name:'AES-GCM',length:256},false,['decrypt']);
  const raw=await crypto.subtle.decrypt({name:'AES-GCM',iv:bytes(a.nonce)},key,bytes(a.ciphertext));
  const token=new TextDecoder().decode(raw);if(!/^[a-f0-9]{64}$/.test(token))throw Error();
  $('password').value='';prepare(token);
 }catch{$('status').textContent='Fel lösenord. Försök igen.';}finally{$('unlock').disabled=false;}
};
$('password').onkeydown=e=>{if(e.key==='Enter'&&!$('unlock').disabled)$('unlock').click();};
$('copy').onclick=async()=>{try{await navigator.clipboard.writeText(invite);$('status').textContent='Kopierad! Klistra in under Anslut i Connect.';}catch{$('manual').open=true;$('invite').focus();$('invite').select();$('status').textContent='Markera och kopiera anslutningen manuellt.';}};
$('retry').onclick=load;load();
