# Eilmer DNS and mail operations

Sanitized operational evidence from 2026-11-08 UTC. DNS and mail configuration qualified publicly; actual forwarded-message delivery is unverified. No outbound test messages were authorized or sent. This is not native Apple qualification.

## Configuration

- Cloudflare Free zone `eilmer.si`, nameservers `dean.ns.cloudflare.com` and `raegan.ns.cloudflare.com`.
- Namecheap Custom DNS persisted with both nameservers after reload; domain status ACTIVE, no pending verification/action banner. No repeat save or DNSSEC/security changes.
- Apex A `136.65.0.235`, DNS-only, TTL 300; `www` DNS-only CNAME to apex.
- Public Cloud DNS zone `applebridge-eilmer-si` in project `excerlock`, DNS name `applebridge.eilmer.si.`. Parent delegation TTL 300:

```
ns-cloud-c1.googledomains.com.
ns-cloud-c2.googledomains.com.
ns-cloud-c3.googledomains.com.
ns-cloud-c4.googledomains.com.
```

- Child address records belong to the relay owner; this ticket did not mutate them.
- DNS-only CNAME `_acme-challenge.eilmer.si` to `_acme-challenge.site.applebridge.eilmer.si`, TTL 300. Exact mapping communicated to the relay owner for explicit ACME alias handling in Cloud DNS; no Cloudflare credential distributed.
- Enabled literal forwarding rules for `support@eilmer.si` and `security@eilmer.si` to the owner's verified Gmail destination. Private destination omitted.
- Cloudflare Email Routing enabled after activation; API reports `enabled: true`, `status: ready`. A transient `synced: false` immediately after enablement subsequently resolved to `synced: true`; final read-only status is enabled, synced and ready.
- MX: priority 26 `route2.mx.cloudflare.net`, 43 `route1.mx.cloudflare.net`, 47 `route3.mx.cloudflare.net`.
- Apex TXT `v=spf1 include:_spf.mx.cloudflare.net ~all`; generated public DKIM at `cf2024-1._domainkey`.
- `_dmarc` TXT `v=DMARC1; p=reject; rua=mailto:security@eilmer.si`.
- Custom apex CAA `0 issue "letsencrypt.org"`. Public responses also include Cloudflare's managed-certificate CAA issuers; this is not an exclusive Let's Encrypt policy.

## Activation and public evidence

Namecheap values were saved around 22:08 UTC. At 22:38 UTC a reload still showed those values and ACTIVE status; registry `b.dns.si` still advertised Namecheap. No additional registrar action was visible.

Authorized bounded monitor: maximum 60 minutes / 13 samples, five-minute intervals, waits capped at 60 seconds. Six read-only samples ran from 22:39 to 23:03:54 UTC, with no transport errors. First five were normally pending. Sample six found registry `b.dns.si` delegated to Cloudflare, Cloudflare zone active, and independent resolver `8.8.8.8` returning Cloudflare NS. Monitor stopped on success after about 25 minutes. Parent and relay owner were notified immediately; no Chrome use during monitoring.

At 23:05 UTC Email Routing enablement succeeded. Read-only rule inspection confirmed both exact role addresses enabled with forwarding actions and a verified destination.

Public recursive checks following activation:

| Record | Observed public evidence |
| --- | --- |
| Parent NS | `1.0.0.1` returned dean/raegan Cloudflare NS; registry confirmed delegation |
| Apex A | `8.8.8.8` returned `136.65.0.235` |
| MX | `1.0.0.1`, `9.9.9.9`, `8.8.4.4` returned all three Cloudflare MX records |
| SPF | `8.8.8.8` returned the Cloudflare SPF value above |
| DMARC | `8.8.8.8` returned the exact reject/report policy above |
| DKIM | `8.8.8.8` returned the generated RSA public key |
| CAA | `8.8.8.8` returned Let's Encrypt plus managed-certificate issuers |
| ACME alias | `8.8.8.8` returned the exact CNAME above; both Cloudflare authorities also verified |
| Child delegation | `8.8.8.8` returned all four Google NS and Cloud DNS SOA |

Resolver caches were mixed: a later `8.8.8.8` response returned old Namecheap NS and MX despite its earlier new NS response; `9.9.9.9` and `8.8.4.4` retained old NS but returned new MX. `1.1.1.1` also retained old NS at activation. This is cache lag, not universal convergence. No cache flush or settings mutation was attempted.

Reproduce with `dig @b.dns.si eilmer.si NS +norecurse`, `dig @1.0.0.1 eilmer.si NS`, and public recursive queries for each row above. Querying a configured authority alone does not prove recursive reachability.

## Repository verification and limits

Final-tree `make lint` and `make features` run on authorized Pro with a unique scratch checkout, private Cargo target outside the checkout, and four build jobs. Lint includes 4 source-registration tests, 11 relay packaging tests, 34 npm packaging tests and capture switch checks. Feature output: 79 features; 78 done, 1 in progress; scenario-levels passing 434/475. No feature status changed. These checks do not qualify DNS effects.

The relay owner separately owns deployment, TLS and live migration qualification. Actual mail delivery requires explicit test-recipient authorization and remains unverified. Do not commit message contents or private evidence. Public configuration proof does not assert all resolver caches have expired.
