# Eilmer DNS and mail operations

Operations performed on 2026-11-08 UTC. This is a sanitized operational record, not native Apple qualification.

## Configuration

- Created the Cloudflare Free zone `eilmer.si`.
- Saved Custom DNS at Namecheap: `dean.ns.cloudflare.com` and `raegan.ns.cloudflare.com`; read back both saved values in the dashboard.
- Set apex A to the existing relay VM address, `136.65.0.235`, with DNS-only resolution and TTL 300. `www` is a DNS-only CNAME to the apex.
- Created public Cloud DNS zone `applebridge-eilmer-si` in project `excerlock`, DNS name `applebridge.eilmer.si.`.
- Added the child delegation in Cloudflare, TTL 300:

```
ns-cloud-c1.googledomains.com.
ns-cloud-c2.googledomains.com.
ns-cloud-c3.googledomains.com.
ns-cloud-c4.googledomains.com.
```

- Added DNS-only CNAME `_acme-challenge.eilmer.si` to `_acme-challenge.site.applebridge.eilmer.si`, TTL 300. The relay owner maps the apex ACME challenge explicitly into this Cloud DNS target; no Cloudflare credential is distributed.
- Created enabled literal recipient forwarding rules for `support@eilmer.si` and `security@eilmer.si` to the owner's verified Gmail destination. The private destination is omitted here.
- Installed the zone-specific Cloudflare routing DNS recommendations: three MX records, the generated public DKIM key at `cf2024-1._domainkey`, and `v=spf1 include:_spf.mx.cloudflare.net ~all` at the apex.
- Added `_dmarc` TXT `v=DMARC1; p=reject; rua=mailto:security@eilmer.si` and apex CAA `0 issue "letsencrypt.org"`.

## Observed evidence and remaining gate

The Cloudflare API accepted all records and both rules. The destination status is verified. Direct authoritative queries to `dean.ns.cloudflare.com` return the relay apex A and the four child NS records (the child records appear in the authority section).

At 22:09 UTC the `.si` registry nameserver `b.dns.si` still returned `dns1.registrar-servers.com` and `dns2.registrar-servers.com`, TTL 7200. Cloudflare zone status was `pending`. Email routing activation returned HTTP 403, code 2009, `Active zone required`. Thus public delegation, publicly effective mail policy, and actual forwarding delivery remain **unverified**. Created rules do not prove delivery.

After registry propagation, complete these gates:

1. Confirm `.si` registry delegation and independent recursive NS resolution use Cloudflare.
2. Enable Cloudflare Email Routing and confirm its status is enabled/synced.
3. Query public resolvers for apex A/MX/TXT/CAA, `_dmarc` TXT, DKIM TXT, and child NS/SOA.
4. Actual mail delivery remains unverified: forwarding setup does not authorize outbound test messages. Only send tagged test messages after explicit test-recipient authorization, and never commit message contents or private mail evidence.
5. Have the relay owner prove live migration against the delegated zone. This ticket does not modify relay runtime, customer records, credentials, or product behavior.

Reproduce authoritative checks with `dig @b.dns.si eilmer.si NS +norecurse` and `dig @dean.ns.cloudflare.com applebridge.eilmer.si NS +norecurse`. A configured authority alone is not proof that recursive clients reach it.

## Follow-up verification

At 22:11 UTC both `dean.ns.cloudflare.com` and `raegan.ns.cloudflare.com` answered the exact ACME CNAME above, TTL 300. All six `.si` authorities (`b`, `f`, `h`, `i`, `k`, `l`.dns.si) continued to advertise Namecheap. The public recursive ACME CNAME query returned no answer. The external blocker is the registrar-to-registry nameserver update, not absence of the staged Cloudflare record.

Final-tree repository gates ran on the authorized Pro in a unique scratch checkout with its own `CARGO_TARGET_DIR` and `CARGO_BUILD_JOBS=4`: `make lint` and `make features` passed. Lint includes 4 source-registration tests, 11 relay packaging tests, 34 npm packaging tests and capture switch checks. Feature output: 79 features; 78 done, 1 in progress; scenario-levels passing 434/475. No feature status changed. These checks do not qualify DNS or mail effects.
