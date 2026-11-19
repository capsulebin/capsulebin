# Contributing to CapsuleBin

Thanks for helping. CapsuleBin is a zero-knowledge paste service: the server must never see
plaintext, keys or link secrets. Most review effort goes into keeping it that way, so please read
[AGENTS.md](AGENTS.md) (architecture, invariants, where things live) and the
[threat model](docs/threat-model.md) before a larger change.

Security issues are **not** reported here — see [SECURITY.md](SECURITY.md).

## Licence and sign-off

CapsuleBin is licensed under the [AGPL-3.0-only](LICENSE). Contributions are accepted under the
same licence; you keep the copyright to your work.

Every commit must carry a **Developer Certificate of Origin** sign-off
([developercertificate.org](https://developercertificate.org/)). With it you certify that you wrote
the change, or otherwise have the right to submit it under the project licence:

```sh
git commit -s -m "Explain what changes and why"
```

This adds a line such as `Signed-off-by: Erika Mustermann <erika@example.org>` to the commit
message. It must match the commit author. Forgot it? `git commit --amend -s` for the last commit,
`git rebase --signoff main` for a whole branch.

There is no contributor licence agreement: no rights beyond the AGPL are transferred
([ADR-014](docs/adr/ADR-014-agpl-licence-and-contributions.md)).

## Before you open a pull request

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
npm run format:check
npm run check -w @capsulebin/core && npm test -w @capsulebin/core
npm run check -w @capsulebin/web  && npm test -w @capsulebin/web
npm run check -w @capsulebin/cli  && npm test -w @capsulebin/cli
```

and, for changes to the server or the assets, the acceptance check and the browser tests against a
release build (see [README.md](README.md#tests)).

- **Tests come with the change.** A new data path needs a test that proves no plaintext travels
  along it; a new log line must not carry ids, tokens or report text (`logging.rs` will tell).
- **Keep the invariants.** Unknown, expired, consumed, moderated and wrong-capability answer
  identically; capability checks are constant-time; the public header's serialization is fixed.
  Changing any of them needs an ADR in `docs/adr/`.
- **Protocol changes** (capsule format, crypto profile, public header) need updated golden vectors
  in `docs/test-vectors/` and a version decision (ADR-003).
- **Commit messages** follow [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/):
  `<type>(<scope>): <description>`, for example `fix(server): …`, `feat(web): …`,
  `build(deps): …`, `ci: …` or `docs: …`. Scopes are the parts of the repository — `server`, `web`,
  `cli`, `core` — or `deps`, `ci` and `release`; a breaking change adds `!` and a
  `BREAKING CHANGE:` footer. The body explains *why*, not only what. One logical change per commit.
  Pull request titles use the same format, because a squash merge turns the title into the commit.
