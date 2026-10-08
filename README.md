# sidemirror

A terminal folder and file comparison app based on [termscp](https://github.com/veeso/termscp).

The current implementation retains termscp's dual-pane file browser, remote connections, bookmarks, and text-diff support. A dedicated folder comparison workflow is planned.

## Build and run

Install Rust through rustup. The required toolchain is pinned in `rust-toolchain.toml`.

```sh
cargo run --no-default-features
```

This disables SMB and system-keyring support; FTP, FTPS, and SFTP remain available.

For an optimized build:

```sh
cargo build --release --no-default-features
./target/release/sidemirror
```

On Windows, run `target\release\sidemirror.exe`.

Connect directly:

```sh
cargo run --no-default-features -- 'sftp://user@example.com:22/path/to/folder'
```

For default features, macOS requires `pkg-config` and Samba; Linux requires `libdbus-1-dev` and `libsmbclient-dev`.

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
cargo check --no-default-features
cargo test --no-default-features --features github-actions
```

## Attribution and license

Based on termscp by Christian Visintin and its contributors. Original attribution and the MIT license are preserved; see [LICENSE](LICENSE).
