# Contributing to Assay

Thanks for your interest in contributing to Assay! This document explains how
to file issues, submit pull requests, and what we expect from contributions.

## Reporting bugs and requesting features

- **Bug reports**: please open an issue on GitHub with a minimal reproduction
  (the smallest `.lua` script that triggers the problem) plus the assay
  version (`assay --version`), platform, and full error output.
- **Feature requests**: open an issue describing the use case first. We'd
  rather discuss design before you spend time on a PR that may need to be
  redone.

## Pull requests

Before sending a PR:

1. **Open or comment on an issue first** for anything non-trivial. A two-line
   bug fix is fine to send directly. A new stdlib module or builtin is not.
2. **Run the full test suite locally**:

   ```sh
   cargo test
   cargo clippy --all-targets -- -D warnings
   cargo fmt --check
   ```

3. **Add tests for your change**. Bug fixes need a regression test that fails
   before the fix and passes after. New features need coverage of the happy
   path and the obvious failure modes.
4. **Match the existing code style**. We use `cargo fmt` for Rust and follow
   the conventions in `stdlib/` for Lua modules. Look at neighbouring files
   before writing new ones.
5. **Update documentation**. If you change a public API, update `CHANGELOG.md`,
   any relevant `@quickref` metadata in stdlib files, and the matching
   section of `README.md` / `site/modules.html` if applicable.
6. **Keep PRs focused**. One logical change per PR. If you find unrelated
   issues while working, open a separate PR for those.

## Contributor License Agreement (CLA)

Assay requires all contributors to sign a Contributor License Agreement before
their PRs can be merged. The full text of the CLA is in [`CLA.md`](CLA.md).

**Why we have a CLA**: it lets the project owner relicense the project (or
include contributions in proprietary commercial editions) in the future
without needing to track down every contributor for permission. You retain
the copyright on your contribution; you grant the project owner a broad
license to use it.

**How to sign**: when you open your first PR, the CLA Assistant bot will post
a comment with a link. Click the link, agree to the terms with your GitHub
account, and you're done — your signature is recorded for all future PRs to
this project.

If you can't or won't sign the CLA (for example, because your employer
prohibits it), please open an issue describing the change instead and we'll
figure out an alternative path together.

## Code of conduct

Be kind. Disagree on technical merits, not on people. Project maintainers
reserve the right to close issues, remove comments, or block users that
consistently fail to engage in good faith.

## License

By contributing to Assay, you agree that your contributions will be licensed
under the [Apache License, Version 2.0](LICENSE) and that your contribution
is also subject to the terms of the [CLA](CLA.md).
