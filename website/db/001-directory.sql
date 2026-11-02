CREATE TABLE IF NOT EXISTS connect_directory_owners (owner text PRIMARY KEY, authority text NOT NULL);
CREATE TABLE IF NOT EXISTS connect_server_names (name text PRIMARY KEY, server text NOT NULL);
CREATE TABLE IF NOT EXISTS connect_name_claim_limits (bucket text PRIMARY KEY, day bigint NOT NULL, claims integer NOT NULL);
CREATE TABLE IF NOT EXISTS connect_directory_records (
 key text PRIMARY KEY, version smallint NOT NULL, owner text NOT NULL, slug text NOT NULL,
 authority text NOT NULL, server text NOT NULL, issued bigint NOT NULL, delegated bigint NOT NULL,
 record jsonb NOT NULL
);
CREATE INDEX IF NOT EXISTS connect_directory_owner ON connect_directory_records(owner);
CREATE TABLE IF NOT EXISTS connect_directory_limits (bucket text PRIMARY KEY, minute bigint NOT NULL, hits integer NOT NULL);

-- All registration checks and the write occur in one DB transaction. This lock
-- also makes first-claim ownership and capacity checks safe across cold starts.
CREATE OR REPLACE FUNCTION connect_directory_register(p jsonb, ip_bucket text) RETURNS integer
LANGUAGE plpgsql AS $$
DECLARE old connect_directory_records; claimed text; named_server text; hit integer; current_minute bigint; current_day bigint; new_claim boolean := false;
BEGIN
 PERFORM pg_advisory_xact_lock(930247103);
 -- Location records have no namespace ownership. Once outside the accepted
 -- signature lifetime they can be safely reclaimed, unlike named claims.
 DELETE FROM connect_directory_records WHERE version=3 AND issued < extract(epoch from now())-180;
 current_minute := floor(extract(epoch from now())/60);
 DELETE FROM connect_directory_limits WHERE minute < current_minute-2;
 INSERT INTO connect_directory_limits VALUES(ip_bucket,current_minute,1)
 ON CONFLICT(bucket) DO UPDATE SET minute=current_minute,
 hits=CASE WHEN connect_directory_limits.minute=current_minute THEN connect_directory_limits.hits+1 ELSE 1 END
 RETURNING hits INTO hit;
 IF hit>90 THEN RETURN 429; END IF;
 IF (p->>'issued')::bigint > extract(epoch from now())+30 OR (p->>'issued')::bigint < extract(epoch from now())-180 THEN RETURN 400; END IF;
 IF (p->>'version')::int=3 AND p->'record' ? 'access' THEN
  SELECT server INTO named_server FROM connect_server_names WHERE name=p->>'slug';
  new_claim := named_server IS NULL;
  IF FOUND AND named_server<>p->>'server' THEN RETURN 409; END IF;
  IF named_server IS NULL AND (SELECT count(*) FROM connect_server_names)>=10000 THEN RETURN 503; END IF;
  IF NOT FOUND AND (SELECT count(*) FROM connect_server_names WHERE server=p->>'server')>=8 THEN RETURN 429; END IF;
 END IF;
 SELECT * INTO old FROM connect_directory_records WHERE key=p->>'key';
 IF FOUND THEN
  IF old.issued >= (p->>'issued')::bigint THEN RETURN 409; END IF;
  IF old.version=2 AND (p->>'version')::int=1 THEN RETURN 409; END IF;
  IF old.version=2 AND old.server<>p->>'server' AND old.delegated >= (p->>'delegated')::bigint THEN RETURN 409; END IF;
 ELSE
  IF (SELECT count(*) FROM connect_directory_records WHERE (version=3)=((p->>'version')::int=3))>=10000 THEN RETURN 503; END IF;
  IF (SELECT count(*) FROM connect_directory_records WHERE authority=p->>'authority')>=32 THEN RETURN 429; END IF;
 END IF;
 IF (p->>'version')::int<>3 THEN
  SELECT authority INTO claimed FROM connect_directory_owners WHERE owner=p->>'owner';
  IF FOUND AND claimed<>p->>'authority' THEN RETURN 409; END IF;
  INSERT INTO connect_directory_owners VALUES(p->>'owner',p->>'authority') ON CONFLICT DO NOTHING;
 END IF;
 IF new_claim THEN
  current_day := floor(extract(epoch from now())/86400);
  DELETE FROM connect_name_claim_limits WHERE day < current_day-1;
  INSERT INTO connect_name_claim_limits VALUES(ip_bucket,current_day,1)
  ON CONFLICT(bucket) DO UPDATE SET day=current_day,claims=CASE WHEN connect_name_claim_limits.day=current_day THEN connect_name_claim_limits.claims+1 ELSE 1 END RETURNING claims INTO hit;
  IF hit>20 THEN RETURN 429; END IF;
 END IF;
 INSERT INTO connect_directory_records VALUES(p->>'key',(p->>'version')::smallint,p->>'owner',p->>'slug',p->>'authority',p->>'server',(p->>'issued')::bigint,(p->>'delegated')::bigint,p->'record')
 ON CONFLICT(key) DO UPDATE SET version=excluded.version,owner=excluded.owner,slug=excluded.slug,authority=excluded.authority,server=excluded.server,issued=excluded.issued,delegated=excluded.delegated,record=excluded.record;
 IF (p->>'version')::int=3 AND p->'record' ? 'access' THEN
  INSERT INTO connect_server_names VALUES(p->>'slug',p->>'server') ON CONFLICT DO NOTHING;
 END IF;
 RETURN 204;
END $$;
