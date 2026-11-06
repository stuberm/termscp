use std::hint::black_box;
use std::time::Instant;

use super::*;

#[test]
fn normalizes_only_crlf_including_across_output_chunks() {
    let mut reader = Normalized::new(&b"a\r\nb\rc \t\n\r"[..]);
    let mut output = Vec::new();
    let mut byte = [0];
    while reader.read(&mut byte).unwrap() != 0 {
        output.push(byte[0]);
    }
    assert_eq!(output, b"a\nb\rc \t\n\r");
    assert_eq!(reader.raw_size, 10);
}

struct ShortReads<'a> {
    bytes: &'a [u8],
    maximum: usize,
}

impl Read for ShortReads<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let length = output.len().min(self.maximum).min(self.bytes.len());
        output[..length].copy_from_slice(&self.bytes[..length]);
        self.bytes = &self.bytes[length..];
        Ok(length)
    }
}

fn reference(bytes: &[u8]) -> Vec<u8> {
    bytes
        .iter()
        .enumerate()
        .filter_map(|(index, byte)| {
            if *byte == b'\r' && bytes.get(index + 1) == Some(&b'\n') {
                None
            } else {
                Some(*byte)
            }
        })
        .collect()
}

fn check_chunks(bytes: &[u8], input_size: usize, output_size: usize) {
    let mut reader = Normalized::new(ShortReads {
        bytes,
        maximum: input_size,
    });
    let mut output = Vec::new();
    let mut buffer = vec![0; output_size];
    loop {
        let previous = reader.raw_size;
        assert_eq!(reader.read(&mut []).unwrap(), 0);
        assert_eq!(reader.raw_size, previous);
        let count = reader.read(&mut buffer).unwrap();
        assert!((previous..=bytes.len() as u64).contains(&reader.raw_size));
        if count == 0 {
            break;
        }
        output.extend_from_slice(&buffer[..count]);
    }
    assert_eq!(
        output,
        reference(bytes),
        "input={input_size}, output={output_size}, bytes={bytes:?}"
    );
    assert_eq!(reader.raw_size, bytes.len() as u64);
    assert_eq!(reader.read(&mut buffer).unwrap(), 0);
    assert_eq!(reader.raw_size, bytes.len() as u64);
}

#[test]
fn exhaustive_short_inputs_match_reference_for_different_read_boundaries() {
    let alphabet = [0, b'\r', b'\n', 255];
    for length in 0..=6 {
        for mut value in 0..alphabet.len().pow(length) {
            let bytes: Vec<_> = (0..length)
                .map(|_| {
                    let byte = alphabet[value % alphabet.len()];
                    value /= alphabet.len();
                    byte
                })
                .collect();
            for input in [1, 2, 3, 8] {
                for output in [1, 2, 3, 8] {
                    check_chunks(&bytes, input, output);
                }
            }
        }
    }
}

#[test]
fn crlf_at_input_and_output_buffer_edges_preserves_raw_byte_accounting() {
    for boundary in [
        INPUT_BUFFER_SIZE - 1,
        INPUT_BUFFER_SIZE,
        INPUT_BUFFER_SIZE + 1,
        2 * INPUT_BUFFER_SIZE - 1,
    ] {
        let mut bytes = vec![b'x'; 2 * INPUT_BUFFER_SIZE + 8];
        bytes[boundary..boundary + 6].copy_from_slice(b"\r\n\r\r\n\r");
        bytes.push(b'\r');
        for input in [7, INPUT_BUFFER_SIZE, INPUT_BUFFER_SIZE + 1] {
            for output in [1, 3, INPUT_BUFFER_SIZE, 64 * 1024] {
                check_chunks(&bytes, input, output);
            }
        }
    }
}

#[test]
fn arbitrary_binary_bytes_match_reference_across_variable_short_reads() {
    let mut seed = 0x1234_abcd_u32;
    for round in 0..32 {
        let bytes: Vec<_> = (0..8192 + round * 31)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                seed as u8
            })
            .collect();
        check_chunks(&bytes, round * 17 + 1, round * 13 + 1);
    }
}

struct Scripted<'a> {
    chunks: std::collections::VecDeque<Result<&'a [u8], io::ErrorKind>>,
}

impl Read for Scripted<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        match self.chunks.pop_front().unwrap_or(Ok(&[])) {
            Ok(bytes) => {
                assert!(bytes.len() <= output.len());
                output[..bytes.len()].copy_from_slice(bytes);
                Ok(bytes.len())
            }
            Err(kind) => Err(io::Error::new(kind, "injected read failure")),
        }
    }
}

#[test]
fn interrupted_reads_do_not_drop_pending_cr_and_io_errors_never_look_like_eof() {
    let mut reader = Normalized::new(Scripted {
        chunks: [
            Ok(b"a\r".as_slice()),
            Err(io::ErrorKind::Interrupted),
            Ok(b"\n".as_slice()),
            Ok(b"b\r".as_slice()),
            Err(io::ErrorKind::Interrupted),
            Ok(b"c\r".as_slice()),
            Err(io::ErrorKind::Interrupted),
        ]
        .into(),
    });
    let mut output = Vec::new();
    let mut buffer = [0; 16];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => output.extend_from_slice(&buffer[..count]),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => panic!("{error}"),
        }
    }
    assert_eq!(output, b"a\nb\rc\r");
    assert_eq!(reader.raw_size, 7);
    let mut reader = Normalized::new(Scripted {
        chunks: [Ok(b"prefix\r".as_slice()), Err(io::ErrorKind::Other)].into(),
    });
    assert_eq!(reader.read(&mut buffer).unwrap(), 6);
    assert_eq!(&buffer[..6], b"prefix");
    assert_eq!(reader.raw_size, 7);
    assert_eq!(
        reader.read(&mut buffer).unwrap_err().kind(),
        io::ErrorKind::Other
    );
}

/// In-memory workload isolates CPU cost from filesystem/network latency.
#[test]
#[ignore = "manual CPU benchmark; run optimized with --ignored --nocapture"]
fn normalization_cpu_benchmark() {
    let size = 16 * 1024 * 1024;
    let repetitions = 8;
    let binary: Vec<_> = (0..=255).collect();
    for (name, pattern) in [
        (
            "LF text",
            b"some representative text with no carriage return\n".as_slice(),
        ),
        (
            "CRLF text",
            b"some representative text with a carriage return\r\n".as_slice(),
        ),
        ("binary", binary.as_slice()),
        ("lone CR", b"\r\r\r\r".as_slice()),
    ] {
        let mut data = pattern.repeat(size / pattern.len() + 1);
        data.truncate(size);
        let expected = size - data.windows(2).filter(|pair| *pair == b"\r\n").count();
        let started = Instant::now();
        for _ in 0..repetitions {
            let mut reader = Normalized::new(black_box(data.as_slice()));
            let mut buffer = [0; 64 * 1024];
            let mut count = 0;
            loop {
                let length = reader.read(&mut buffer).unwrap();
                black_box(&buffer[..length]);
                if length == 0 {
                    break;
                }
                count += length;
            }
            assert_eq!(count, expected);
            assert_eq!(reader.raw_size, size as u64);
        }
        let elapsed = started.elapsed();
        eprintln!(
            "{name}: {:.2} MiB/s ({:.3}s for {} MiB)",
            (size * repetitions) as f64 / (1024.0 * 1024.0) / elapsed.as_secs_f64(),
            elapsed.as_secs_f64(),
            size * repetitions / (1024 * 1024)
        );
    }
}
