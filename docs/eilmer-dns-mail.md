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
4. Send explicitly authorized tagged test messages to both public role addresses and verify receipt at the owner destination. Do not commit message contents or private mail evidence.
5. Have the relay owner prove live migration against the delegated zone. This ticket does not modify relay runtime, customer records, credentials, or product behavior.

Reproduce authoritative checks with `dig @b.dns.si eilmer.si NS +norecurse` and `dig @dean.ns.cloudflare.com applebridge.eilmer.si NS +norecurse`. A configured authority alone is not proof that recursive clients reach it.
