# Sidemirror SFTP reader patch

This directory contains the published `remotefs-ssh` **1.0.0** crate, under its
original MIT license. Its provenance is retained in `.cargo_vcs_info.json`.
The upstream crate checksum is
`ac4c22b5d9dda14562e028c1acfe32117d85ec7d7114d34b54aac3c4af6d8be4`.
Upstream: <https://github.com/remotefs-rs/remotefs-rs-ssh>.

The root `Cargo.toml` selects this copy using `[patch.crates-io]`. Do not modify
Cargo's global registry cache: that would not reproduce in CI or other clones.
The remainder of the upstream crate (including other SSH backends and SCP)
is intentionally retained, apart from the feature-gating adjustments noted below. Root dprint excludes this third-party directory so
formatting does not rewrite unrelated upstream sources. Local build output is
ignored by this directory's `.gitignore`.

## Changes

SFTP reader changes are limited to `src/ssh/backend/russh/stream.rs`:

- Start with a **64 KiB** batch and do not speculatively fetch its successor
  until the consumer requests data beyond that probe.
- Grow windows **64 KiB → 256 KiB → 1 MiB → 4 MiB → 16 MiB**. The upper batch size
  remains the upstream value. Ramp-up batches split into up to four parallel
  tasks, with chunks between 64 KiB and the upstream 4 MiB maximum.
- Continue bounded large-file prefetch after the probe. This is not a promise to
  read only the bytes requested: sustained reads intentionally read ahead, and
  a consumer that reads beyond the probe may trigger larger windows.
- Await pending READ/CLOSE work and propagate its errors during `finish` and
  `seek`; upstream silently discarded those task errors. Never abort a task in
  a way that skips awaiting its remote CLOSE. The upstream best-effort Drop
  behavior remains unchanged; application readers must still call `finish`.
- Reset the adaptive probe after a successful seek. Zero-length reads do not
  advance the prefetch state.

Existing explicit range limits, byte order, short-read handling, unknown-size
EOF behavior, seeking capabilities and complete-file verification remain intact.
The change applies to all Russh SFTP streams, including transfers and text diffs,
not only folder comparison. No file contents or comparison modes are changed.

Additional feature-gating fixes in `src/ssh.rs`, `src/ssh/scp.rs` and
`src/ssh/sftp.rs` compile the private blocking constructors and ranged-reader
module only when `libssh` or `libssh2` needs them (or when testing the reader).
This removes unused-code warnings in Russh-only application builds without
suppressing warnings globally or dropping alternate-backend support.

Tests added:

- `src/ssh/backend/russh/sftp_reader_tests.rs` exercises the actual reader and
  `russh-sftp` client over a Tokio duplex transport with real SFTP framing.
  It requires no SSH server, network, credentials or Docker. It covers byte
  counts, read-ahead bounds, ranges, seeking, short reads, unknown sizes,
  truncation, zero-length reads, and pending READ/CLOSE failures.
- `should_read_large_files_and_finalize_an_early_probe` in
  `src/ssh/sftp/tests/russh.rs` checks an early close and subsequent full 20 MiB
  read using an actual OpenSSH server through upstream's Docker fixture.

## Repeatable checks

From the sidemirror repository root:

```sh
cargo test --manifest-path vendor/remotefs-ssh/Cargo.toml --no-default-features --features russh sftp_reader_tests
cargo test --manifest-path vendor/remotefs-ssh/Cargo.toml --no-default-features --features russh sftp_batching_benchmark -- --ignored --nocapture
cargo test --manifest-path vendor/remotefs-ssh/Cargo.toml --no-default-features --features russh ssh::sftp::tests::russh -- --test-threads=1
```

The last command needs Docker. If its socket is not `/var/run/docker.sock`, set
`DOCKER_HOST` to the active Docker context's endpoint. Testcontainers removes
its own temporary containers. The vendored standalone lockfile is retained from
the published crate; the sidemirror build uses the root lockfile.

## Measurements and tradeoffs

In the same debug-build protocol fixture, a 64 MiB file is exposed through a
simulated 128 MiB/s link (no SSH encryption). Reading a 64 KiB prefix and then
finalizing fetched **32 MiB before** and **64 KiB after**: **512× fewer bytes**.
Time to the first 64 KiB was approximately **0.455 s → 0.004 s**, and total time
including finalization **0.882 s → 0.004 s** in those runs.

A full 64 MiB read fetched exactly 64 MiB in both cases. It took about
**2.41 s before → 2.59 s after** in this fixture, with 16 versus 29 total file
opens, reflecting adaptive ramp-up overhead. A 16 KiB file fetched 16 KiB in
both cases (about 0.002 s). These are illustrative component measurements,
not guaranteed end-to-end speedups or measurements of Internet latency. The
fixture checks bytes and CLOSE completion but deliberately has no timing-based
pass/fail threshold. Equal-file-heavy or high-latency workloads may see ramp-up
cost; measure actual server performance before further tuning.

For CRLF-heavy input, normalized 64 KiB comparisons can need more than 64 KiB
of raw data. The fixture also tests a 128 KiB consumed prefix and bounds fetched
data to 1,376,256 bytes (the first three windows), not the exact 64 KiB bound.

## Maintenance and distribution

When upgrading remotefs-ssh, compare this patch with the new upstream reader,
rerun both protocol and OpenSSH tests, and update the root lockfile. Remove the
patch when upstream provides an equivalent fix. No upstream PR or release is
implied by this local patch.

This override is used by repository/source builds and binaries built from them.
Cargo strips `[patch]` overrides when publishing a crate. Before publishing
sidemirror to crates.io, upstream the fix or use a published patched dependency;
do not assume that `cargo publish` distributes this override.
