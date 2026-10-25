# Contributing

Every change goes through an issue and a pull request, also the maintainer's:

1. Open an issue: what is wrong or missing, how to reproduce it, which camera and firmware.
2. Open a pull request against `main` whose description starts with `Fixes #<issue>`.
3. CI must be green (`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`).
   Changes that only touch docs (Markdown files, `LICENSE`, `docs/`) skip those three; the
   `required` check still reports. No changelog entry is enforced per pull request; the release
   tag needs its `CHANGELOG.md` section.
4. Commits are signed; `main` only accepts signed commits and a linear history.
5. The maintainer merges fast-forward only, by pushing the green PR head to `main`, so your
   commits keep their hashes and signatures. The merge button is not used.

Protocol details and captures live in
[obsbot-tiny2-lite-protocol](https://github.com/nofuturekid/obsbot-tiny2-lite-protocol).
Changes to camera commands need evidence (a capture or a measured read-back), not a guess.

Contributions written with AI assistance are welcome; say so in the pull request.
