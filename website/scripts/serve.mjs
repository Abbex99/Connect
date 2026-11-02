import http from 'node:http';
import {readFile, stat} from 'node:fs/promises';
import {resolve, extname, sep} from 'node:path';
const root = resolve(new URL('../public', import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, '$1'));
const types = {'.html':'text/html; charset=utf-8','.css':'text/css; charset=utf-8','.js':'text/javascript; charset=utf-8','.png':'image/png','.svg':'image/svg+xml','.woff2':'font/woff2','.zip':'application/zip','.json':'application/json','.txt':'text/plain; charset=utf-8'};
const server = http.createServer(async(req,res)=>{
  try {
    const pathname = decodeURIComponent(new URL(req.url,'http://localhost').pathname);
    let file=resolve(root,'.'+pathname);
    if(file!==root && !file.startsWith(root+sep)){res.writeHead(403);res.end();return;}
    if(pathname.endsWith('/')) file=resolve(file,'index.html');
    else if(!extname(file)) file+='.html';
    const info=await stat(file);
    if(!info.isFile()) throw new Error('not file');
    res.writeHead(200,{'Content-Type':types[extname(file)]||'application/octet-stream','Content-Length':info.size,'X-Content-Type-Options':'nosniff'});
    res.end(req.method==='HEAD'?undefined:await readFile(file));
  } catch {res.writeHead(404);res.end('Sidan finns inte.');}
});
server.listen(Number(process.env.PORT||4780),'127.0.0.1',()=>console.log('Connect website: http://127.0.0.1:'+server.address().port));
