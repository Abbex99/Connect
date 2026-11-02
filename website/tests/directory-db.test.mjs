import test from 'node:test';
import assert from 'node:assert/strict';
import {randomBytes,sign} from 'node:crypto';
import {fixture,withAccess} from './directory.test.mjs';
import {verifyRegistration} from '../lib/directory-protocol.mjs';
import {database,register,lookup,byServerName} from '../lib/directory-store.mjs';
test('persistent ownership race, replay, expired lookup, isolated server ID',async()=>{
 const owner='qa-'+randomBytes(8).toString('hex'),bucket=owner;
 const a=fixture(owner),b=fixture(owner),sql=database();
 try{
  const one=verifyRegistration(1,a.signed),two=verifyRegistration(1,b.signed);
  const outcomes=await Promise.all([register(one,bucket),register(two,bucket)]);
  assert.deepEqual([...outcomes].sort(),[204,409]);
  const winner=outcomes[0]===204?one:two;
  assert.equal((await lookup(winner.key,1)).record.address.id,winner.server);
  assert.equal(await register(winner,bucket),409);
  const location=withAccess(fixture(owner,undefined,3));
  const v3=verifyRegistration(3,location.signed);assert.equal(await register(v3,bucket),204);
  assert.equal((await lookup(v3.key,3)).record.address.id,location.id);
  assert.equal((await byServerName(owner)).record.address.id,location.id);
  const thief=withAccess(fixture(owner,undefined,3));
  assert.equal(await register(verifyRegistration(3,thief.signed),bucket),409,'A duplicate friendly name must not be reassigned');
  assert.equal((await byServerName(owner)).record.address.id,location.id);
  await sql`UPDATE connect_directory_records SET issued=0 WHERE key=${v3.key}`;
  assert.equal(await lookup(v3.key,3),undefined);
  const replacement=verifyRegistration(3,fixture(owner,undefined,3).signed);
  assert.equal(await register(replacement,bucket),204);
  const expired=await sql`SELECT count(*)::int AS count FROM connect_directory_records WHERE key=${v3.key}`;
  assert.equal(expired[0].count,0,'Expired automatic identities must release capacity');
  const claims=await sql`SELECT authority FROM connect_directory_owners WHERE owner=${owner}`;
  assert.equal(claims[0].authority,winner.authority,'Namespace ownership must survive cleanup');
  // Tampered records never reach the storage API.
  assert.throws(()=>verifyRegistration(1,{...a.signed,signature:b.signed.signature}));
 }finally{
  await sql`DELETE FROM connect_directory_records WHERE owner=${owner}`;
  await sql`DELETE FROM connect_directory_owners WHERE owner=${owner}`;
  await sql`DELETE FROM connect_server_names WHERE name=${owner}`;
  await sql`DELETE FROM connect_directory_limits WHERE bucket=${bucket}`;
  await sql`DELETE FROM connect_name_claim_limits WHERE bucket=${bucket}`;
 }
});
