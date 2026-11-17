# Security policy

## Supported versions

Security fixes go into the latest release. Please update before reporting.

## Reporting a vulnerability

Report it privately through GitHub: on this repository's **Security** tab, choose
**Report a vulnerability**. Please don't open a public issue.

Include the EQ version (`eq --version`), your macOS version, and the steps that show the
problem. You'll get a reply within a week, and a fix or a plan as soon as the issue is
confirmed.

## What EQ has access to

EQ holds macOS's System Audio Recording permission so that it can equalize what your Mac
plays. It processes audio on your Mac only, makes no network connections, and stores
nothing but its settings in `~/Library/Application Support/eq/`. Releases are built by
GitHub Actions from tagged source, signed with the hardened runtime, and published with
a SHA-256 checksum.
