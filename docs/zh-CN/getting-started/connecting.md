# 连接到服务器

不带参数启动 sidemirror 会显示已保存的比较和 **New comparison（新建比较）**。选择新建比较或按 `n`，输入名称并选择源和目标；两端均可选择 Local、SFTP/SCP、FTP/FTPS、S3、GCS、Kubernetes、WebDAV 或启用功能时的 SMB，并提供对应的连接、认证和文件夹设置。F2/F3 选择面板；Enter 保存并打开，已有比较可用 Enter 重新打开或用 `e` 更新。请参阅[文件夹比较](../usage/folder-comparison.md)。旧配置会自动加载并在下次保存时迁移。

原有文件传输工作流程仍可通过以下方式使用：

- `sidemirror transfer`：打开认证表单，你在其中提供连接到远程主机所需的参数。
- 带地址参数：sidemirror 跳过认证表单，直接连接到远程主机。
- 通过 `-b <bookmark-name>` 传入书签名称：sidemirror 将参数解析为已保存的书签并进行连接。重复使用 `-b` 可打开多个书签。

当你提供地址参数或书签名称时，还可以为本地主机提供一个起始工作目录。

## 认证表单

当 `sidemirror transfer` 在不带地址的情况下启动时，会显示认证表单。填写协议、地址、端口、用户名和密码，然后进行连接。连接成功后，sidemirror 将打开双面板浏览器。

## SSH 配置优先级

对于 SFTP 和 SCP 连接，sidemirror 会按以下顺序解析 CLI 的 Username 和 Port 值：显式指定的 `Username`/`Port`、SSH 配置中的 `User`/`Port`，然后是当前 OS 用户和 Port `22`。Username 和 Port 会独立解析，因此其中一个值被显式指定不会阻止 SSH 配置提供另一个值。

例如，`sidemirror myhost` 会使用为 `myhost` 配置的 Port 和 User。无论 SSH 配置为何，`sidemirror alice@myhost:22` 都会使用 `alice` 和 `22`。

## 地址参数语法

通用地址参数采用以下语法：

```txt
[protocol://][username@]<address>[:port][:wrkdir]
```

这种语法很方便，你很可能会用它来代替交互式表单。下面是一些示例。

使用默认协议（在你的配置中定义）连接到 `192.168.1.31`。对于 SFTP 和 SCP，未提供的 Port 或 Username 会从匹配的 SSH 配置条目中获取。如果没有配置相应的值，则会回退到协议默认 Port 或当前 OS 用户。其他协议会使用其默认 Port 和当前 OS 用户。

```sh
sidemirror 192.168.1.31
```

使用默认协议连接到 `192.168.1.31`，用户名为 `root`：

```sh
sidemirror root@192.168.1.31
```

使用 SCP 连接到 `192.168.1.31`，端口为 `4022`，用户名为 `omar`：

```sh
sidemirror scp://omar@192.168.1.31:4022
```

使用 SCP 连接到 `192.168.1.31`，端口为 `4022`，用户名为 `omar`，并以目录 `/tmp` 作为起始目录：

```sh
sidemirror scp://omar@192.168.1.31:4022:/tmp
```

有关各协议专属的地址语法（S3、GCS、Kube、WebDAV 和 SMB），请参阅[连接参数](connection-parameters.md)。

## 密码的提供方式

当你以参数形式提供地址时，地址本身没有用于填写密码的字段。你可以通过三种方式提供密码：

- 系统会提示你输入密码。这是默认方式：如果你不使用下面的任何方法，sidemirror 会像 `scp`、`ssh` 等经典工具一样提示你输入密码。
- `-P, --password` 选项：直接在命令行上传入密码。不推荐这种方法，因为它不安全：密码可能会保留在你的 shell 历史记录中。
- 通过 `sshpass`：借助 `sshpass` 提供密码，例如：

  ```sh
  sshpass -f ~/.ssh/topsecret.key sidemirror cvisintin@192.168.1.31
  ```
