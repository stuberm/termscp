# 命令行用法

不带参数运行 `sidemirror` 会打开主屏幕，显示已保存的比较和 **New comparison（新建比较）**。新建比较独立配置源和目标，提供原有协议专用设置：Local、SFTP/SCP、FTP/FTPS、S3、GCS、Kubernetes、WebDAV 和启用功能时的 SMB，然后保存并打开。已有配置可重新打开并原位更新。`sidemirror compare` 打开同一个主屏幕，`sidemirror compare SOURCE DESTINATION` 可直接开始比较。旧的 left/right 配置会自动加载，并在下次保存时迁移为有类型的 source/destination 数据。

原有传输工作流程仍支持以下旧式调用形式：

```sh
sidemirror [options]... [protocol://user@address:port:wrkdir] [protocol://user@address:port:wrkdir] [local-wrkdir]
```

或

```sh
sidemirror [options]... -b [bookmark-name] -b [bookmark-name] [local-wrkdir]
```

以及两者的任意组合。

使用 `sidemirror transfer` 打开原有的身份验证与书签界面。如果提供了地址参数或书签名称，传输工作流程会跳过该表单并直接连接到远程服务器。当提供地址或书签时，你还可以将本地主机的起始工作目录作为最后一个位置参数提供。

## 选项

| Key                  | 说明                                               |
| -------------------- | ------------------------------------------------ |
| `-b <bookmark-name>` | 将位置地址参数解析为书签名称。重复使用该标志可以打开多个书签。                  |
| `-D`                 | 启用 `TRACE` 日志级别（调试/详细日志）。                        |
| `-P <password>`      | 从命令行提供密码。重复使用该标志可为多个远程主机提供密码；其顺序必须与地址参数一致。不推荐使用。 |
| `-q`                 | 禁用日志记录。                                          |
| `-T <ticks>`         | 设置 UI 的 tick 间隔（以毫秒为单位）。默认值为 `10`。               |
| `--wno-keyring`      | 已废弃，使用时会报错；原生凭据存储是强制要求。                                 |
| `-v`                 | 打印版本信息。                                          |
| `--help`             | 打印帮助页面。                                          |

不推荐使用 `-P` 选项，因为密码可能会保留在 shell 历史记录中。请参阅书签和密码安全章节，了解更安全的凭据提供方式。

## 子命令

运行 `sidemirror compare SOURCE DESTINATION` 开始只读递归文件夹比较，或运行 `sidemirror compare` 打开已保存的比较。两个端点各自都可以是本地文件夹、标准 `sftp://`、`scp://`、`ftp://`、`ftps://` 或 HTTP(S) WebDAV URL，以及任意受支持协议的 `bookmark:NAME`。完整的协议专用选项通过设置面板或保存的配置提供。`--save NAME` 保存新配置，`--profile NAME` 重新打开配置，`--ignore GLOB` 排除条目，`--differences` 仅显示差异，`--password-prompt` 提供掩码密码输入。键盘操作、限制和安全说明请参阅[文件夹比较](../usage/folder-comparison.md)。

sidemirror 提供以下子命令。

### 原有文件传输

```sh
sidemirror transfer
sidemirror transfer 'sftp://user@server:22:/folder'
sidemirror transfer -b connection-bookmark
```

没有端点参数时打开原有的身份验证与书签界面；提供端点时打开文件传输浏览器。传输地址使用原有的冒号分隔目录语法，与比较 URL 不同。全局日志、tick 和密钥环选项请放在 `transfer` 前面。

### 导入主题

```sh
sidemirror theme <theme-file>
```

导入在 `<theme-file>` 中定义的主题。

### 安装最新版本

```sh
sidemirror update
```

普通源码构建默认禁用自动更新，请更新仓库后重新编译。只有编译时通过 `SIDEMIRROR_UPDATE_REPOSITORY=owner/repo` 明确配置了分支发布仓库，此命令才能使用；默认不会下载上游 termscp 的发布版本。

### 导入 ssh 主机

```sh
sidemirror import-ssh-hosts [ssh-config-file]
```

将指定 ssh 配置文件中的所有主机作为书签导入到 sidemirror 中。如果未提供 `[ssh-config-file]`，则使用默认位置 `~/.ssh/config`。身份文件也会作为 ssh 密钥导入到 sidemirror 中。

### 打开配置

```sh
sidemirror config
```

直接在配置（setup）界面中启动 sidemirror。
