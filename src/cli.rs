//! ## CLI opts
//!
//! defines the types for main.rs types

#[cfg(test)]
mod compare_tests;
mod remote;

use std::path::PathBuf;
use std::time::Duration;

use argh::FromArgs;
pub use remote::{Remote, RemoteArgs};

use crate::activity_manager::NextActivity;
use crate::system::logging::LogLevel;

pub enum Task {
    Activity(NextActivity),
    /// Import ssh hosts from the specified ssh config file, or from the default location
    /// and save them as bookmarks.
    ImportSshHosts(Option<PathBuf>),
    ImportTheme(PathBuf),
    InstallUpdate,
    Compare(CompareArgs),
    Version,
}

#[derive(Default, FromArgs)]
#[argh(description = "
With no arguments, open the comparison profile picker.
Use `compare SOURCE DESTINATION` to compare folders, or `transfer` for the legacy connection screen.

Legacy positional arguments can be:
        - [address_a] [address_b] [local-wrkdir]
    OR
        - -b [bookmark-name_1] -b [bookmark-name_2] [local-wrkdir]

    and any combination of the above

Address syntax can be:

    - `protocol://user@address:port:wrkdir` for protocols such as Sftp, Scp, Ftp
    - `s3://bucket-name@region:profile:/wrkdir` for Aws S3 protocol
    - `gcs://<bucket>[:/working/directory]` for Google Cloud Storage (ADC)
    - `\\\\<server>[:port]\\<share>[\\path]` for SMB (on Windows)
    - `smb://[user@]<server>[:port]</share>[/path]` for SMB (on other systems)

Sidemirror — terminal file and folder comparison based on termscp.")]
pub struct Args {
    #[argh(subcommand)]
    pub nested: Option<ArgsSubcommands>,
    /// resolve address argument as a bookmark name
    #[argh(option, short = 'b')]
    pub bookmark: Vec<String>,
    /// enable TRACE log level
    #[argh(switch, short = 'D')]
    pub debug: bool,
    /// provide password from CLI; if you need to provide multiple passwords, use multiple -P flags.
    /// In case just respect the order of the addresses
    #[argh(option, short = 'P')]
    pub password: Vec<String>,
    /// disable logging
    #[argh(switch, short = 'q')]
    pub quiet: bool,
    /// set UI ticks; default 10ms
    #[argh(option, short = 'T', default = "10")]
    pub ticks: u64,
    /// print version
    #[argh(switch, short = 'v')]
    pub version: bool,
    /// obsolete: rejected because native credential storage is required
    #[argh(switch)]
    pub wno_keyring: bool,
    // -- positional
    #[argh(positional, description = "address1 address2 local-wrkdir")]
    pub positional: Vec<String>,
}

#[derive(FromArgs)]
#[argh(subcommand)]
pub enum ArgsSubcommands {
    Compare(CompareArgs),
    Transfer(TransferArgs),
    Config(ConfigArgs),
    ImportSshHosts(ImportSshHostsArgs),
    ImportTheme(ImportThemeArgs),
    Update(UpdateArgs),
}

#[derive(Default, FromArgs)]
/// compare local/remote folders; configure all protocols in the profile editor
#[argh(subcommand, name = "compare")]
pub struct CompareArgs {
    /// source and destination: local folder, sftp/scp/ftp/ftps/http(s) URL, or bookmark:NAME
    #[argh(positional)]
    pub roots: Vec<String>,
    /// reopen a saved comparison profile
    #[argh(option)]
    pub profile: Option<String>,
    /// save a new profile with this name
    #[argh(option)]
    pub save: Option<String>,
    /// ignore glob matching a relative path (repeatable; matched directories are pruned)
    #[argh(option)]
    pub ignore: Vec<String>,
    /// start with differences only
    #[argh(switch)]
    pub differences: bool,
    /// prompt separately for each remote endpoint password (never saved)
    #[argh(switch)]
    pub password_prompt: bool,
}

#[derive(Default, FromArgs)]
/// open the legacy file-transfer browser or connection/bookmark screen
#[argh(subcommand, name = "transfer")]
pub struct TransferArgs {
    /// remote address(es) and optional local working directory
    #[argh(positional)]
    pub positional: Vec<String>,
    /// connect using a saved connection bookmark (repeatable)
    #[argh(option, short = 'b')]
    pub bookmark: Vec<String>,
    /// remote password (repeatable; discouraged, prefer a saved bookmark)
    #[argh(option, short = 'P')]
    pub password: Vec<String>,
}

#[derive(FromArgs)]
/// open sidemirror configuration
#[argh(subcommand, name = "config")]
pub struct ConfigArgs {}

#[derive(FromArgs)]
/// update sidemirror to the latest version
#[argh(subcommand, name = "update")]
pub struct UpdateArgs {}

#[derive(FromArgs)]
/// import ssh hosts from the specified ssh config file, or from the default location
/// and save them as bookmarks.
#[argh(subcommand, name = "import-ssh-hosts")]
pub struct ImportSshHostsArgs {
    #[argh(positional)]
    /// optional ssh config file; if not specified, the default location will be used
    pub ssh_config: Option<PathBuf>,
}

#[derive(FromArgs)]
/// import the specified theme
#[argh(subcommand, name = "theme")]
pub struct ImportThemeArgs {
    #[argh(positional)]
    /// theme file
    pub theme: PathBuf,
}

pub struct RunOpts {
    pub remote: RemoteArgs,
    pub keyring: bool,
    pub ticks: Duration,
    pub log_level: LogLevel,
    pub task: Task,
}

impl RunOpts {
    pub fn config() -> Self {
        Self {
            task: Task::Activity(NextActivity::SetupActivity),
            ..Default::default()
        }
    }

    pub fn update() -> Self {
        Self {
            task: Task::InstallUpdate,
            ..Default::default()
        }
    }

    pub fn import_ssh_hosts(ssh_config: Option<PathBuf>, keyring: bool) -> Self {
        Self {
            task: Task::ImportSshHosts(ssh_config),
            keyring,
            ..Default::default()
        }
    }

    pub fn import_theme(theme: PathBuf) -> Self {
        Self {
            task: Task::ImportTheme(theme),
            ..Default::default()
        }
    }
}

impl Default for RunOpts {
    fn default() -> Self {
        Self {
            remote: RemoteArgs::default(),
            ticks: Duration::from_millis(10),
            keyring: true,
            log_level: LogLevel::Info,
            task: Task::Compare(CompareArgs::default()),
        }
    }
}
