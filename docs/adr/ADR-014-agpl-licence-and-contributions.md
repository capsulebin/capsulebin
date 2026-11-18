# ADR-014 — AGPL licence, DCO for contributions

**Status:** Accepted

## Context

CapsuleBin is a zero-knowledge product: its central promise is that the operator cannot read what is
shared. That promise is only as credible as the ability of outsiders to check it — the crypto
profile, the capsule format, the client that performs the encryption, and the server that must
never receive plaintext. Independent audits and users who want to verify the claims themselves both
need the source.

## Decision

- **CapsuleBin is open source under AGPL-3.0-only**, as declared in `LICENSE` and the package
  manifests: server, web client, CLI, `@capsulebin/core`, the specifications and the test vectors.
  Every capability of the paste service is in this repository; there is no closed edition.
- **Contributions are accepted under the same licence** ("inbound = outbound") with a **Developer
  Certificate of Origin** sign-off (`git commit -s`, see `CONTRIBUTING.md`): the contributor
  certifies they may contribute the code under the project licence and keeps the copyright.

## Rejected

- **Closed source with commissioned audits.** Possible, but each audit is a snapshot, and users of a
  zero-knowledge product reasonably expect to be able to look themselves.
- **Open clients, closed server.** The security-critical code is mostly client-side, but a closed
  server makes claims such as "never logs a paste id" or "answers moderated and unknown ids alike"
  unverifiable.
- **Source-available licences (BSL, FSL, ELv2).** Not open source, and the trust argument is weaker.
- **A permissive licence (MIT, Apache-2.0).** Would let a modified CapsuleBin be run as a service
  without its changes being published. For a product whose users must be able to check what a
  server does, the AGPL's network clause (§13) is the point: whoever runs a modified version must
  offer its source to the people using it.
- **A contributor licence agreement (CLA).** It would keep relicensing open, but is a barrier for
  contributors and, for a project that earns trust by being open, reads as a reservation to close
  it later.

## Consequences

- Anyone may run CapsuleBin, also as a service for others; whoever modifies it must offer the
  modified source to its users.
- Without a CLA, contributed code can only ever be used under the AGPL. Should dual licensing or a
  relicensing be wanted one day, an agreement has to be in place before the first outside
  contribution; it cannot be applied retroactively.
- The licence covers the code, not the name "CapsuleBin" or the logo (see `docs/brand/README.md`).
- Vulnerabilities are reported privately through GitHub's private vulnerability reporting, as
  described in `SECURITY.md`.
