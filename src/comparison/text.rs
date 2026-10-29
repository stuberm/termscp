//! Bounded, read-only UTF-8 diff. Retains CRLF and final-newline differences.
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use similar::{Algorithm, ChangeTag, TextDiff};

use super::filesystem::CompareFs;

const MAX_BYTES: usize = 5 * 1024 * 1024;
const MAX_LINES: usize = 10_000;

#[derive(Debug, Clone)]
pub struct Line {
    pub left: Option<(usize, String)>,
    pub right: Option<(usize, String)>,
    pub changed: bool,
}

#[derive(Debug, Clone)]
pub struct TextView {
    pub title: String,
    pub lines: Vec<Line>,
}

pub fn read_text(
    fs: &mut dyn CompareFs,
    path: &Path,
    cancel: &AtomicBool,
) -> Result<String, String> {
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
        if bytes.contains(&0) {
            return Err("Binary file: byte comparison is available, text diff is not".into());
        }
        let text = String::from_utf8(bytes).map_err(|_| {
            "Non-UTF-8 file: byte comparison is available, text diff is not".to_string()
        })?;
        if text.lines().count() > MAX_LINES {
            return Err("Text diff supports at most 10,000 lines per side".into());
        }
        Ok(text)
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
    TextView { title, lines }
}

#[cfg(test)]
mod tests {
    use super::super::filesystem::LocalFs;
    use super::*;

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
        for bytes in [vec![0, 1], vec![255], vec![b'x'; MAX_BYTES + 1]] {
            std::fs::write(directory.path().join("file"), bytes).unwrap();
            assert!(read_text(&mut fs, Path::new("file"), &cancel).is_err());
        }
    }
}
