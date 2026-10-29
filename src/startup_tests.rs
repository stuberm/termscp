use argh::FromArgs;

use super::*;

fn parse_options(arguments: &[&str]) -> RunOpts {
    parse_args(Args::from_args(&["sidemirror"], arguments).unwrap()).unwrap()
}

#[test]
fn default_startup_opens_comparison_profiles() {
    for options in [RunOpts::default(), parse_options(&[])] {
        let Task::Compare(compare) = options.task else {
            panic!("expected comparison picker");
        };
        assert!(compare.roots.is_empty());
        assert!(compare.profile.is_none());
    }
}

#[test]
fn default_comparison_respects_global_switches() {
    let args = Args::from_args(&["sidemirror"], &["--wno-keyring", "--quiet"]).unwrap();
    assert!(
        parse_args(args)
            .err()
            .unwrap()
            .contains("Native credential storage is required")
    );
    let options = parse_options(&["--quiet"]);
    assert!(matches!(options.task, Task::Compare(_)));
    assert!(options.keyring);
    assert!(matches!(options.log_level, LogLevel::Off));
    assert!(matches!(parse_options(&["-v"]).task, Task::Version));
}

#[test]
fn explicit_compare_profiles_and_configuration_still_work() {
    let options = parse_options(&["compare", "--profile", "website"]);
    let Task::Compare(compare) = options.task else {
        panic!("expected comparison");
    };
    assert_eq!(compare.profile.as_deref(), Some("website"));
    assert!(matches!(
        parse_options(&["config"]).task,
        Task::Activity(NextActivity::SetupActivity)
    ));
}

#[test]
fn transfer_is_explicit_and_legacy_remote_invocations_remain_compatible() {
    assert!(matches!(
        parse_options(&["transfer"]).task,
        Task::Activity(NextActivity::Authentication)
    ));
    for arguments in [
        vec!["transfer", "sftp://user@host"],
        vec!["sftp://user@host"],
        vec!["transfer", "-b", "server"],
        vec!["-b", "server"],
    ] {
        assert!(matches!(
            parse_options(&arguments).task,
            Task::Activity(NextActivity::FileTransfer)
        ));
    }
    let args = Args::from_args(
        &["sidemirror"],
        &["--wno-keyring", "transfer", "sftp://host"],
    )
    .unwrap();
    assert!(parse_args(args).is_err());
    let options = parse_options(&["--quiet", "transfer", "sftp://host"]);
    assert!(options.keyring);
    assert!(matches!(options.log_level, LogLevel::Off));
}

#[test]
fn password_without_an_endpoint_is_not_silently_ignored() {
    let args = Args::from_args(&["sidemirror"], &["-P", "secret"]).unwrap();
    assert!(parse_args(args).is_err());
}
