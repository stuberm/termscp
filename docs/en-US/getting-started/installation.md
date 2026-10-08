# Installation

Sidemirror is currently built from source. Published termscp packages and upstream installer scripts do not install sidemirror.

Install Rust using rustup, then run from the repository root:

```sh
cargo run --no-default-features
```

The required Rust version is pinned in `rust-toolchain.toml`. Disabling default features removes SMB and system-keyring support, but retains FTP, FTPS, and SFTP.

For an optimized build:

```sh
cargo build --release --no-default-features
./target/release/sidemirror
```

On Windows, run `target\release\sidemirror.exe`.

For default features, install `pkg-config` and Samba on macOS, or `libdbus-1-dev` and `libsmbclient-dev` on Linux, then run `cargo run`.

Sidemirror uses separate configuration and cache directories from termscp. Existing termscp data is not automatically imported.

Automatic updates are disabled unless `SIDEMIRROR_UPDATE_REPOSITORY=owner/repo` is set when compiling a build intended for that release repository. For ordinary source builds, update your checkout and rebuild.
