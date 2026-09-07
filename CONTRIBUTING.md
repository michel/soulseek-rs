# Contributing

Issues and pull requests are both welcome, and so is just saying what's missing.
No contributor agreement, no template.

## Pull requests

Branch off `develop` and target `develop`. `master` only moves through a release
PR.

Write the subject as a
[conventional commit](https://www.conventionalcommits.org), like
`fix(lib): route on the full four-byte message code`. release-plz reads these to
pick the next version and write the changelog. One change per pull request;
small ones land fastest.

Before you push:

```bash
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test
```

CI runs those on Linux, macOS and Windows, plus an end-to-end suite against a
real [soulfind](https://github.com/soulfind-dev/soulfind) server. The e2e suites
skip when no server is around, so `cargo test` stays green without one.
[Development](./README.md#development) has the setup if you want to run them.

AI-assisted changes are fine, and no need to say so. Keep the trailers out of
the commit message: no `Co-Authored-By:` for a model, no session links.

## Issues

The command you ran, what you expected, and what happened is plenty.
`RUST_LOG=trace` output helps. Don't worry about polishing it.

Built something on `soulseek-rs-lib`? Open a PR adding it to the README, and
reserve a client minor version in
[#12](https://github.com/michel/soulseek-rs/issues/12) so it's identifiable on
the network.

Contributions are under MIT, like the rest of the project.
