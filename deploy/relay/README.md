# Relay deployment

Build `cmd/relay` for linux/amd64 on the Pro. Install as `/usr/local/bin/ab-relay`; create system user `ab-relay`, install config and the control TLS certificate/key under `/etc/ab-relay` (directory 0750, key 0640, group ab-relay), then enable `ab-relay.service`.

The public ciphertext listener is 443. The authenticated TLS control API and outbound device tunnel share port 443 using control-host SNI routing. Configure `APPLE_BRIDGE_RELAY_URL=https://control.ab.lexaai.tech` on the Mac. The control certificate must be trusted by the Mac; self-signed bootstrap certificates do not enable production enrollment.

Cloud DNS uses Application Default Credentials from the VM service account. Project: `excerlock`; managed zone: `ab-lexaai-tech`. Required role if denied: `roles/dns.admin` on that zone (or equivalent custom permissions for record-set list/create/update/delete and changes create/get). Do not modify IAM during deployment.

Delegate `ab.lexaai.tech` from Cloudflare to the four name servers reported by `gcloud dns managed-zones describe ab-lexaai-tech --project excerlock`. Create `*.ab.lexaai.tech` and `control.ab.lexaai.tech` A records for the VM. No public readiness claim is possible until delegation and trusted control TLS are established.

ACME rate-limit override request: “Apple Bridge assigns a stable hostname per device under ab.lexaai.tech. End-to-end TLS keys are generated and retained on each Mac. Please raise the registered-domain issuance limit for our device enrollment service. We renew only within 30 days of expiration, reuse existing certificates and stable device names, and use Pebble/staging for tests. Expected device count and daily issuance volume: operator to supply before submission.”
