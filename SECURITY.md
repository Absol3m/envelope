# Security policy

## Report a vulnerability
**Do not publish it in an issue.** Use GitHub's private reporting: the repository's *Security* tab → *Report a vulnerability* (if it doesn't appear, open an issue asking for a private contact, without technical details). Include the Envelope version, your system, the steps to reproduce and the impact you see. No real credentials or real emails in the report.

Envelope is maintained by one person: response and fix on a best-effort basis, with no guaranteed delay. The reporter is credited in the release notes if they wish.

## Supported versions
Envelope is a pre-release: only the latest published version receives fixes.

## What is considered sensitive
- Email content is **untrusted** input (HTML, headers, attachments, IMAP folder names): any code execution, data leak or bypass of the remote-content blocking that results from it is a vulnerability.
- Secrets (passwords, OAuth2 tokens): system keychain only, never in the database or in logs.
- TLS certificates: always verified by the system, with no global exception.
- MCP server: read-only on the database; any email content it returns is flagged as untrusted.

## Data at rest
Messages, their search index and attachments are stored **in clear text** in the application's data folder: Envelope does not encrypt its files (like Thunderbird, Apple Mail or Outlook). Protection relies on:
- the **system's disk encryption** (FileVault, BitLocker, LUKS): a badge in the sidebar shows whether it is on;
- **file permissions**: on Unix the data folder is `0700` and files are `0600` (owner only, no group, no others; `root` always bypasses permissions); on Windows, the access control lists inherited from the profile. Exports (mbox, eml) follow the `umask` of the chosen folder, and an MCP server started by another account can no longer read the database;
- the **application lock** (password, Touch ID, Windows Hello), which is **not** encryption: a program that reads the files directly is not stopped, nor is a backup of the profile;
- secrets (passwords, OAuth tokens) stay in the system keychain, never in these files.

Remember to encrypt backups of the data folder. Encrypting the database itself was studied and set aside for now (see issue #9).

## Supply chain
Dependencies are monitored by Dependabot and by `cargo deny` (RustSec, licenses, sources) and `pnpm audit` in CI; publishing GitHub Actions are pinned by hash; release files come with `SHA256SUMS` and a provenance attestation (`gh attestation verify <file> --repo Absol3m/envelope`).

## Automatic updates
Official `tauri-plugin-updater` plugin. The app reads `latest.json` from the repository's “updater” Release (HTTPS, certificates verified by the system), then installs an archive only after verifying its signature against the public key in `tauri.conf.json`. The private key stays with the maintainer; in CI only the signing job receives it (it does not run the build). Each signature carries the signed version (`requireSignedVersion`): an old version cannot be served again under a newer number. Binaries are not signed by Apple or Microsoft yet: that signature is independent. The automatic check (at startup, then daily) contacts GitHub, which sees the IP address and the version; it can be turned off in Settings → About.
