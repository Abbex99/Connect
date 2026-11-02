import {readFile} from 'node:fs/promises';
import {database} from '../lib/directory-store.mjs';
const sql=database();
const source=await readFile(new URL('../db/001-directory.sql',import.meta.url),'utf8');
const boundary=source.indexOf('-- All registration');
const statements=source.slice(0,boundary).split(';').map(s=>s.trim()).filter(Boolean);
statements.push(source.slice(boundary));
await sql.transaction(statements.map(statement=>sql.query(statement)));
console.log('Directory schema applied. No user data changed.');
