//! Streaming CRLF normalization; all other bytes remain significant.
use std::io::{self, BufRead, BufReader, Read};

const INPUT_BUFFER_SIZE: usize = 8 * 1024;

pub(super) struct Normalized<R: Read> {
    reader: BufReader<R>,
    pending_cr: bool,
    /// Consumed raw bytes, excluding any unconsumed buffered read-ahead.
    pub raw_size: u64,
}

impl<R: Read> Normalized<R> {
    pub fn new(reader: R) -> Self {
        Self {
            reader: BufReader::with_capacity(INPUT_BUFFER_SIZE, reader),
            pending_cr: false,
            raw_size: 0,
        }
    }
}

impl<R: Read> Read for Normalized<R> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        loop {
            let input = self.reader.fill_buf()?;
            if input.is_empty() {
                return if self.pending_cr {
                    self.pending_cr = false;
                    output[0] = b'\r';
                    Ok(1)
                } else {
                    Ok(0)
                };
            }
            let mut written = 0;
            let mut consumed = 0;
            if self.pending_cr {
                // Only CR followed by LF is removed. A CR at the end of an
                // input chunk stays pending until its successor (or EOF) is known.
                self.pending_cr = false;
                if input[0] != b'\n' {
                    output[0] = b'\r';
                    written = 1;
                }
            }
            while written < output.len() && consumed < input.len() {
                let length = (output.len() - written).min(input.len() - consumed);
                let end = consumed + length;
                // Include one look-ahead byte so a CR at the output boundary
                // can still be recognized as part of CRLF without emitting it.
                let search_end = (end + 1).min(input.len());
                if let Some(position) = memchr::memmem::find(&input[consumed..search_end], b"\r\n")
                {
                    output[written..written + position]
                        .copy_from_slice(&input[consumed..consumed + position]);
                    written += position;
                    consumed += position + 1; // Drop CR; copy its LF on the next pass.
                } else {
                    self.pending_cr = end == input.len() && input[end - 1] == b'\r';
                    let copied = length - usize::from(self.pending_cr);
                    output[written..written + copied]
                        .copy_from_slice(&input[consumed..consumed + copied]);
                    written += copied;
                    consumed = end;
                }
            }
            self.reader.consume(consumed);
            self.raw_size += consumed as u64;
            if written != 0 {
                return Ok(written);
            }
            // A chunk containing only a trailing CR must not look like EOF.
        }
    }
}

#[cfg(test)]
#[path = "line_endings_tests.rs"]
mod tests;
