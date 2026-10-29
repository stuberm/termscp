//! Cached syntax colors, computed once per document rather than on every redraw.
use std::path::Path;
use std::sync::LazyLock;

use syntect::easy::HighlightLines;
use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSet;

static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);
static THEMES: LazyLock<ThemeSet> = LazyLock::new(ThemeSet::load_defaults);

#[derive(Debug, Clone)]
pub struct Token {
    pub text: String,
    pub color: [u8; 3],
}

pub fn highlight(filename: &str, text: &str) -> Vec<Vec<Token>> {
    let syntaxes = &*SYNTAXES;
    let name = Path::new(filename)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(filename);
    let syntax = syntaxes
        .find_syntax_by_extension(name)
        .or_else(|| {
            Path::new(name)
                .extension()
                .and_then(|extension| extension.to_str())
                .and_then(|extension| syntaxes.find_syntax_by_extension(extension))
        })
        .or_else(|| {
            text.lines()
                .next()
                .and_then(|line| syntaxes.find_syntax_by_first_line(line))
        })
        .unwrap_or_else(|| syntaxes.find_syntax_plain_text());
    let theme = &THEMES.themes["base16-ocean.dark"];
    let mut highlighter = HighlightLines::new(syntax, theme);
    text.split_inclusive('\n')
        .map(|line| {
            let mut tokens: Vec<_> = match highlighter.highlight_line(line, syntaxes) {
                Ok(ranges) => ranges
                    .into_iter()
                    .filter_map(|(style, value)| {
                        let value = value.trim_end_matches(['\r', '\n']);
                        (!value.is_empty()).then(|| Token {
                            text: value.into(),
                            color: [style.foreground.r, style.foreground.g, style.foreground.b],
                        })
                    })
                    .collect(),
                Err(_) => vec![Token {
                    text: line.trim_end_matches(['\r', '\n']).into(),
                    color: [192, 197, 206],
                }],
            };
            let marker = if line.ends_with("\r\n") {
                " ␍"
            } else if !line.ends_with('\n') {
                " [no final newline]"
            } else {
                ""
            };
            if !marker.is_empty() {
                tokens.push(Token {
                    text: marker.into(),
                    color: [128, 128, 128],
                });
            }
            tokens
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_tokens_and_multiline_comments_are_colored() {
        let tokens = highlight(
            "test.rs",
            "fn main() { /* comment\nstill a comment */ let x = 42; }\n",
        );
        assert_eq!(tokens.len(), 2);
        assert_eq!(
            tokens[0]
                .iter()
                .map(|token| token.text.as_str())
                .collect::<String>(),
            "fn main() { /* comment"
        );
        assert!(
            tokens[0]
                .windows(2)
                .any(|pair| pair[0].color != pair[1].color)
        );
        let comment_color = tokens[0].last().unwrap().color;
        assert_eq!(tokens[1][0].color, comment_color);
    }

    #[test]
    fn unknown_files_fall_back_and_keep_newline_markers() {
        let tokens = highlight("file.unknown-extension", "café\r\nlast");
        assert_eq!(
            tokens[0]
                .iter()
                .map(|token| token.text.as_str())
                .collect::<String>(),
            "café ␍"
        );
        assert_eq!(
            tokens[1]
                .iter()
                .map(|token| token.text.as_str())
                .collect::<String>(),
            "last [no final newline]"
        );
        assert!(highlight("empty.rs", "").is_empty());
    }
}
