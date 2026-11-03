# Contributing

Thank you for helping with Mailer.

## Reporting problems

Open an issue and describe what you did, what you expected and what happened.
Include your operating system and the Mailer version (shown at the bottom of the
settings window). Do not post passwords or private email content.

## Making changes

1. Fork the repository and create a branch.
2. Make your change and add a line to the `Unreleased` section of
   [CHANGELOG.md](CHANGELOG.md).
3. Check that these commands pass:

   ```sh
   cargo fmt --check
   cargo clippy --all-targets -- -D warnings
   cargo test
   ```

4. Open a pull request.

## Making a release

1. Update `version` in `Cargo.toml` and run `cargo build` to update `Cargo.lock`.
2. Move the `Unreleased` entries in `CHANGELOG.md` to a new version section.
3. Commit, then tag and push:

   ```sh
   git tag v1.2.3
   git push origin main v1.2.3
   ```

The release workflow builds Mailer for Linux, macOS and Windows and creates the
GitHub release. Versions with a suffix such as `-alpha` are marked as
pre-releases.
