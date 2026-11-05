# Reporting a vulnerability

Report suspected vulnerabilities privately using [GitHub private vulnerability reporting](https://github.com/JohanSardar/apple-bridge/security/advisories/new). Do not open a public issue containing exploit details, credentials or personal data.

Include the affected revision, platform, reproduction using synthetic data, expected behavior and observed impact. Remove tokens, personal records and private host addresses from logs and attachments. Maintainers will assess the report and coordinate a fix and disclosure through the private advisory.

Apple Bridge is in early access. Only the latest main revision is currently maintained; no released-version support window or response-time guarantee is established. Review permissions and requested access before connecting apps.

## Out of scope today

Malware already running under the same macOS account can reach Apple Bridge's data. The Helper's launch check narrows this access but does not remove it: malware can run the genuinely signed `bridge` against a home it controls. On a writable copy of the app, it could also swap a file between the check and launch. Whether macOS passes the Helper's permissions to child processes is untested.

A relay's operator controls its DNS and could obtain a certificate for a Mac's address. `bridge doctor` and `bridge remote status` can only detect this after the fact.
