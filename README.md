# sidemirror

A terminal folder and file comparison app based on [termscp](https://github.com/veeso/termscp).

Saved folder comparisons pair a source and destination, each independently Local, SFTP, SCP, FTP/FTPS, S3, GCS, Kubernetes, WebDAV, or feature-enabled SMB. Read-only recursive scans provide aligned rows, difference filters, keyboard navigation, and UTF-8 file diffs. The original termscp transfer browser remains available.

## Build and run

Install Rust through rustup. The required toolchain is pinned in `rust-toolchain.toml`.

```sh
cargo run --no-default-features --features keyring
```

The home screen lists saved comparisons plus **New comparison**. Select it (or press `n`), name the comparison, and configure the independent source and destination panels. Each offers protocol-specific settings; F2/F3 selects a panel, Space/Left/Right chooses its type, and Tab moves through fields. Enter saves and opens it. Select a saved comparison and press `e` to update its settings; saving updates that same profile and reopens it. This build disables SMB but keeps mandatory native credential storage. Passwords always use the OS credential store; there is no session-only password mode or plaintext-key fallback.

For an optimized build:

```sh
cargo build --release --no-default-features --features keyring
./target/release/sidemirror
```

On Windows, run `target\release\sidemirror.exe`.

The inherited transfer/connection screen is opt-in:

```sh
cargo run --no-default-features --features keyring -- transfer
cargo run --no-default-features --features keyring -- transfer 'sftp://user@example.com:22:/path/to/folder'
```

For default features, macOS requires `pkg-config` and Samba; Linux requires `libdbus-1-dev` and `libsmbclient-dev`.

## Compare folders

```sh
cargo run --no-default-features --features keyring -- compare ./left ./right
cargo run --no-default-features --features keyring -- compare ./local 'sftp://user@server:22/path' --save website
cargo run --no-default-features --features keyring -- compare --profile website
cargo run --no-default-features --features keyring -- compare
```

The last command is an alias for starting sidemirror without arguments: both open the saved-comparisons home screen. Either endpoint may also be a remote URL or `bookmark:NAME`, including remote-to-remote comparisons. Comparison profiles are separate from connection bookmarks; older left/right profiles are loaded automatically and migrated to typed source/destination data on the next save. Use Enter to open a directory/file, `d` to filter differences, `f` for recursive rows, `n`/`p` to jump differences, `r` to refresh, `c` to cancel, `o` for profiles, and `q` to quit. Comparison never writes to either endpoint. Passwords and S3 secret access keys entered in masked endpoint fields are saved in the native OS credential store after successful authentication; temporary S3 tokens remain ephemeral. Credential profiles/connection bookmarks are also supported. Secrets are never written into comparison profile files. Existing version-1/2 profiles load automatically and are written as version 3 on the next save.

See [comparison documentation](docs/en-US/usage/folder-comparison.md) for details. The inherited SFTP client currently **does not verify server host keys**; use trusted endpoints only.

## Application data

Sidemirror uses its own config and cache directories:

- Linux: `$XDG_CONFIG_HOME/sidemirror` (normally `~/.config/sidemirror`)
- macOS: `~/.config/sidemirror`
- Windows: `%USERPROFILE%\.sidemirror`

Existing termscp configuration is not automatically imported. Automatic updates are disabled unless a sidemirror release repository is configured at compile time with `SIDEMIRROR_UPDATE_REPOSITORY=owner/repo`. Rebuild from source to update.

The inherited website, published packages, installer scripts, and release-distribution infrastructure still refer to upstream termscp; they are not sidemirror installers.

## Documentation and development

Documentation is under `docs/en-US/` and `docs/zh-CN/`. Some inherited pages describe upstream functionality and resources.

```sh
cargo check --no-default-features --features keyring
cargo test --no-default-features --features keyring,github-actions
```

## Attribution and license

Based on termscp by Christian Visintin and its contributors. Original attribution and the MIT license are preserved; see [LICENSE](LICENSE).
