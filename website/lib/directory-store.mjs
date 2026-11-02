import {neon} from '@neondatabase/serverless';
export function database(){
 if(!process.env.DATABASE_URL)throw new Error('Directory database not configured');
 return neon(process.env.DATABASE_URL,{fetchOptions:{signal:AbortSignal.timeout(6000)}});
}
export async function register(value,bucket){
 const sql=database();
 const rows=await sql`SELECT connect_directory_register(${JSON.stringify(value)}::jsonb,${bucket}) AS status`;
 return rows[0].status;
}
export async function lookup(key,version){
 const sql=database();
 const rows=await sql`SELECT record FROM connect_directory_records WHERE key=${key} AND version=${version} AND issued>=extract(epoch from now())-180 LIMIT 1`;
 return rows[0]?.record;
}
export async function byOwner(owner){
 const sql=database();
 const rows=await sql`SELECT record FROM connect_directory_records WHERE owner=${owner} AND version=1 AND issued>=extract(epoch from now())-180 ORDER BY key LIMIT 32`;
 return rows.map(r=>r.record);
}
export async function byServerName(name){
 const sql=database();
 const rows=await sql`SELECT r.record FROM connect_server_names n JOIN connect_directory_records r ON r.key='id:'||n.server WHERE n.name=${name} AND r.version=3 AND r.issued>=extract(epoch from now())-180 LIMIT 1`;
 return rows[0]?.record;
}
