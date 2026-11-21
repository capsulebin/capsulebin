# Security policy

Report security issues privately — never in a public issue, discussion or pull request, where they
would be readable before a fix exists.

**Use GitHub's private vulnerability reporting:** on the repository's **Security** tab, choose
**Report a vulnerability**. The report, the discussion and a draft advisory stay visible only to you
and the maintainers until a fix is published.

## Supported versions

Security fixes go into the latest release of the current major version. Run the latest release;
older releases do not receive backports.

| Version | Supported |
| --- | --- |
| 1.x (latest release) | yes |

## What counts as a vulnerability

Anything that breaks a promise from the [threat model](docs/threat-model.md), in particular:

- plaintext, a link secret, a password or a key reaching the server, the database, a backup or a
  log;
- retrieving or deleting a capsule without the matching capability, or consuming a burn capsule
  more than once;
- learning whether a paste id exists from any response, status, header or measurable timing;
- script execution through rendered Markdown, code or file names (XSS, CSP or Trusted Types bypass);
- bypassing creation tokens, size limits, rate limits or moderation;
- a crafted capsule, public header or link that crashes or misleads the client.

Known, documented limits are not vulnerabilities: a compromised server can ship modified
JavaScript to web users; there is no physical erasure; someone holding both the link and the
ciphertext can guess a password offline.

## Reporting

Include the affected version or commit, the steps or a minimal proof of concept, and the impact you
see. Do not test against instances you do not operate, and never include real share links or keys.

## Handling

1. Confirm and assess against the threat model.
2. Fix on a private branch with a regression test; the test is part of the fix.
3. Release, publish a GitHub security advisory (with a CVE where warranted) and tell operators what
   to do (upgrade, rotate creation tokens, purge, …). Reporters are credited unless they prefer not
   to be.
4. Update the threat model if the issue revealed a gap in it.

Dependency advisories are tracked with `cargo deny` and `npm audit`. An accepted
advisory without a fix is listed with its reasoning and removal condition in `deny.toml`.
