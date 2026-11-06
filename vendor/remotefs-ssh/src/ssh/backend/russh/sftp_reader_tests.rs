//! Real SFTP framing over an in-memory transport: no SSH server, Docker or secrets.
use std::future::poll_fn;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};

use super::*;

#[derive(Default)]
struct Metrics {
    bytes: AtomicU64,
    reads: AtomicUsize,
    opens: AtomicUsize,
    closes: AtomicUsize,
}

#[derive(Clone)]
struct Server {
    size: u64,
    reported_size: Option<u64>,
    short_read: usize,
    fail_at: Option<u64>,
    fail_close: bool,
    // Simulated single-link bandwidth, not an assertion about real network speed.
    bytes_per_second: Option<u64>,
}

impl Server {
    fn new(size: u64) -> Self {
        Self {
            size,
            reported_size: Some(size),
            short_read: usize::MAX,
            fail_at: None,
            fail_close: false,
            bytes_per_second: None,
        }
    }
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn string(bytes: &[u8]) -> Vec<u8> {
    let mut result = (bytes.len() as u32).to_be_bytes().to_vec();
    result.extend_from_slice(bytes);
    result
}

fn status(id: u32, code: u32) -> Vec<u8> {
    let mut response = vec![101]; // SSH_FXP_STATUS
    response.extend_from_slice(&id.to_be_bytes());
    response.extend_from_slice(&code.to_be_bytes());
    response.extend_from_slice(&string(b"fixture status"));
    response.extend_from_slice(&string(b""));
    response
}

fn value_at(offset: u64) -> u8 {
    // Identifies misplaced, repeated and missing bytes across chunk boundaries.
    ((offset.wrapping_mul(37) ^ (offset >> 8) ^ (offset >> 16)) % 251) as u8
}

async fn serve(mut transport: DuplexStream, server: Server, metrics: Arc<Metrics>) {
    let mut handle = 0_u32;
    while let Ok(length) = transport.read_u32().await {
        let mut packet = vec![0; length as usize];
        if transport.read_exact(&mut packet).await.is_err() {
            break;
        }
        let kind = packet[0];
        let id = u32_at(&packet, 1);
        let response = match kind {
            1 => {
                // SSH_FXP_INIT -> VERSION (no extensions)
                let mut response = vec![2];
                response.extend_from_slice(&3_u32.to_be_bytes());
                response
            }
            17 => {
                // STAT -> ATTRS
                let mut response = vec![105];
                response.extend_from_slice(&id.to_be_bytes());
                response
                    .extend_from_slice(&u32::from(server.reported_size.is_some()).to_be_bytes());
                if let Some(size) = server.reported_size {
                    response.extend_from_slice(&size.to_be_bytes());
                }
                response
            }
            3 => {
                // OPEN -> HANDLE
                metrics.opens.fetch_add(1, Ordering::SeqCst);
                handle += 1;
                let mut response = vec![102];
                response.extend_from_slice(&id.to_be_bytes());
                response.extend_from_slice(&string(&handle.to_be_bytes()));
                response
            }
            5 => {
                // READ -> DATA or EOF/failure
                metrics.reads.fetch_add(1, Ordering::SeqCst);
                let offset_at = 9 + u32_at(&packet, 5) as usize;
                let offset =
                    u64::from_be_bytes(packet[offset_at..offset_at + 8].try_into().unwrap());
                let requested = u32_at(&packet, offset_at + 8) as usize;
                if server.fail_at.is_some_and(|start| offset >= start) {
                    status(id, 4)
                } else if offset >= server.size {
                    status(id, 1)
                } else {
                    let count = requested
                        .min(server.short_read)
                        .min((server.size - offset) as usize);
                    if let Some(rate) = server.bytes_per_second {
                        tokio::time::sleep(Duration::from_secs_f64(count as f64 / rate as f64))
                            .await;
                    }
                    metrics.bytes.fetch_add(count as u64, Ordering::SeqCst);
                    let mut response = vec![103];
                    response.extend_from_slice(&id.to_be_bytes());
                    response.extend_from_slice(&(count as u32).to_be_bytes());
                    response.extend((offset..offset + count as u64).map(value_at));
                    response
                }
            }
            4 => {
                // CLOSE must be acknowledged, even after read failures.
                metrics.closes.fetch_add(1, Ordering::SeqCst);
                status(id, if server.fail_close { 4 } else { 0 })
            }
            _ => panic!("Unexpected SFTP request: {kind}"),
        };
        if transport.write_u32(response.len() as u32).await.is_err()
            || transport.write_all(&response).await.is_err()
        {
            break;
        }
    }
}

async fn fixture(server: Server, options: ReadOptions) -> (RusshSftpReader, Arc<Metrics>) {
    let (client, transport) = tokio::io::duplex(512 * 1024);
    let metrics = Arc::new(Metrics::default());
    tokio::spawn(serve(transport, server, metrics.clone()));
    let session = Arc::new(SftpSession::new(client).await.unwrap());
    let reader = RusshSftpReader::open(session, "/fixture".into(), &options)
        .await
        .unwrap();
    (reader, metrics)
}

async fn read(reader: &mut RusshSftpReader, buffer: &mut [u8]) -> io::Result<usize> {
    poll_fn(|cx| Pin::new(&mut *reader).poll_read(cx, buffer)).await
}

async fn consume(reader: &mut RusshSftpReader, bytes: usize) -> io::Result<Vec<u8>> {
    let mut result = Vec::new();
    let mut buffer = vec![0; 64 * 1024];
    while result.len() < bytes {
        let count = (bytes - result.len()).min(buffer.len());
        let count = read(reader, &mut buffer[..count]).await?;
        if count == 0 {
            break;
        }
        result.extend_from_slice(&buffer[..count]);
    }
    Ok(result)
}

fn assert_bytes(bytes: &[u8], offset: u64) {
    assert!(
        bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| *byte == value_at(offset + index as u64))
    );
}

fn assert_closed(metrics: &Metrics) {
    assert_eq!(
        metrics.opens.load(Ordering::SeqCst),
        metrics.closes.load(Ordering::SeqCst)
    );
}

#[tokio::test]
async fn sftp_probe_early_exit_fetches_only_64_kib_and_awaits_close() {
    let (mut reader, metrics) =
        fixture(Server::new(64 * 1024 * 1024), ReadOptions::default()).await;
    let bytes = consume(&mut reader, 64 * 1024).await.unwrap();
    assert_bytes(&bytes, 0);
    // Yield so premature speculative work would have a chance to start.
    tokio::time::sleep(Duration::from_millis(5)).await;
    assert_eq!(metrics.bytes.load(Ordering::SeqCst), INITIAL_BATCH_SIZE);
    assert!(reader.pending.is_none());
    Box::new(reader).finish().await.unwrap();
    assert_eq!(metrics.bytes.load(Ordering::SeqCst), INITIAL_BATCH_SIZE);
    assert_eq!(metrics.opens.load(Ordering::SeqCst), 1);
    assert_closed(&metrics);
}

#[tokio::test]
async fn sftp_consumption_after_probe_keeps_read_ahead_bounded() {
    let (mut reader, metrics) =
        fixture(Server::new(64 * 1024 * 1024), ReadOptions::default()).await;
    // CRLF normalization can consume beyond 64 KiB raw input to produce a
    // 64 KiB comparison chunk. Exercise that ramp-up, not just binary prefixes.
    let bytes = consume(&mut reader, 128 * 1024).await.unwrap();
    assert_bytes(&bytes, 0);
    Box::new(reader).finish().await.unwrap();
    let limit = INITIAL_BATCH_SIZE * (1 + BATCH_GROWTH + BATCH_GROWTH * BATCH_GROWTH);
    assert!(metrics.bytes.load(Ordering::SeqCst) <= limit);
    assert!(limit < 2 * 1024 * 1024);
    assert_closed(&metrics);
}

#[tokio::test]
async fn sftp_full_reads_preserve_bytes_and_grow_back_to_large_parallel_batches() {
    for size in [
        0,
        1,
        16 * 1024,
        INITIAL_BATCH_SIZE - 1,
        INITIAL_BATCH_SIZE,
        INITIAL_BATCH_SIZE + 1,
        23 * 1024 * 1024,
    ] {
        let (mut reader, metrics) = fixture(Server::new(size), ReadOptions::default()).await;
        let bytes = consume(&mut reader, size as usize + 1).await.unwrap();
        assert_eq!(bytes.len(), size as usize);
        assert_bytes(&bytes, 0);
        assert_eq!(read(&mut reader, &mut [0; 10]).await.unwrap(), 0);
        if size > BATCH_SIZE {
            assert_eq!(reader.next_batch_size, BATCH_SIZE);
            assert!(metrics.opens.load(Ordering::SeqCst) > SFTP_PIPELINE_DEPTH);
        }
        Box::new(reader).finish().await.unwrap();
        assert_eq!(metrics.bytes.load(Ordering::SeqCst), size);
        assert_closed(&metrics);
    }
}

#[tokio::test]
async fn sftp_partial_server_responses_do_not_skip_or_repeat_bytes() {
    for known in [false, true] {
        let mut server = Server::new(900_123);
        server.short_read = 1237;
        if !known {
            server.reported_size = None;
        }
        let (mut reader, metrics) = fixture(server, ReadOptions::default()).await;
        let bytes = consume(&mut reader, 1_000_000).await.unwrap();
        assert_eq!(bytes.len(), 900_123);
        assert_bytes(&bytes, 0);
        Box::new(reader).finish().await.unwrap();
        assert_eq!(metrics.bytes.load(Ordering::SeqCst), 900_123);
        assert_closed(&metrics);
    }
}

#[tokio::test]
async fn sftp_ranges_never_fetch_outside_the_requested_range() {
    for known in [false, true] {
        for (offset, length) in [(0, 0), (17, 300_019), (999_997, 100)] {
            let mut server = Server::new(1_000_000);
            if !known {
                server.reported_size = None;
            }
            let (mut reader, metrics) =
                fixture(server, ReadOptions::default().offset(offset).length(length)).await;
            assert!(!reader.seekable());
            let bytes = consume(&mut reader, 1_000_001).await.unwrap();
            let expected = length.min(1_000_000 - offset);
            assert_eq!(bytes.len() as u64, expected);
            assert_bytes(&bytes, offset);
            Box::new(reader).finish().await.unwrap();
            assert_eq!(metrics.bytes.load(Ordering::SeqCst), expected);
            assert_closed(&metrics);
        }
    }
}

#[tokio::test]
async fn sftp_seek_drains_pending_reads_and_resets_the_probe_window() {
    let (mut reader, metrics) =
        fixture(Server::new(20 * 1024 * 1024), ReadOptions::default()).await;
    consume(&mut reader, INITIAL_BATCH_SIZE as usize + 1)
        .await
        .unwrap();
    assert!(reader.pending.is_some());
    assert_eq!(
        reader.seek(SeekFrom::Start(1_234_567)).await.unwrap(),
        1_234_567
    );
    assert_eq!(reader.next_batch_size, INITIAL_BATCH_SIZE * BATCH_GROWTH);
    assert!(!reader.prefetch_enabled);
    let bytes = consume(&mut reader, INITIAL_BATCH_SIZE as usize)
        .await
        .unwrap();
    assert_bytes(&bytes, 1_234_567);
    assert!(reader.pending.is_none());
    assert_eq!(
        reader.seek(SeekFrom::End(-10)).await.unwrap(),
        20 * 1024 * 1024 - 10
    );
    let bytes = consume(&mut reader, 100).await.unwrap();
    assert_eq!(bytes.len(), 10);
    assert_bytes(&bytes, 20 * 1024 * 1024 - 10);
    Box::new(reader).finish().await.unwrap();
    assert_closed(&metrics);
}

#[tokio::test]
async fn sftp_unknown_size_streams_stop_at_eof_and_cannot_seek() {
    let mut server = Server::new(75_001);
    server.reported_size = None;
    let (mut reader, metrics) = fixture(server, ReadOptions::default()).await;
    assert!(!reader.seekable());
    assert_eq!(
        reader.seek(SeekFrom::Start(0)).await.unwrap_err().kind(),
        io::ErrorKind::Unsupported
    );
    let bytes = consume(&mut reader, 100_000).await.unwrap();
    assert_eq!(bytes.len(), 75_001);
    assert_bytes(&bytes, 0);
    Box::new(reader).finish().await.unwrap();
    assert_closed(&metrics);
}

#[tokio::test]
async fn sftp_pending_read_and_close_failures_are_not_hidden_by_finish_or_seek() {
    // Failure before any read: finish must await the task and expose its result.
    for close_failure in [false, true] {
        let mut server = Server::new(1_000_000);
        server.fail_close = close_failure;
        if !close_failure {
            server.fail_at = Some(0);
        }
        let (reader, metrics) = fixture(server, ReadOptions::default()).await;
        assert!(Box::new(reader).finish().await.is_err());
        assert_closed(&metrics);
    }
    // Failure only in speculative data: the consumed prefix is valid, but the
    // finalizer must not silently turn a failed prefetch into verified equality.
    for seek in [false, true] {
        let mut server = Server::new(10 * 1024 * 1024);
        server.fail_at = Some(INITIAL_BATCH_SIZE * (1 + BATCH_GROWTH));
        let (mut reader, metrics) = fixture(server, ReadOptions::default()).await;
        let bytes = consume(&mut reader, INITIAL_BATCH_SIZE as usize + 1)
            .await
            .unwrap();
        assert_bytes(&bytes, 0);
        assert!(reader.pending.is_some());
        if seek {
            assert!(reader.seek(SeekFrom::Start(0)).await.is_err());
            Box::new(reader).finish().await.unwrap();
        } else {
            assert!(Box::new(reader).finish().await.is_err());
        }
        assert_closed(&metrics);
    }
}

#[tokio::test]
async fn sftp_truncated_known_size_reads_fail_but_still_close_handles() {
    let mut server = Server::new(100_003);
    server.reported_size = Some(1_000_000);
    let (mut reader, metrics) = fixture(server, ReadOptions::default()).await;
    assert!(consume(&mut reader, 1_000_000).await.is_err());
    let _ = Box::new(reader).finish().await;
    assert_closed(&metrics);
}

#[tokio::test]
async fn sftp_zero_length_reads_do_not_start_additional_batches() {
    let (mut reader, metrics) =
        fixture(Server::new(10 * 1024 * 1024), ReadOptions::default()).await;
    consume(&mut reader, INITIAL_BATCH_SIZE as usize)
        .await
        .unwrap();
    for _ in 0..10 {
        assert_eq!(read(&mut reader, &mut []).await.unwrap(), 0);
    }
    assert!(reader.pending.is_none());
    Box::new(reader).finish().await.unwrap();
    assert_eq!(metrics.bytes.load(Ordering::SeqCst), INITIAL_BATCH_SIZE);
    assert_closed(&metrics);
}

#[tokio::test]
#[ignore = "manual SFTP batching benchmark, simulated bandwidth and no SSH encryption"]
async fn sftp_batching_benchmark() {
    for (label, size, consumed) in [
        ("early mismatch", 64 * 1024 * 1024, 64 * 1024),
        ("full matching file", 64 * 1024 * 1024, 64 * 1024 * 1024),
        ("small file", 16 * 1024, 16 * 1024),
    ] {
        let mut server = Server::new(size);
        server.bytes_per_second = Some(128 * 1024 * 1024);
        let start = Instant::now();
        let (mut reader, metrics) = fixture(server, ReadOptions::default()).await;
        let first = consume(&mut reader, 64 * 1024).await.unwrap();
        let first_elapsed = start.elapsed();
        assert_bytes(&first, 0);
        let mut total = first.len();
        while total < consumed as usize {
            let count = (consumed as usize - total).min(64 * 1024);
            let mut buffer = vec![0; count];
            let count = read(&mut reader, &mut buffer).await.unwrap();
            assert_ne!(count, 0);
            assert_bytes(&buffer[..count], total as u64);
            total += count;
        }
        Box::new(reader).finish().await.unwrap();
        assert_closed(&metrics);
        eprintln!(
            "{label}: first={:.3}s total={:.3}s consumed={} fetched={} opens={} closes={}",
            first_elapsed.as_secs_f64(),
            start.elapsed().as_secs_f64(),
            total,
            metrics.bytes.load(Ordering::SeqCst),
            metrics.opens.load(Ordering::SeqCst),
            metrics.closes.load(Ordering::SeqCst)
        );
    }
}
