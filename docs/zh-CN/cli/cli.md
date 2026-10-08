# 命令行用法

sidemirror 可以通过以下调用形式启动：

```sh
sidemirror [options]... [protocol://user@address:port:wrkdir] [protocol://user@address:port:wrkdir] [local-wrkdir]
```

或

```sh
sidemirror [options]... -b [bookmark-name] -b [bookmark-name] [local-wrkdir]
```

以及两者的任意组合。

如果未提供额外参数，sidemirror 会显示身份验证表单。如果提供了地址参数或书签名称，sidemirror 会跳过该表单并直接连接到远程服务器。当提供地址或书签时，你还可以将本地主机的起始工作目录作为最后一个位置参数提供。

## 选项

| Key                  | 说明                                               |
| -------------------- | ------------------------------------------------ |
| `-b <bookmark-name>` | 将位置地址参数解析为书签名称。重复使用该标志可以打开多个书签。                  |
| `-D`                 | 启用 `TRACE` 日志级别（调试/详细日志）。                        |
| `-P <password>`      | 从命令行提供密码。重复使用该标志可为多个远程主机提供密码；其顺序必须与地址参数一致。不推荐使用。 |
| `-q`                 | 禁用日志记录。                                          |
| `-T <ticks>`         | 设置 UI 的 tick 间隔（以毫秒为单位）。默认值为 `10`。               |
| `--wno-keyring`      | 禁用系统 keyring 支持。                                 |
| `-v`                 | 打印版本信息。                                          |
| `--help`             | 打印帮助页面。                                          |

不推荐使用 `-P` 选项，因为密码可能会保留在 shell 历史记录中。请参阅书签和密码安全章节，了解更安全的凭据提供方式。

## 子命令

sidemirror 提供以下子命令。

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
