use clap::{ColorChoice, Parser, ValueEnum};
use std::{ffi::OsString, path::PathBuf};

#[derive(Copy, Debug, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
pub enum Command {
    /// Build multiple binaries
    Build,
    /// Run multiple binaries, then feed them test files extracted from their neighboring samples archive
    Run,
    /// Run a single binary which inherits stdin
    Test,
    /// Delete the output directory
    Clean,
}

#[derive(Copy, Debug, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
pub enum Interactivity {
    /// Informs of the failure and continues
    Skip,
    /// Doesn't prompt and immediatelly shows the diff
    No,
    /// Prompts the user
    Yes,
}

#[derive(Copy, Debug, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
pub enum Os {
    /// Use LF (sample files *not* ending in _win.txt)
    Unix,
    /// Use CRLF (sample files ending in _win.txt)
    Windows,
}

#[derive(Debug, Parser)]
#[command(name = "testprog", about = "A program tester to run progtest locally", long_about = None)]
pub struct Arguments {
    /// Sets the root path of the project, otherwise PWD is used
    #[arg(long, value_name = "DIR", default_value_os_t = std::env::current_dir().unwrap(), )]
    pub root: PathBuf,
    /// Increase output verbosity, can be specified second time to get trace messages
    #[arg(
        long,
        short,
        action = clap::ArgAction::Count
    )]
    pub verbose: u8,
    // Print only errors
    #[arg(long, short, conflicts_with = "verbose")]
    pub quiet: bool,
    /// Print only errors
    #[arg(long = "options", value_name = "STRING")]
    pub compiler_args: Option<String>,
    /// Additional options to pass to the compiler, split by whitespace
    #[arg(
        long = "override-options",
        conflicts_with = "compiler_args",
        value_name = "STRING"
    )]
    pub override_compiler_args: Option<String>,
    /// Options to pass to the compiler, split by whitespace
    ///
    /// This completely overrides the default progtest arguments '-std=c++11 -Wall -pedantic'
    #[arg(long, value_name = "STRING")]
    pub diff: Option<String>,
    /// The command to run to diff mismatched outputs
    ///
    /// It is interpreted by the shell, the variables $EXPECTED and $ACTUAL are present
    #[arg(long = "subdir", value_name = "STRING", default_values_os_t = [OsString::from("CZE")])]
    /// Name of a subdirectory within the samples archive to include
    pub sample_subdirs: Vec<OsString>,
    /// Select os' sample file variant to use (line endings)
    #[arg(long, value_enum, default_value_t = Os::Unix)]
    pub os: Os,
    /// Whether to prompt the user
    #[arg(long, value_enum, default_value_t = Interactivity::Yes)]
    pub ask: Interactivity,
    /// The action to perform
    #[arg(long, value_enum, default_value_t = ColorChoice::Auto)]
    pub color: ColorChoice,
    /// The action to perform
    #[arg(value_enum)]
    pub command: Command,
    /// The names of the source files to use, relative to the root directory
    #[arg(value_name = "DIR")]
    pub targets: Vec<PathBuf>,
}
