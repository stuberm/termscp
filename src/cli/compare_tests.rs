use argh::FromArgs;

use super::{Args, ArgsSubcommands};

#[test]
fn parses_roots_profiles_and_repeatable_ignores() {
    let args = Args::from_args(
        &["sidemirror"],
        &[
            "compare",
            "./left",
            "sftp://user@host:22/path",
            "--ignore",
            ".git",
            "--ignore",
            "*.tmp",
            "--save",
            "example",
            "--password-prompt",
            "--differences",
        ],
    )
    .unwrap();
    let Some(ArgsSubcommands::Compare(compare)) = args.nested else {
        panic!("expected compare");
    };
    assert_eq!(compare.roots, vec!["./left", "sftp://user@host:22/path"]);
    assert_eq!(compare.ignore, vec![".git", "*.tmp"]);
    assert_eq!(compare.save.as_deref(), Some("example"));
    assert!(compare.password_prompt);
    assert!(compare.differences);
    let args = Args::from_args(
        &["sidemirror"],
        &["--wno-keyring", "compare", "--profile", "example"],
    )
    .unwrap();
    assert!(args.wno_keyring);
    let Some(ArgsSubcommands::Compare(compare)) = args.nested else {
        panic!("expected compare");
    };
    assert!(compare.roots.is_empty());
    assert_eq!(compare.profile.as_deref(), Some("example"));
}

#[test]
fn no_roots_opens_picker_command() {
    let args = Args::from_args(&["sidemirror"], &["compare"]).unwrap();
    let Some(ArgsSubcommands::Compare(compare)) = args.nested else {
        panic!("expected compare");
    };
    assert!(compare.roots.is_empty());
    assert!(compare.profile.is_none());
}
