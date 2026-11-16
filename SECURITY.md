# Reporting a vulnerability

Report suspected vulnerabilities privately: use [Report a vulnerability](https://github.com/jujdontbudge/apple-bridge/security/advisories/new) on GitHub, which opens a private advisory only maintainers can see. Do not open a public issue containing exploit details, credentials or personal data.

Include the affected revision, platform, reproduction using synthetic data, expected behavior and observed impact. Remove tokens, personal records and private host addresses from logs and attachments. Maintainers will assess the report and coordinate a fix and disclosure in the private advisory.

Apple Bridge is in early access. Only the latest main revision is currently maintained; no released-version support window or response-time guarantee is established. Review permissions and requested access before connecting apps.

## Out of scope today

Malware already running under the same macOS account can reach Apple Bridge's data. The Helper's launch check narrows this access but does not remove it: malware can run the genuinely signed `bridge` against a home it controls. On a writable copy of the app, it could also swap a file between the check and launch. Whether macOS passes the Helper's permissions to child processes is untested.
