# Contributing to Thrall

Thanks for trying Thrall. It's a preview, so the most useful thing you can do is use it and tell
us what broke, what confused you and what you wished it did.

## Reporting a problem

Open an issue with the **Bug report** form. Say which system you're on (and the desktop, for
Linux), what you did, what you expected and what happened. Logs are in the `logs` folder of
Thrall's data folder (see the README); paste the relevant lines, **with any keys removed**.

Found a security problem (a way past the permission prompts, or a key leaking somewhere)? Please
don't open a public issue: use GitHub's **Report a vulnerability** on the Security tab instead.

## Mac users: help wanted

The macOS builds are made automatically and have never been tried by hand. If you have a Mac,
installing it and telling us what happens (the label `macos`) is a big help, and fixes are very
welcome.

## Code

1. Read [BUILDING.md](BUILDING.md) and get the checks passing on your machine first.
2. Keep a change to one thing, with tests for what it changes. Small pull requests get reviewed
   faster.
3. Follow what's there:
   - Rust: `thiserror` in libraries, no `unwrap()` outside tests, named constants in each crate's
     `constants.rs`, one file per tool or command, registered in one list.
   - Everything a person reads is plain words, says what happened and what to do next.
   - Anything risky asks first, the same way on every client.
   - Clients (web, terminal, desktop, phone) only talk to the engine through the protocol
     (`docs/protocol.md`); no agent logic in a client.
4. Changing the protocol? Update `docs/protocol.md`, refresh the schema
   (`THRALL_UPDATE_SCHEMA=1 cargo test -p thrall-protocol`), regenerate the web types
   (`npm run gen:types` in `web/`) and update the phone's `Protocol.kt`.
5. New dependencies need a reason and a licence compatible with shipping under MIT (`cargo deny check`).

## How changes get in

- **Branches:** `main` is always releasable and is only changed through pull requests. Work on a
  branch named for what it does: `feat/…`, `fix/…`, `security/…`, `docs/…`, `ci/…`, `chore/…`.
- **One kind of change per pull request.** Group related work (a feature and its tests, a set of
  security fixes, a docs pass) and keep unrelated things apart. A pull request is merged as one
  squashed commit, titled the way the commit should read.
- **Commit and pull request titles** follow [Conventional Commits](https://www.conventionalcommits.org/):
  `type(scope): what changed`, for example `feat(memory): keep an experience log` or
  `fix(server): stop serving the access token in the page`. Types: `feat`, `fix`, `security`,
  `docs`, `ci`, `refactor`, `test`, `chore`.
- **Checks must pass** before merging: formatting, lint and tests on Windows, Linux and macOS, the
  web and phone apps, licences and advisories, and a scan for anything private (personal email
  addresses, home folders, private network addresses). Commit with GitHub's no-reply address.
- **Changelog:** add a line under `## [Unreleased]` in [CHANGELOG.md](CHANGELOG.md) for anything a
  user would notice (Added, Changed, Fixed, Security).
- **Versions** follow [Semantic Versioning](https://semver.org/). While Thrall is 0.x, a release
  with new features bumps the middle number and a fixes-only release bumps the last. A release is
  a tag `vX.Y.Z` on `main`; CI builds the installers and drafts the release from the changelog.

## Licence of contributions

Unless you explicitly say otherwise, any contribution you submit for inclusion in Thrall is
licensed under MIT as in the README, without any additional terms or conditions.
