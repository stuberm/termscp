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

## Temporary folder-comparison diagnostics

For investigating quick-comparison results, use a separate **opt-in JSONL session
log** instead of enabling broad protocol TRACE output. From a source checkout on
macOS/Linux, after building:

```sh
SIDEMIRROR_COMPARE_LOG=/tmp/sidemirror-compare-session.jsonl ./target/debug/sidemirror
```

On Windows PowerShell:

```powershell
$env:SIDEMIRROR_COMPARE_LOG = "$env:TEMP\sidemirror-compare-session.jsonl"
.\target\debug\sidemirror.exe
Remove-Item Env:SIDEMIRROR_COMPARE_LOG
```

Choose a **new filename** in an existing directory. Existing files (including
symlinks) are never overwritten or appended to. Unix files are created with
owner-only permissions; Windows uses the parent directory's access controls.
The path is printed before startup and again after exit. An invalid/unwritable
path stops this explicitly requested diagnostic launch rather than silently
running without a log. Later write failures are reported in the normal log and
marked as incomplete on exit; they do not change comparison results.

Reproduce one misleading result, note its relative path, optionally open that
file's diff or press F5 to record full verification, then quit normally. Records
are flushed as they are written. The log includes:

- Session/schema version, safe backend labels, request/pool/scan IDs, modes,
  active per-profile quick criteria, and whole-comparison versus copied-path scope.
- Relative entry names, kinds, sizes, ignore patterns, exact UTC modification
  times and nanosecond epoch values, the **actual whole-second values compared**,
  signed right-minus-left size/time deltas, outcomes and mismatch reasons.
- Final directory statuses and their pre-aggregation bases, so file differences
  can be traced to folder summaries; displayed snapshots after completion,
  navigation, filtering, ignores or copy updates.
- Diff/copy request outcomes. A decoded text diff with no changes is not itself
  proof of equal raw encodings; full comparison records normalized-byte results.

Schema version 2 adds `quick_check` (`none`, `size`, `timestamp`, `both`) and explicit
`size_check_enabled`/`timestamp_check_enabled` flags to row records. Mismatch
reasons consider only enabled checks; raw `size_equal` and `mtime_seconds_equal`
fields still show the underlying metadata, including disabled checks. With `none`,
both flags are false and the actual scan mode is `full`: content verification is
required, not a metadata match. The session
policy describes per-profile checks rather than assuming size and time are both
required. Version-1 logs retain their original both-checks meaning.

Epoch values/deltas are decimal strings to avoid JSON-number precision loss.
Each line is one JSON object with `event`, `sequence`, `at_utc` and `data` fields.
A `row` event's `data.result` holds the metadata and actual scan decision;
`display_row` describes retained UI state, not a new comparison decision.

**Privacy:** this file contains filenames and metadata, which may be sensitive.
Review it before sharing. It deliberately omits contents, hashes, credentials,
connection URLs/root settings, profile dumps and raw remote error messages. No
password-entry keystrokes are logged. No `-D` flag is needed, and diagnostics do
not cause extra file-content reads. Diagnostic serialization and local log writes
add overhead, so do not use these session timings as performance benchmarks.

Without `SIDEMIRROR_COMPARE_LOG` there is no diagnostic file or per-row
serialization. A command-prefix environment assignment lasts for that launch
only; if exported, unset it afterwards. The variable is independent of `-q`,
which disables only the normal application log. Remove the diagnostic file when
it is no longer needed; no profile/configuration changes are made.

## Disabling logging

To turn logging off, start sidemirror with the `-q` or `--quiet` option. You can
alias sidemirror to make it persistent.

## Security

The log file does not contain any plaintext password. It exposes the same
information as the sibling `bookmarks` file.
