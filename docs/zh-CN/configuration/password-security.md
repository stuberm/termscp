# 密码安全

## 比较配置

原生凭据存储是强制要求，始终启用。成功使用的密码和 S3 秘密访问密钥直接存入操作系统原生凭据存储，并在端点设置相同时复用。秘密值不会进入 `comparisons.json`。不提供仅会话密码模式、关闭选项或文件存储回退；存储不可用或被锁定时会明确报错。`--wno-keyring` 会被拒绝。原生存储始终编译，即使使用 `--no-default-features` 也不会禁用；旧 `keyring` 特性仅保留为兼容别名。旧配置也不能关闭此策略。

S3 临时会话令牌和安全令牌不是密码，仍仅临时使用。请使用操作系统凭据管理器删除已保存的条目。SSH agent 和凭据配置仍受支持。详情及当前传输验证限制见[文件夹比较](../usage/folder-comparison.md)。

## 连接书签

连接书签同样强制使用原生凭据存储。

书签连同其密码一起保存在你的配置目录中。密码不会以明文存储：它们使用 AES 加密。

用于加密密码的密钥仅存储在操作系统的密钥存储中：

- Windows 上的 Windows Vault
- Linux 上的系统密钥环
- macOS 上的 Keychain

这由你的操作系统直接管理。

没有可用原生存储的系统会明确报错，绝不将密钥保存为明文文件。WSL 需要正常运行的 Linux Secret Service。已有文件密钥书签不会被自动迁移或删除；请保留旧文件，直到恢复凭据并重新安全保存。

## Linux 密钥环

在 Linux 上，你的系统中可能没有安装密钥环。密钥存储需要一个在 D-Bus 上暴露 `org.freedesktop.secrets` 的服务，而只有少数服务提供它：

- 如果你使用 GNOME 作为桌面环境（例如 Ubuntu 用户），密钥环已经由 `gnome-keyring` 提供，一切应该开箱即用。
- 对于其他桌面环境，你可以使用 [KeepassXC](https://keepassxc.org/) 来获取一个密钥环。它必须经过设置才能与 sidemirror 配合使用；参见下面的 [KeepassXC 设置](#keepassxc-设置)。
- 必须有正常运行且已解锁的 Secret Service 和会话 D-Bus，否则凭据操作失败，不提供回退。

### KeepassXC 设置

按照以下步骤为 sidemirror 设置 KeepassXC：

1. 安装 KeepassXC。
2. 在工具栏中进入 "Tools" > "Settings"。
3. 选择 "Secret service integration" 并启用 "Enable KeepassXC freedesktop.org secret service integration"。
4. 如果你还没有数据库，请创建一个：在工具栏中，"Database" > "New database"。
5. 在工具栏中，进入 "Database" > "Database settings"。
6. 选择 "Secret service integration" 并启用 "Expose entries under this group"。
7. 选择将保存 sidemirror 密钥的组。请注意，任何其他应用程序都可以读取通过 D-Bus 为该组暴露的密钥。
