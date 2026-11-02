import {mountPanel} from './app.js';
const root=document.querySelector('#manager'), panel=document.querySelector('#panel');
const el=(tag,text,cls)=>{const n=document.createElement(tag);if(text!==undefined)n.textContent=text;if(cls)n.className=cls;return n;};
const field=(title,value='',type='text')=>{const l=el('label',title),i=el('input');i.type=type;i.value=value;l.append(i);return[l,i];};
let csrf=sessionStorage.getItem('connect-operator-csrf')||'',busy=false,current;
const error=el('p','','error');error.setAttribute('role','alert');
async function request(path,value){const res=await fetch(path,{method:'POST',headers:{'content-type':'application/json','x-connect-csrf':csrf},body:JSON.stringify(value)});if(!res.ok)throw Error(await res.text());let result=await res.json();while(result.job){await new Promise(resolve=>setTimeout(resolve,500));const poll=await fetch('/local/action',{method:'POST',headers:{'content-type':'application/json','x-connect-csrf':csrf},body:JSON.stringify({action:'job',id:result.job})});if(!poll.ok)throw Error(await poll.text());result=await poll.json();}return result;}
function button(text,fn,cls){const b=el('button',text,cls);b.type='button';b.disabled=busy;b.onclick=async()=>{if(busy)return;busy=true;root.setAttribute('aria-busy','true');root.querySelectorAll('button').forEach(n=>n.disabled=true);error.textContent='Arbetar…';try{await fn();}catch(e){error.textContent=e.message;}finally{busy=false;root.setAttribute('aria-busy','false');root.querySelectorAll('button').forEach(n=>n.disabled=false);if(error.textContent==='Arbetar…')error.textContent='';}};return b;}
async function action(value){current=await request('/local/action',value);await render();}
function saveFile(name,text){const url=URL.createObjectURL(new Blob([text],{type:'text/plain'})),a=el('a');a.href=url;a.download=name;a.click();setTimeout(()=>URL.revokeObjectURL(url),1000);}
function restoreForm(){const box=el('details');box.append(el('summary','Återställ en säkerhetskopia'));const[b,backup]=field('Fullständig sökväg till säkerhetskopian (.age)'),[k,key]=field('Fullständig sökväg till återställningsnyckeln (.txt)');box.append(el('p','Återställning kräver en ny, tom datamapp. Befintlig serverdata skrivs aldrig över.'),b,k,button('Återställ server',()=>action({action:'restore',backup:backup.value.trim().replace(/^"|"$/g,''),key_file:key.value.trim().replace(/^"|"$/g,'')})));return box;}
async function render(){
 root.replaceChildren();panel.replaceChildren();
 const head=el('header'),title=el('div');title.append(el('small','CONNECT / SERVER'),el('h1',current.configured?'Din server':'Välkommen till Connect Server'));head.append(title,el('span',current.running?'Servern körs':current.configured?'Servern är stoppad':'Första starten','badge'));root.append(head,error);error.textContent=current.error||'';
 if(!current.configured){
  const setup=el('section',undefined,'card stack'),[n,name]=field('Vad ska servern heta?','Min server'),[a,alias]=field('Serveradress','@mittnamn/mitt-rum');
  setup.append(el('h2','Skapa din server'),el('p','Ingen inloggning behövs för att administrera servern på den här datorn. När den är skapad kan du bjuda in vänner med en länk.'),n,a,el('small','Adressen sparas här. Registrering av ett profilägt namn i adressboken görs fortfarande med ägarens Connect-profil.'),button('Skapa och starta server',()=>action({action:'setup',name:name.value,alias:alias.value}),'primary'));root.append(setup,restoreForm());return;
 }
 const bar=el('div',undefined,'toolbar');bar.append(button(current.running?'Stoppa servern':'Starta servern',async()=>{if(current.running&&!confirm('Stoppa servern? Anslutna personer kopplas från.'))return;await action({action:current.running?'stop':'start'});},current.running?'danger':'primary'),button('Uppdatera status',()=>action({action:'status'})));root.append(bar,el('p','Servern fortsätter köra när du stänger webbläsarfliken. Ändringar här gäller servern.'));
 const settings=el('details');settings.append(el('summary','Drift och säkerhetskopiering'),el('p','Data sparas i '+current.data),el('p','Adress: '+current.config.alias));
 const access=el('label'),checkbox=el('input');checkbox.type='checkbox';checkbox.checked=current.config.public;checkbox.disabled=current.running;access.append(checkbox,document.createTextNode(' Tillåt anslutning utan inbjudan'));settings.append(access,el('small','Stoppa servern för att ändra åtkomst. En inbjudningslänk fungerar även när servern är privat.'),button('Spara åtkomst',()=>action({action:'configure',public:checkbox.checked})));
 settings.append(el('h2','Koppla en ägare i Connect'),el('p','Valfritt: koppla din Connect-profil för att administrera på distans och registrera ditt profilägda servernamn. Lokal administration fungerar utan detta.'),button('Skapa första ägarlänken',async()=>{const result=await request('/local/action',{action:'owner_invite'});const link=el('textarea',undefined,'link-result');link.readOnly=true;link.value=result.invite;settings.append(el('p','Hemlig engångslänk, giltig i en timme. Klistra in i din egen Connect-app, inte hos en kompis.'),link);link.select();}));
 const [label,recipient]=field('Publik backupnyckel (age1…)');
 settings.append(el('h2','Krypterad säkerhetskopia'),el('p','Skapa en återställningsnyckel och spara den separat. Med samma publika nyckel kan du sedan göra fler säkerhetskopior.'),button('Skapa och spara återställningsnyckel',async()=>{const key=await request('/local/action',{action:'recovery_key'});recipient.value=key.public;saveFile('Connect-aterstallningsnyckel.txt',key.private+'\n');error.textContent='Nyckelfilen har skickats till webbläsarens nedladdningar. Spara den säkert innan du gör backup.';}),label,button('Skapa säkerhetskopia',async()=>{const result=await request('/local/action',{action:'backup',recipient:recipient.value.trim()});error.textContent='Säkerhetskopian är sparad: '+result.path;}));root.append(settings);
 if(current.running)await mountPanel(panel,value=>request('/local/control',value),{localOperator:true});
}
async function start(){try{
 const key=location.hash.slice(1);history.replaceState(null,'',location.pathname);
 if(key){const login=await request('/local/login',{key});csrf=login.csrf;sessionStorage.setItem('connect-operator-csrf',csrf);}
 await action({action:'status'});
}catch(e){
 const box=el('section',undefined,'login card stack'),[label,key]=field('Administrationsnyckel','','password');key.autocomplete='off';
 box.append(el('small','CONNECT / SERVER'),el('h1','Öppna serverpanelen'),el('p','Klistra in nyckeln från serverinstallationen. Du behöver inget Connect-konto.'),label,button('Öppna panelen',async()=>{const login=await request('/local/login',{key:key.value.trim()});key.value='';csrf=login.csrf;sessionStorage.setItem('connect-operator-csrf',csrf);await action({action:'status'});},'primary'),error,el('small','Windows: du kan också öppna panelen genom Starta Server.bat.'));root.replaceChildren(box);key.focus();
 } }
start();
