# 安装

Sidemirror 目前需要从源码构建。已发布的 termscp 软件包和上游安装脚本不会安装 sidemirror。

使用 rustup 安装 Rust，然后在仓库根目录运行：

```sh
cargo run --no-default-features --features keyring
```

所需的 Rust 版本由 `rust-toolchain.toml` 指定。此构建移除 SMB，但保留强制原生凭据存储以及 FTP、FTPS 和 SFTP。在 Linux 上，凭据需要会话 D-Bus 上正常运行且已解锁的 Secret Service。不提供文件密钥回退或仅会话密码模式。

构建优化版本：

```sh
cargo build --release --no-default-features --features keyring
./target/release/sidemirror
```

在 Windows 上运行 `target\release\sidemirror.exe`。

若需要默认功能，请在 macOS 上安装 `pkg-config` 和 Samba，或在 Linux 上安装 `libdbus-1-dev` 和 `libsmbclient-dev`，然后运行 `cargo run`。

Sidemirror 使用独立于 termscp 的配置和缓存目录，不会自动导入现有 termscp 数据。

除非编译时设置了 `SIDEMIRROR_UPDATE_REPOSITORY=owner/repo` 并使用对应的发布仓库，否则自动更新被禁用。普通源码构建请更新仓库后重新编译。
