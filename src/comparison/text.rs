//! Bounded UTF-8 diff and chunk merging. Retains CRLF and final-newline differences.
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use similar::{Algorithm, ChangeTag, TextDiff};

use super::filesystem::CompareFs;

const MAX_BYTES: usize = 5 * 1024 * 1024;
const MAX_LINES: usize = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    Utf8Bom,
    Windows1252,
}

impl Encoding {
    pub fn label(self) -> &'static str {
        match self {
            Self::Utf8 => "UTF-8",
            Self::Utf8Bom => "UTF-8 BOM",
            Self::Windows1252 => "Windows-1252",
        }
    }
    fn encode(self, text: &str) -> Result<Vec<u8>, String> {
        match self {
            Self::Utf8 => Ok(text.as_bytes().to_vec()),
            Self::Utf8Bom => Ok([b"\xef\xbb\xbf".as_slice(), text.as_bytes()].concat()),
            Self::Windows1252 => {
                let (bytes, _, errors) = encoding_rs::WINDOWS_1252.encode(text);
                if errors {
                    return Err("Cannot copy chunk: characters cannot be represented in Windows-1252; copy the whole file to retain source encoding".into());
                }
                Ok(bytes.into_owned())
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct Document {
    pub text: String,
    pub bytes: Vec<u8>,
    pub encoding: Encoding,
}

fn decode(bytes: Vec<u8>) -> Result<Document, String> {
    if bytes
        .iter()
        .any(|&byte| byte < 32 && !matches!(byte, b'\t' | b'\r' | b'\n' | 12))
    {
        return Err("Binary file: byte comparison is available, text diff is not".into());
    }
    let (text, encoding) = if let Some(body) = bytes.strip_prefix(b"\xef\xbb\xbf") {
        (
            std::str::from_utf8(body)
                .map_err(|_| "Invalid UTF-8 after BOM")?
                .to_owned(),
            Encoding::Utf8Bom,
        )
    } else if let Ok(text) = std::str::from_utf8(&bytes) {
        (text.to_owned(), Encoding::Utf8)
    } else {
        let (text, errors) = encoding_rs::WINDOWS_1252.decode_without_bom_handling(&bytes);
        if errors {
            return Err("Cannot decode text as Windows-1252".into());
        }
        (text.into_owned(), Encoding::Windows1252)
    };
    if text.lines().count() > MAX_LINES {
        return Err("Text diff supports at most 10,000 lines per side".into());
    }
    Ok(Document {
        text,
        bytes,
        encoding,
    })
}

#[derive(Debug, Clone)]
pub struct Line {
    pub left: Option<(usize, String)>,
    pub right: Option<(usize, String)>,
    pub changed: bool,
}

#[derive(Debug, Clone)]
pub struct Highlights {
    pub left: Vec<Vec<super::syntax::Token>>,
    pub right: Vec<Vec<super::syntax::Token>>,
}

#[derive(Debug, Clone)]
pub struct TextView {
    pub title: String,
    pub left: String,
    pub right: String,
    pub lines: Vec<Line>,
    pub highlights: Box<Highlights>,
    pub left_document: Document,
    pub right_document: Document,
}

impl TextView {
    pub fn merged_bytes(
        &self,
        selected: usize,
        to_right: bool,
        whole: bool,
    ) -> Result<Vec<u8>, String> {
        let (source, target) = if to_right {
            (&self.left_document, &self.right_document)
        } else {
            (&self.right_document, &self.left_document)
        };
        if whole {
            return Ok(source.bytes.clone());
        }
        target
            .encoding
            .encode(&self.merged(selected, to_right, false)?)
    }

    /// Copy the selected contiguous change group, or the entire source snapshot.
    pub fn merged(&self, selected: usize, to_right: bool, whole: bool) -> Result<String, String> {
        let (source, target) = if to_right {
            (&self.left, &self.right)
        } else {
            (&self.right, &self.left)
        };
        if whole {
            return Ok(source.clone());
        }
        if !self.lines.get(selected).is_some_and(|line| line.changed) {
            return Err("Select a changed chunk first".into());
        }
        let mut start = selected;
        let mut end = selected + 1;
        while start > 0 && self.lines[start - 1].changed {
            start -= 1;
        }
        while end < self.lines.len() && self.lines[end].changed {
            end += 1;
        }
        let source_lines: Vec<_> = source.split_inclusive('\n').collect();
        let target_lines: Vec<_> = target.split_inclusive('\n').collect();
        let side = |line: &Line, source: bool| {
            if to_right == source {
                line.left.as_ref()
            } else {
                line.right.as_ref()
            }
            .map(|(number, _)| *number)
        };
        let before = self.lines[..start]
            .iter()
            .filter(|line| side(line, false).is_some())
            .count();
        let replaced = self.lines[start..end]
            .iter()
            .filter(|line| side(line, false).is_some())
            .count();
        let mut result = target_lines[..before].concat();
        for line in &self.lines[start..end] {
            if let Some(number) = side(line, true) {
                result.push_str(source_lines[number - 1]);
            }
        }
        result.push_str(&target_lines[before + replaced..].concat());
        Ok(result)
    }
}

pub fn read_text(
    fs: &mut dyn CompareFs,
    path: &Path,
    cancel: &AtomicBool,
) -> Result<Document, String> {
    let mut reader = fs.open(path)?;
    let result = (|| {
        let mut bytes = Vec::new();
        let mut buffer = [0; 32 * 1024];
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err("Diff cancelled".into());
            }
            let count = reader
                .read(&mut buffer)
                .map_err(|error| error.to_string())?;
            if count == 0 {
                break;
            }
            if bytes.len() + count > MAX_BYTES {
                return Err("Text diff supports at most 5 MiB per side".into());
            }
            bytes.extend_from_slice(&buffer[..count]);
        }
        decode(bytes)
    })();
    let finish = reader.finish().map_err(|error| error.to_string());
    finish?;
    result
}

fn visible_line(value: &str) -> String {
    if let Some(value) = value.strip_suffix("\r\n") {
        format!("{value} ␍")
    } else if let Some(value) = value.strip_suffix('\n') {
        value.into()
    } else {
        format!("{value} [no final newline]")
    }
}

pub fn diff_documents(title: String, left: Document, right: Document) -> TextView {
    let mut view = diff(title, &left.text, &right.text);
    view.left_document = left;
    view.right_document = right;
    view
}

pub fn diff(title: String, left: &str, right: &str) -> TextView {
    let diff = TextDiff::configure()
        .algorithm(Algorithm::Myers)
        .timeout(Duration::from_secs(2))
        .diff_lines(left, right);
    let mut lines = Vec::new();
    let mut removed = Vec::new();
    let mut added = Vec::new();
    let flush = |lines: &mut Vec<Line>,
                 removed: &mut Vec<(usize, String)>,
                 added: &mut Vec<(usize, String)>| {
        let count = removed.len().max(added.len());
        let mut a = removed.drain(..);
        let mut b = added.drain(..);
        for _ in 0..count {
            lines.push(Line {
                left: a.next(),
                right: b.next(),
                changed: true,
            });
        }
    };
    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Equal => {
                flush(&mut lines, &mut removed, &mut added);
                lines.push(Line {
                    left: change
                        .old_index()
                        .map(|index| (index + 1, visible_line(change.value()))),
                    right: change
                        .new_index()
                        .map(|index| (index + 1, visible_line(change.value()))),
                    changed: false,
                });
            }
            ChangeTag::Delete => removed.push((
                change.old_index().unwrap_or(0) + 1,
                visible_line(change.value()),
            )),
            ChangeTag::Insert => added.push((
                change.new_index().unwrap_or(0) + 1,
                visible_line(change.value()),
            )),
        }
    }
    flush(&mut lines, &mut removed, &mut added);
    TextView {
        highlights: Box::new(Highlights {
            left: super::syntax::highlight(&title, left),
            right: super::syntax::highlight(&title, right),
        }),
        title,
        lines,
        left_document: Document {
            text: left.into(),
            bytes: left.as_bytes().to_vec(),
            encoding: Encoding::Utf8,
        },
        right_document: Document {
            text: right.into(),
            bytes: right.as_bytes().to_vec(),
            encoding: Encoding::Utf8,
        },
        left: left.into(),
        right: right.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::filesystem::LocalFs;
    use super::*;

    #[test]
    fn mixed_encodings_compare_and_copy_without_loss() {
        let left = decode("café €\r\n".as_bytes().to_vec()).unwrap();
        let right = decode(b"caf\xe9 old\r\n".to_vec()).unwrap();
        assert_eq!(right.encoding, Encoding::Windows1252);
        let view = diff_documents("mixed".into(), left, right);
        assert_eq!(
            view.merged_bytes(0, true, false).unwrap(),
            b"caf\xe9 \x80\r\n"
        );
        assert_eq!(
            view.merged_bytes(0, true, true).unwrap(),
            "café €\r\n".as_bytes()
        );
        assert_eq!(
            view.merged_bytes(0, false, false).unwrap(),
            "café old\r\n".as_bytes()
        );
        let view = diff_documents(
            "equal".into(),
            decode("café".as_bytes().to_vec()).unwrap(),
            decode(b"caf\xe9".to_vec()).unwrap(),
        );
        assert!(view.lines.iter().all(|line| !line.changed));
        assert_ne!(view.left_document.bytes, view.right_document.bytes);
    }

    #[test]
    fn rejects_unrepresentable_chunk_and_preserves_bom() {
        let view = diff_documents(
            "test".into(),
            decode("你好\n".as_bytes().to_vec()).unwrap(),
            decode(b"caf\xe9\n".to_vec()).unwrap(),
        );
        assert!(
            view.merged_bytes(0, true, false)
                .unwrap_err()
                .contains("cannot be represented")
        );
        assert_eq!(
            view.merged_bytes(0, true, true).unwrap(),
            "你好\n".as_bytes()
        );
        let view = diff_documents(
            "bom".into(),
            decode(b"new\n".to_vec()).unwrap(),
            decode(b"\xef\xbb\xbfold\n".to_vec()).unwrap(),
        );
        assert_eq!(view.right_document.encoding, Encoding::Utf8Bom);
        assert_eq!(
            view.merged_bytes(0, true, false).unwrap(),
            b"\xef\xbb\xbfnew\n"
        );
    }

    #[test]
    fn merges_chunks_in_both_directions_preserving_bytes() {
        for (a, b) in [
            ("same\nold\nend\n", "same\nnew\nend\n"),
            ("same\ninserted\r\nend\n", "same\nend\n"),
            ("same\nend\n", "same\ninserted\nend\n"),
            ("a\r\n", "a"),
            ("", "new\n"),
            ("old", ""),
        ] {
            let view = diff("test".into(), a, b);
            let selected = view.lines.iter().position(|line| line.changed).unwrap();
            assert_eq!(view.merged(selected, true, false).unwrap(), a);
            assert_eq!(view.merged(selected, false, false).unwrap(), b);
            assert_eq!(view.merged(0, true, true).unwrap(), a);
            assert_eq!(view.merged(0, false, true).unwrap(), b);
        }
        let view = diff("test".into(), "a\nx\nb\ny\n", "a\nX\nb\nY\n");
        assert_eq!(view.merged(1, true, false).unwrap(), "a\nx\nb\nY\n");
        assert!(view.merged(0, true, false).is_err());
    }

    #[test]
    fn retains_newline_changes_and_aligns_replacements() {
        let view = diff("test".into(), "a\r\nb\n", "a\nb");
        assert!(view.lines.iter().all(|line| line.changed));
        assert!(view.lines[0].left.as_ref().unwrap().1.contains('␍'));
        assert!(
            view.lines[1]
                .right
                .as_ref()
                .unwrap()
                .1
                .contains("no final newline")
        );
        assert_eq!(diff("same".into(), "a\n", "a\n").lines.len(), 1);
    }

    #[test]
    fn rejects_binary_encoding_and_oversized_files() {
        let directory = tempfile::tempdir().unwrap();
        let mut fs = LocalFs::new(directory.path().into());
        let cancel = AtomicBool::new(false);
        for bytes in [vec![0, 1], vec![b'x'; MAX_BYTES + 1]] {
            std::fs::write(directory.path().join("file"), bytes).unwrap();
            assert!(read_text(&mut fs, Path::new("file"), &cancel).is_err());
        }
    }
}
