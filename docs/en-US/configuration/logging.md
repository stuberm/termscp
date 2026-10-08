# Logging

sidemirror writes a log file for each session, located at:

- `$HOME/.cache/sidemirror/sidemirror.log` on Linux/BSD
- `$HOME/Library/Caches/sidemirror/sidemirror.log` on macOS
- `FOLDERID_LocalAppData\sidemirror\sidemirror.log` on Windows

The log is not rotated: it is truncated on each launch of sidemirror. If you want
to report an issue and attach the log file, save the log somewhere safe before
launching sidemirror again.

By default the log reports at the `INFO` level, so it is not very verbose.

## Reproducing an issue at TRACE level

To submit an issue, reproduce the problem with the log level set to `TRACE` by
launching sidemirror with the `-D` CLI option.

## Disabling logging

To turn logging off, start sidemirror with the `-q` or `--quiet` option. You can
alias sidemirror to make it persistent.

## Security

The log file does not contain any plaintext password. It exposes the same
information as the sibling `bookmarks` file.
