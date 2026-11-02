import {createRequire} from 'node:module';
import assert from 'node:assert/strict';
import {mkdir, readFile, writeFile} from 'node:fs/promises';
import {createHash} from 'node:crypto';
const require=createRequire(import.meta.url);
const {chromium}=require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const base=process.env.TEST_URL||'http://127.0.0.1:4780';
const browser=await chromium.launch({headless:true,...(process.env.CHROMIUM_PATH?{executablePath:process.env.CHROMIUM_PATH}:{})});
await mkdir('test-results',{recursive:true});
const context=await browser.newContext({viewport:{width:1440,height:1000},acceptDownloads:true,reducedMotion:'reduce'});
const page=await context.newPage();
const errors=[];page.on('pageerror',e=>errors.push(e.message));
try {
 await page.goto(base,{waitUntil:'networkidle'});
 await page.screenshot({path:'test-results/desktop-dark.png',fullPage:true});
 for(const width of [1440,1006,768,390,320]) {
   await page.setViewportSize({width,height:900});
   assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),`overflow at ${width}`);
 }
 await page.setViewportSize({width:390,height:844});
 await page.screenshot({path:'test-results/mobile-dark.png',fullPage:true});
 await page.getByRole('button',{name:'Byt till ljust tema'}).click();
 assert.equal(await page.locator('html').getAttribute('data-theme'),'light');
 await page.reload();assert.equal(await page.locator('html').getAttribute('data-theme'),'light');
 await page.screenshot({path:'test-results/mobile-light.png',fullPage:true});
 await page.setViewportSize({width:1440,height:1000});
 await page.screenshot({path:'test-results/desktop-light.png',fullPage:true});
 await page.getByRole('button',{name:'Byt till mörkt tema'}).click();
 await page.locator('[data-view="share"]').click();assert(await page.locator('.share-preview').isVisible());
 await page.locator('[data-view="chat"]').click();assert(await page.locator('.chat-preview').isVisible());
 await page.locator('[data-quality="720p"]').click();assert(await page.locator('.share-preview').isVisible());assert.match(await page.locator('.quality').innerText(),/720p/);
 await page.locator('button[data-skin="rose"]').click();assert.equal(await page.locator('.skin-demo').getAttribute('data-skin'),'rose');
 await page.locator('summary').filter({hasText:'Är Connect verkligen gratis?'}).click();assert(await page.getByText('Ja. Appen och alla funktioner är gratis,',{exact:false}).isVisible());
 await page.locator('[data-download]').first().click();assert(await page.getByRole('dialog').isVisible());
 await page.screenshot({path:'test-results/download-dialog.png'});
 for(let i=0;i<8;i++){await page.keyboard.press('Tab');assert(await page.evaluate(()=>document.querySelector('dialog').contains(document.activeElement)),'dialog focus escaped');}
 await page.keyboard.press('Escape');assert(!(await page.getByRole('dialog').isVisible()));
 assert.equal(await page.evaluate(()=>document.activeElement.hasAttribute('data-download')),true);
 await page.locator('[data-download]').first().click();
 const promise=page.waitForEvent('download');await page.locator('#actual-download').click();const download=await promise;
 assert.equal(await download.failure(),null);const path=await download.path();
 const manifest=JSON.parse(await readFile('public/downloads/releases.json','utf8'));
 const client=manifest.find(x=>x.file.startsWith('Connect-Windows'));
 assert.equal(createHash('sha256').update(await readFile(path)).digest('hex'),client.sha256,'download differs from release');
 await page.keyboard.press('Escape');
 const links=await page.locator('a[href]').evaluateAll(nodes=>nodes.map(n=>n.getAttribute('href')));
 for(const href of new Set(links)){
   if(href.startsWith('#'))assert(await page.locator(href).count(),`missing anchor ${href}`);
   else if(href.startsWith('/')){const response=await context.request.head(base+href);assert(response.ok(),`broken link ${href}: ${response.status()}`);}
 }
 assert.equal(errors.length,0,errors.join('\n'));
 await writeFile('test-results/result.json',JSON.stringify({status:'passed',viewports:[1440,1006,768,390,320],themes:2,downloadHash:client.sha256,errors},null,2));
 console.log('PASS: responsive widths, dark/light persistence, preview, skins, quality, FAQ, dialog keyboard, real ZIP download SHA256 and all internal links.');
}finally{await browser.close();}
