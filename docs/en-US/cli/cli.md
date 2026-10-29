# Command-line usage

Run `sidemirror` without arguments for the home screen: saved comparisons plus **New comparison**. A new comparison independently configures source and destination with the inherited protocol-specific settings (Local, SFTP/SCP, FTP/FTPS, S3, GCS, Kubernetes, WebDAV, and feature-enabled SMB), then saves and opens. Existing profiles can be reopened and updated in place. `sidemirror compare` opens the same screen; `sidemirror compare SOURCE DESTINATION` starts directly. Older left/right profile data loads automatically and is migrated to typed source/destination data on the next save.

The inherited transfer workflow also supports the following legacy invocation forms:

```sh
sidemirror [options]... [protocol://user@address:port:wrkdir] [protocol://user@address:port:wrkdir] [local-wrkdir]
```

OR

```sh
sidemirror [options]... -b [bookmark-name] -b [bookmark-name] [local-wrkdir]
```

AND any combination of the two.

Use `sidemirror transfer` to open the legacy authentication/bookmark form. If an
address argument or a bookmark name is provided, the transfer workflow skips the form and
connects directly to the remote server. When an address or bookmark is given,
you may also provide the starting working directory for the local host as the
last positional argument.

## Options

| Key                  | Description                                                                                                                       |
| -------------------- | --------------------------------------------------------------------------------------------------------------------------------- |
| `-b <bookmark-name>` | Resolve the positional address argument as a bookmark name. Repeat the flag to open multiple bookmarks.                           |
| `-D`                 | Enable the `TRACE` log level (debug/verbose logging).                                                                             |
| `-P <password>`      | Provide the password from the CLI. Repeat the flag for multiple remotes; the order must match the address arguments. Discouraged. |
| `-q`                 | Disable logging.                                                                                                                  |
| `-T <ticks>`         | Set the UI tick interval in milliseconds. Default is `10`.                                                                        |
| `--wno-keyring`      | Obsolete; rejected. Native credential storage is mandatory.                                                                                                   |
| `-v`                 | Print version info.                                                                                                               |
| `--help`             | Print the help page.                                                                                                              |

The `-P` option is discouraged because the password may be kept in the shell
history. See the bookmarks and password-security chapters for safer ways to
provide credentials.

## Folder comparison

Run `sidemirror compare SOURCE DESTINATION` for read-only recursive folder comparison, or `sidemirror compare` for saved comparisons. Both endpoints independently accept a local folder, a standard `sftp://`, `scp://`, `ftp://`, `ftps://`, or HTTP(S) WebDAV URL, or `bookmark:NAME` for any supported connection protocol. Use the settings panels or saved profiles for full protocol-specific options. Use `--save NAME` to save a new profile, `--profile NAME` to reopen it, `--ignore GLOB` for exclusions, `--differences` for filtering, and `--password-prompt` for masked authentication. See [folder comparison](../usage/folder-comparison.md) for keyboard controls, limits, and security considerations.

## Subcommands

sidemirror exposes the following subcommands.

### Legacy file transfer

```sh
sidemirror transfer
sidemirror transfer 'sftp://user@server:22:/folder'
sidemirror transfer -b connection-bookmark
```

Without endpoints, this opens the inherited authentication/bookmark screen. With endpoints, it opens the file-transfer browser. Transfer addresses use the inherited colon-separated directory syntax, not comparison URLs. Global logging, tick, and keyring options go before `transfer`.

### Import a theme

```sh
sidemirror theme <theme-file>
```

Import the theme defined in `<theme-file>`.

### Install the latest version

```sh
sidemirror update
```

Automatic updates are disabled in ordinary source builds. Update your checkout and rebuild instead. This command is available only when a fork release repository is explicitly configured at compile time using `SIDEMIRROR_UPDATE_REPOSITORY=owner/repo`; it never downloads upstream termscp releases by default.

### Import ssh hosts

```sh
sidemirror import-ssh-hosts [ssh-config-file]
```

Import all the hosts from the specified ssh config file as bookmarks in
sidemirror. If `[ssh-config-file]` is not provided, the default location
`~/.ssh/config` is used. Identity files are imported as ssh keys in sidemirror too.

### Open configuration

```sh
sidemirror config
```

Start sidemirror directly in the configuration (setup) screen.
