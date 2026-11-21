# Security policy

pu-get downloads files from official public sources and runs mostly in the
terminal, but it still handles network responses, catalogs, paths and filesystem
publication. Please report security issues privately.

## Supported versions

| Version | Supported |
|---|---|
| `0.0.x` | Yes (public pre-release) |

Security fixes are made on the latest release line. Older pre-release versions may
not receive backports.

## Reporting a vulnerability

**Do not open a public issue for a vulnerability.**

Use GitHub's private vulnerability reporting:

1. Open the repository's **Security** tab.
2. Choose **Report a vulnerability**.
3. Include the affected version, platform, impact and a minimal reproduction.

If private reporting is unavailable, open a minimal public issue asking the
maintainers for a private contact channel **without including exploit details**.

Useful reports include:

- affected `pu-get --version`;
- operating system and terminal;
- the command or TUI action involved;
- the relevant log or stack trace, with personal paths redacted;
- whether the issue requires a malicious source, a malicious catalog, or only
  local user interaction.

## Scope

In scope:

- TLS/certificate verification and HTTP request handling;
- path traversal, folder-template abuse or unsafe filename handling;
- file overwrite/conflict behavior, `.part` handling and atomic publication;
- catalog parsing, embedded/user catalog precedence and remote update checks;
- SHA-256 verification and manifest handling;
- denial of service caused by malicious or pathological catalog/source data;
- terminal escape injection or unsafe rendering of catalog content;
- dependency vulnerabilities that affect the shipped binary.

Out of scope:

- broken, moved or removed links on official portals (use the source/broken-link
  issue form);
- the content, licensing or availability of documents published by boards;
- requests to bypass logins, postbacks, paywalls or TLS verification;
- reports requiring a compromised local account or physical access;
- social engineering.

## Disclosure and response

This is a volunteer project; response is best effort. We will acknowledge a
complete private report as soon as practical, investigate, and coordinate a fix
and disclosure date with the reporter. Please give us reasonable time to release a
fix before publishing details.

There is no bug bounty program.

## Hardening expectations

Contributors should preserve these invariants:

- rustls verification is never disabled;
- catalog URLs are absolute HTTP(S);
- folder templates reject absolute paths and `..`;
- filenames are sanitised;
- downloads are checked for PDF content before publication;
- remote catalogs are checksum-verified;
- generated catalogs and resolvers never fabricate URLs.

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md#security-invariants) for details.
