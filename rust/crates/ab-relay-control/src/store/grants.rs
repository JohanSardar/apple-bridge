use super::{
    COMPLETION_SECONDS, Control, DEVICES_PER_ACCOUNT, Error, GRANT_SECONDS, OptionalExtension,
    Result, Value, VerifiedGrant, VerifyingKey, clean_name, decode, hash, held_devices, json,
    normalize_code, params, proof, random, same_network, user_code,
};

impl Control {
    pub(crate) fn start_grant(
        &self,
        public: &str,
        name: &str,
        source: &str,
        now: i64,
    ) -> Result<Value> {
        let key: [u8; 32] = decode(public)?.try_into().map_err(|_| Error::Invalid)?;
        VerifyingKey::from_bytes(&key).map_err(|_| Error::Invalid)?;
        let name = clean_name(name);
        let verification = format!("{}/device", self.public_url);
        self.with(|writer| {
            writer
                .db
                .execute("DELETE FROM grants WHERE expires<=?1", [now])?;
            writer.db.execute(
                "DELETE FROM grant_sources WHERE grant_id NOT IN (SELECT id FROM grants)",
                [],
            )?;
            let waiting: i64 = writer
                .db
                .query_row("SELECT count(*) FROM grants", [], |row| row.get(0))?;
            if waiting >= 4096 {
                return Err(Error::Capacity);
            }
            let id = random(24)?;
            let poll = random(32)?;
            let code = user_code()?;
            let nonce = random(32)?;
            let fingerprint = ab_wire::framing::device_key_fingerprint(&key);
            writer.db.execute(
                "INSERT INTO grants VALUES(?1,?2,?3,?4,?5,?6,?7,0,?8,?9,5,NULL,'pending')",
                params![
                    id,
                    hash(&poll),
                    hash(&code),
                    public,
                    fingerprint,
                    name,
                    nonce,
                    now + GRANT_SECONDS,
                    now + 5
                ],
            )?;
            writer.db.execute(
                "INSERT INTO grant_sources VALUES(?1,?2,?3)",
                params![id, source, now],
            )?;
            Ok(json!({
                "transaction": id,
                "polling_secret": poll,
                "user_code": code,
                "fingerprint": fingerprint,
                "expires_in": GRANT_SECONDS,
                "interval": 5,
                "verification_uri": verification,
                "verification_uri_complete": format!("{verification}?code={code}"),
            }))
        })
    }
    pub(crate) fn grant_view(&self, code: &str, viewer: &str, now: i64) -> Result<Value> {
        let code = normalize_code(code);
        self.with(|writer| {
            writer.db.query_row(
                "SELECT g.fingerprint,g.name,s.source,s.created FROM grants g LEFT JOIN grant_sources s ON s.grant_id=g.id WHERE g.code=?1 AND g.expires>?2 AND g.state='pending'",
                params![hash(&code), now],
                |row| {
                    let source: Option<String> = row.get(2)?;
                    let created: Option<i64> = row.get(3)?;
                    let same_network = source.as_deref().map(|source| same_network(source, viewer));
                    Ok(json!({
                        "code": code,
                        "fingerprint": row.get::<_, String>(0)?,
                        "name": row.get::<_, String>(1)?,
                        "source": source,
                        "same_network": same_network,
                        "age_seconds": created.map(|created| (now - created).max(0)),
                    }))
                },
            )
            .optional()?
            .ok_or(Error::Expired)
        })
    }
    pub fn confirm(&self, account: &str, code: &str, fingerprint: &str, now: i64) -> Result<Value> {
        let code = normalize_code(code);
        self.with(|writer| {
            let held = held_devices(&writer.db, account)?;
            if held >= DEVICES_PER_ACCOUNT {
                return Err(Error::Capacity);
            }
            if writer.db.execute(
                "UPDATE grants SET account=?1,state='approved' WHERE code=?2 AND fingerprint=?3 AND expires>?4 AND state='pending'",
                params![account, hash(&code), fingerprint, now],
            )? != 1
            {
                return Err(Error::Expired);
            }
            Ok(json!({"approved": true}))
        })
    }
    pub fn poll(&self, poll: &str, now: i64) -> Result<Value> {
        self.with(|writer| {
            let row: (String, String, i64, i64, String) = writer
                .db
                .query_row(
                    "SELECT id,public,next_poll,interval,state FROM grants WHERE poll=?1 AND expires>?2",
                    params![hash(poll), now],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
                )
                .optional()?
                .ok_or(Error::Expired)?;
            let (id, public, next, interval, state) = row;
            if state == "consumed" {
                return Err(Error::Expired);
            }
            if now < next {
                writer.db.execute(
                    "UPDATE grants SET interval=min(interval+5,60),next_poll=?1+min(interval+5,60) WHERE id=?2",
                    params![now, id],
                )?;
                return Err(Error::SlowDown);
            }
            writer.db.execute("UPDATE grants SET next_poll=?1 WHERE id=?2", params![now + interval, id])?;
            if state != "approved" {
                return Err(Error::Pending);
            }
            let nonce = random(32)?;
            writer.db.execute(
                "UPDATE grants SET nonce=?1,proof_until=?2 WHERE id=?3",
                params![nonce, now + 30, id],
            )?;
            Ok(json!({"transaction": id, "nonce": nonce, "public_key": public}))
        })
    }
    pub(crate) fn verify_grant(
        &self,
        poll: &str,
        signature: &str,
        now: i64,
    ) -> Result<VerifiedGrant> {
        self.with(|writer| {
            let row: (String, String, String, String, String) = writer
                .db
                .query_row(
                    "SELECT id,nonce,public,name,account FROM grants WHERE poll=?1 AND expires>?2 AND state='approved' AND proof_until>?2",
                    params![hash(poll), now],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
                )
                .optional()?
                .ok_or(Error::Expired)?;
            let (id, nonce, public_key, name, account) = row;
            let message =
                ab_wire::framing::device_link_message(&self.public_url, &id, &nonce, &public_key);
            proof(&public_key, signature, &message)?;
            let tx = writer.db.transaction()?;
            tx.execute("DELETE FROM grant_completions WHERE until<=?1", [now])?;
            let in_progress: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM grant_completions WHERE grant_id=?1)",
                [&id],
                |row| row.get(0),
            )?;
            if in_progress {
                return Err(Error::SlowDown);
            }
            let completing: i64 = tx.query_row(
                "SELECT count(*) FROM grant_completions WHERE account=?1",
                [&account],
                |row| row.get(0),
            )?;
            if held_devices(&tx, &account)? + completing >= DEVICES_PER_ACCOUNT {
                return Err(Error::Capacity);
            }
            tx.execute(
                "INSERT INTO grant_completions VALUES(?1,?2,?3)",
                params![id, account, now + COMPLETION_SECONDS],
            )?;
            tx.commit()?;
            Ok(VerifiedGrant { id, account, public_key, name })
        })
    }
    pub(crate) fn finish_grant(
        &self,
        grant: &VerifiedGrant,
        hostname: &str,
        now: i64,
    ) -> Result<()> {
        self.with(|writer| {
            let tx = writer.db.transaction()?;
            if held_devices(&tx, &grant.account)? >= DEVICES_PER_ACCOUNT {
                return Err(Error::Capacity);
            }
            tx.execute(
                "INSERT INTO devices VALUES(?1,?2,?3,?4,'active',?5)",
                params![hostname, grant.account, grant.public_key, grant.name, now],
            )?;
            tx.execute(
                "UPDATE grants SET state='consumed',nonce='' WHERE id=?1",
                [&grant.id],
            )?;
            tx.execute(
                "DELETE FROM grant_completions WHERE grant_id=?1",
                [&grant.id],
            )?;
            tx.commit()?;
            Ok(())
        })
    }
    pub(crate) fn release_grant(&self, grant: &VerifiedGrant) -> Result<()> {
        self.with(|writer| {
            writer.db.execute(
                "DELETE FROM grant_completions WHERE grant_id=?1",
                [&grant.id],
            )?;
            Ok(())
        })
    }
}
