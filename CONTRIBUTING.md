# Contributing

Thanks for helping! Questions and ideas are welcome on
[Discord](https://discord.gg/sTzwBzDkK9) or in an issue.

## Setup

- Rust (stable), GTK 4.14+ development files (`libgtk-4-dev`, `gtk4-devel`, `gtk4`)
- [Deno](https://deno.com) 2

```sh
cargo run                                    # the launcher, using ./sdk and your installed extensions
BACKSLASH_EXTENSIONS_PATH=$PWD/extensions cargo run   # with the bundled extensions
cargo run -- run npm/search react            # CLI
make check                                   # what CI runs, but the UI tests
make ui-test                                 # the window in Xvfb, driven by xdotool
```

`backslash dev <dir>` runs a separate instance (`app.backslash.Backslash.Devel`)
that never hides on focus loss and prints extension logs.

## Where things are

Read [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) first. In short:

- `crates/backslash-core`: protocol, host, ranking, manifests. Most logic, most tests.
- `crates/backslash`: the GTK window and the CLI.
- `sdk/`: the TypeScript API and its runtime.
- `extensions/`: bundled extensions.

Protocol changes must update `protocol.rs`, `sdk/`, and `docs/PROTOCOL.md` together.

## Pull requests

- Keep them focused; describe the user-visible change.
- `make check` must pass (format, clippy, tests, Deno lint and type check).
- New behavior in `backslash-core` comes with a test. Keyboard and window
  behavior can be tested in `crates/backslash/tests/ui.rs`: the launcher
  writes what it shows to `BACKSLASH_UI_TRACE` as JSON.

## Releasing

One click: **Actions → Release → Run workflow**, type the version (`0.4.0`),
**Run**. The workflow:

1. commits the version (Cargo, AppStream, PKGBUILD) and tags it `v0.4.0`;
2. builds the `.deb`, `.rpm` and tarball for x86_64 and aarch64;
3. publishes the GitHub release with notes generated from the merged pull requests;
4. publishes the `backslash` package on the AUR;
5. opens a pull request bringing the new version to `main` (merge it).

Pushing a `v0.4.0` tag, or creating the release on GitHub, does the same.
`packaging/set-version.sh 0.4.0` changes the version locally.

### AUR setup (once)

1. Create an account on [aur.archlinux.org](https://aur.archlinux.org/register)
   with the project address, `contact@getbackslash.com` (the `PKGBUILD`
   names it as maintainer and the AUR commits are made under it).
2. Make a key for CI: `ssh-keygen -t ed25519 -f aur -N "" -C backslash-ci`.
3. Paste `aur.pub` in *My Account → SSH Public Key* on the AUR.
4. Paste the content of `aur` (the private key) in the repository's
   *Settings → Secrets and variables → Actions → New repository secret*,
   named `AUR_SSH_PRIVATE_KEY`. Then delete both files.
5. Allow *Settings → Actions → General → Allow GitHub Actions to create and
   approve pull requests*, for the version pull request.

The first release after this creates the `backslash` package on the AUR.
Without the secret, the AUR step only checks the package and says so.

### Registry signing key (once)

Backslash reads the extension registry only from commits signed by the
registry key, which the CI of
[backslash-app/plugins](https://github.com/backslash-app/plugins) makes on its
`signed` branch each time `main` passes the checks.

1. Make the key: `ssh-keygen -t ed25519 -f registry -N "" -C backslash-registry`.
2. In backslash-app/plugins, add the content of `registry` (the private key) as
   the Actions secret `REGISTRY_SIGNING_KEY`, then re-run the last *Check*
   workflow on `main`: the `signed` branch appears.
3. Here, add the line of `registry.pub` to `DEFAULT_SIGNERS` in
   `crates/backslash-core/src/catalog.rs` and release. Keep the private key
   offline (a password manager), then delete both files.

To replace the key, add the new public key next to the old one, release, then
change the secret, and remove the old key in a later release. Until
`DEFAULT_SIGNERS` holds a key, the registry is read from `main` unchecked.
