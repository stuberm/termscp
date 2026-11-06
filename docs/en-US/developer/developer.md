# Developer manual

Welcome to the developer manual for sidemirror. This chapter does NOT contain the
documentation for sidemirror modules, which can instead be found on Rust Docs at
<https://docs.rs/termscp>. This chapter describes how sidemirror works and the
guidelines to implement features such as file transfers and additions to the
user interface.

sidemirror is written in Rust (edition 2024, MSRV 1.98.0). The user interface is
built with [tuirealm](https://github.com/veeso/tui-realm) v3, which runs on top
of [crossterm](https://github.com/crossterm-rs/crossterm).

## How sidemirror works

sidemirror is basically made up of 3 core modules:

- The **host**: the host module provides functions to interact with file
  systems. It exposes the `HostBridge` trait, which abstracts file operations
  over both the local host (`Localhost`) and the remote host (`RemoteBridged`).
- The **ui**: this module contains the implementation of the user interface. As
  shown in the next chapter, this is achieved through **activities**.
- The **activity_manager**: the activity manager takes care of managing
  activities. It runs the activities of the user interface and chooses, based on
  their state, when to terminate the current activity and which activity to run
  next.

In addition to the 3 core modules, others have been added over time:

- **config**: provides the configuration schema and its serialization methods.
- **explorer**: exposes the explorer structures, which are used to handle the
  file explorer in the ui. They store the current directory model and the view
  states (e.g. sorting, whether to display hidden files, the transfer queue).
- **filetransfer**: defines the `FileTransferProtocol` enum and the
  `RemoteFsBuilder`, which constructs the appropriate `RemoteFs` client from the
  connection parameters.
- **system**: provides a way to interact with the configuration, the ssh key
  storage and the bookmarks.
- **utils**: contains the utilities used by pretty much all of the project.

sidemirror supports the following protocols: SFTP, SCP, FTP/FTPS, Kube, S3, GCS,
SMB and WebDAV.

## Comparison CPU benchmarks

Two ignored tests provide repeatable CPU-focused workloads without remote servers
or persistent user data:

```sh
cargo test --release --no-default-features --features github-actions normalization_cpu_benchmark -- --ignored --nocapture
cargo test --release --no-default-features --features github-actions browser_render_cpu_benchmark -- --ignored --nocapture
```

The normalization test reports raw MiB/s for in-memory LF text, CRLF text, binary
bytes and lone CR bytes, checking normalized lengths and raw-byte accounting. The
rendering test draws 100 fixed-size frames with inventories of 100 and 100,000
entries using a terminal test backend. Setup is excluded from the measured loops.
Compare the same build profile and machine; these are microbenchmarks, not
predictions of complete remote-comparison timings. They have no timing thresholds
and are excluded from normal test runs.

The comparison UI keeps derived equal-entry counts outside rendering and only
rebuilds visible paths when membership/order can change. Unchanged idle screens
do not redraw; background/timer redraws are coalesced at 100 ms, while input and
resize redraws bypass that throttle. Normalization uses bounded buffered input
and bulk copies/searches. Only CRLF is normalized: lone CR, other whitespace,
binary bytes and final-newline differences remain significant. Hash/cache
semantics and verification safeguards must remain unchanged by CPU optimizations.

## Quick and full folder comparisons

`ComparisonMode` defaults to `Quick`. Opening a profile and ordinary refreshes
use only the selected size/mtime checks on inventory `Entry` values. Profiles store
`QuickCheck::{None, Size, Timestamp, Both}` as `quick_check` (default/omitted: Both;
size-only is opt-in). The editor maps these four states to two checkboxes.
Both unchecked (`"none"`) disables quick checks and selects Full, not an empty
metadata predicate. Existing profile versions default to Both.
Modification times are floored to UTC seconds to accommodate remote precision.
Disabled metadata is ignored even if missing or different; missing enabled
metadata is unverified unless another enabled check establishes a mismatch.
`Status::QuickMatch` is intentionally distinct from content-verified `Equal`.
Both count as matches for filtering/navigation, but quick results never seed
content hashes. Directory aggregation preserves that distinction and uncertainty.

The folder view's F5 queues a fresh `Full` scan, retaining the bounded verification
pool, CRLF normalization, strong-version cache validation and stream finalization.
Each scan request carries its own mode, quick-check criteria, generation and
cancellation token. The worker applies profile criteria on initial connection and
each refresh; the same pool setting controls scoped post-copy quick checks.
UI, worker and pool resolve requested Quick scans to Full when criteria are None,
including initial startup, refreshes and scoped copy checks. The metadata classifier
also refuses to produce a vacuous match if called directly with None.
Full scans and diff-copy verification do not use quick criteria. The
shared pool's initial connection construction survives cancellation of the first
scan so a full/quick replacement queued during startup can still execute.
Ordinary r refreshes restore quick mode. Copies never queue whole-tree refreshes.
`Pool::scan_paths` reuses classification/verification for selected paths only:
files inspect their own metadata and parents, while selected directories recurse
only within those subtrees. Parent metadata is refreshed without traversing
unrelated siblings. Browser copies recheck in the existing mode; diff copies use
full byte-normalized verification for that file rather than decoded-text equality.
Diff reads and pre-copy checks continue to read live contents regardless of mode.

Workers send generation-tagged `Updated` events before the final copy/text result.
`Browser::apply_copy_updates` merges those observations, preserves unrelated rows
and parent inventory uncertainty, invalidates possibly partial/cancelled writes,
and reaggregates directory summaries without restarting the comparison timer.
Failed rechecks never retain affected old equality claims. `Mode::Diff` owns its
file path so a result hidden by Differences only cannot redirect the next copy to
the browser's newly selected sibling. Esc/q always return without rescanning.
Regression tests in `src/comparison/scoped_tests.rs` assert bounded listing/read
scope; UI copy/diff tests cover mode/timer/selection preservation, created parents,
partial-result uncertainty, failed copies and unchanged unrelated snapshots.

`src/comparison/quick_tests.rs` asserts no quick-mode opens, fingerprints or extra
connections and checks metadata precision, missing metadata, errors, ignores,
aggregation and full verification after metadata matches. UI regression tests in
`src/ui/comparison/quick_tests.rs` exercise F5/r, mode/timer rendering, replacement
of active jobs, fresh inventories and difference filtering/navigation.

## Temporary comparison session diagnostics

`comparison::diagnostics` is opt-in via `SIDEMIRROR_COMPARE_LOG`. It writes a new,
private JSONL file, independently of protocol TRACE logging. Session/scan/request
IDs correlate classification, full verification, aggregation and displayed rows.
File records include exact nanosecond and actual compared-second timestamps,
signed deltas and whitelisted decision reasons. Schema version 2 records active
quick criteria per scan/row and explicit enabled flags; reasons use only enabled
checks, while raw equality/delta fields remain available for diagnosis. Displayed snapshots are explicitly
not new comparison decisions. Backend labels exclude URLs/root settings; raw
errors, profile/config dumps, credentials, hashes, contents and general keystrokes
must never be passed to this diagnostic sink.

With logging disabled, lazy record builders do not run. Logging reuses already
available metadata and never requests extra remote or file-content reads. Tests
cover precision/signs, reasons, JSON escaping, redaction, exclusive/private file
creation and flushing. An isolated test subprocess enables the real sink and runs
quick/full workers against temporary local fixtures; it never initializes the
normal application logger or touches user profiles/logs. Its ignored fixture is
invoked automatically by the parent regression test, not a manual-session test.
See the English logging guide for launch commands and privacy limitations.

## SFTP batching regression tests

Repository builds patch `remotefs-ssh` using the licensed source copy under
`vendor/remotefs-ssh`. The production change is confined to the Russh reader's
adaptive batch windows and pending-transfer error propagation, plus feature gates
for unused blocking-backend helpers in Russh-only builds. Do not edit the
global Cargo registry cache. See `vendor/remotefs-ssh/SIDEMIRROR.md` for upstream
provenance, the exact patch scope, benchmarks, upgrade notes and distribution
limitations (Cargo strips patch overrides when publishing crates).

```sh
# Real SFTP framing over a local in-memory transport; no Docker or SSH needed.
cargo test --manifest-path vendor/remotefs-ssh/Cargo.toml --no-default-features --features russh sftp_reader_tests

# Manual bandwidth-simulated benchmark; no timing thresholds.
cargo test --manifest-path vendor/remotefs-ssh/Cargo.toml --no-default-features --features russh sftp_batching_benchmark -- --ignored --nocapture

# Actual OpenSSH integration tests; requires Docker.
cargo test --manifest-path vendor/remotefs-ssh/Cargo.toml --no-default-features --features russh ssh::sftp::tests::russh -- --test-threads=1
```

For Docker contexts with a non-default socket, set `DOCKER_HOST` to the context's
endpoint before running the OpenSSH tests. The fixture uses temporary containers
and verifies a comparison-style early close followed by a full 20 MiB read over
the same connection, in addition to existing ranges/seek/write tests.

## Activities

This paragraph gives a short overview of activities. Read the code and the
documentation for a clear idea of how the ui works.

There are many ways to implement a user interface. This project borrows what
works best from different frameworks:

- **Activities on top**: each "view" is an Activity, and an `Activity Manager`
  handles them. This approach is inspired by Android. It fits a ui that has
  different views, each one with its own components and logic. Activities work
  with the `Context`, which is a data holder used to share data between
  activities.
- **Activities display Applications**: each activity can show different
  **Applications**. An application contains a **View**, which is basically a list
  of **components**, each one with its properties. The view is a facade to the
  components and also handles the focus, which is the current active component.
  You cannot have more than one active component, so this must be handled; at the
  same time, focus must be given back to the previously active component if the
  current one is destroyed. The **Application** takes care of all this. To learn
  more, read <https://github.com/veeso/tui-realm>.
- **Components**: components are built around tui in order to reuse widgets. This
  is achieved through the `Component` trait, inspired by
  [React](https://reactjs.org/). Each component has its _Properties_ and can have
  its _States_. Each component must handle input events, accept new properties,
  and provide a method to **render** itself. This logic now lives in
  [tui-realm](https://github.com/veeso/tui-realm).
- **Messages: an Elm-based approach**: input events are handled with an approach
  inspired by [Elm](https://elm-lang.org/). In Elm you implement your ui using
  three basic functions: **update**, **view** and **init**. sidemirror implements
  the equivalent of the Elm update function as a large match case inside a
  recursive function, which you can find in the `update.rs` file inside each
  activity. This match case handles the messages produced by the components in
  response to incoming input events and causes the activity to change its state.

sidemirror implements a trait called `Activity`, a much reduced version of the
Android activity. This trait provides these methods:

- `on_create`: initializes the activity. The context is passed to the activity,
  which becomes the only owner of the Context until the activity terminates.
- `on_draw`: called each time the user interface should be updated. This is
  basically the run method of the activity, and it also handles input events. The
  interface should not be drawn on every call (this method may be called hundreds
  of times per second), but only when something has actually changed (for example
  after an input event).
- `will_umount`: returns whether the activity should be destroyed. If so, it
  returns an `ExitReason`, which indicates why the activity should terminate.
  Based on the reason, the activity manager chooses whether to stop the execution
  of sidemirror or to start a new activity, and which one.
- `on_destroy`: finalizes the activity and drops it. This method returns the
  Context to the caller (the activity manager).

### The Context

The context is a structure that holds data shared between activities. Every time
an Activity starts, the Context is taken by the activity, until it is destroyed,
where the context is finally returned to the activity manager. The context holds
the following data:

- The **Localhost**: the local host structure.
- The **File Transfer Params**: the current parameters used to connect to the
  remote.
- The **Config Client**: a structure that provides functions to access the user
  configuration.
- The **Store**: a key-value storage that can hold any kind of data. It can be
  used to share state between activities or to keep persistence for heavy or slow
  tasks (such as checking for updates).
- The **Terminal**: used to render the tui on the terminal.

## Achieving an abstract file transfer client

When the implementation of sidemirror started, in December 2020, file transfer was
at the core of the design, since it is at the heart of sidemirror. The first
implementation consisted of a `filetransfer` module that exposed a trait called
`FileTransfer`, which provided methods to generically interact with the remote
file system.

This changed over time, as different users asked for a dedicated library. In the
last quarter of 2021, [remotefs](https://github.com/veeso/remotefs-rs) was born:
an abstract library to work with remote device file systems. remotefs provides a
`RemoteFs` trait that exposes all of the core file-system functionalities, and
since version 0.8.0 it has replaced the `FileTransfer` trait.

The file transfer module still exists, but its only task is to build a
`RemoteFs` client implementation from the file transfer parameters through the
`RemoteFsBuilder`.
