# Reporting a vulnerability

Report suspected vulnerabilities privately: [open a new issue](https://gitlab.com/johansardar/apple-bridge/-/issues/new) and tick **This issue is confidential** before you submit it, so only maintainers can see it. Do not open a public issue containing exploit details, credentials or personal data.

Include the affected revision, platform, reproduction using synthetic data, expected behavior and observed impact. Remove tokens, personal records and private host addresses from logs and attachments. Maintainers will assess the report and coordinate a fix and disclosure in the confidential issue.

Apple Bridge is in early access. Only the latest main revision is currently maintained; no released-version support window or response-time guarantee is established. Review permissions and requested access before connecting apps.

## Out of scope today

Malware already running under the same macOS account can reach Apple Bridge's data. The Helper's launch check narrows this access but does not remove it: malware can run the genuinely signed `bridge` against a home it controls. On a writable copy of the app, it could also swap a file between the check and launch. Whether macOS passes the Helper's permissions to child processes is untested.
