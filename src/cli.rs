use clap::{Args, ColorChoice, Parser, Subcommand, ValueEnum};
use std::{ffi::OsString, path::PathBuf};

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

#[derive(Debug, Args)]
pub struct BuildOpts {
    /// Define a preprocessor variable
    #[arg(long = "define", short = 'D', value_name = "<macroname>=<value>", action = clap::ArgAction::Append)]
    pub defines: Vec<String>,
    /// Additional options to pass to the compiler, split by whitespace
    #[arg(long = "options", value_name = "STRING")]
    pub compiler_args: Option<String>,
    /// This disables the default progtest arguments '-std=c++11 -Wall -pedantic'
    #[arg(long, value_name = "STRING")]
    pub no_default_args: bool,
    /// The names of the source files to use, relative to the root directory
    #[arg(value_name = "DIR")]
    pub targets: Vec<PathBuf>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Build multiple binaries
    Build {
        #[clap(flatten)]
        build_options: BuildOpts,
    },
    /// Build binaries, then run specified command with them
    With {
        #[clap(flatten)]
        build_options: BuildOpts,
        /// Command and arguments to execute as the `with` command, the placeholder {bin} denotes the path to the built binaries, it is appended to the arguments if omitted
        #[arg(last = true)]
        with: Vec<OsString>,
    },
    /// Run a single binary which inherits stdin
    Run {
        #[clap(flatten)]
        build_options: BuildOpts,
    },
    /// Run multiple binaries, then feed them test files extracted from their neighboring samples archive
    Test {
        #[clap(flatten)]
        build_options: BuildOpts,
        /// The command to run to diff mismatched outputs
        ///
        /// It is interpreted by the shell, the variables $INPUT, $EXPECTED, and $ACTUAL are present
        /// Command and arguments to execute as the `with` command, the placeholder {bin} denotes the
        /// path to the built binaries, it is appended to the arguments if omitted
        #[arg(long, value_name = "STRING")]
        diff: Option<String>,
    },
    /// Convert a sample .tar.gz archive to a .sample file
    Convert {
        archive: PathBuf,
        output: Option<PathBuf>,
        /// Name of a subdirectory within the samples archive to include
        #[arg(long = "subdir", value_name = "STRING", default_values_os_t = [OsString::from("CZE")])]
        sample_subdirs: Vec<OsString>,
    },
    /// Delete the output directory
    Clean,
}

impl Command {
    pub fn get_build_options(&self) -> Option<&BuildOpts> {
        match self {
            Command::Build { build_options }
            | Command::With { build_options, .. }
            | Command::Run { build_options }
            | Command::Test { build_options, .. } => Some(build_options),
            Command::Clean | Command::Convert { .. } => None,
        }
    }
}

#[derive(Debug, Parser)]
#[command(name = "testprog", about = "A program tester to run progtest locally", long_about = None)]
pub struct Arguments {
    /// Sets the root path of the project, otherwise PWD is used
    #[arg(long, value_name = "DIR", default_value_os_t = std::env::current_dir().unwrap())]
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
    /// Select os' sample file variant to use (line endings)
    #[arg(long, value_enum, default_value_t = Os::Unix)]
    pub os: Os,
    /// Whether to prompt the user
    #[arg(long, value_enum, default_value_t = Interactivity::Yes)]
    pub ask: Interactivity,
    /// The action to perform
    #[arg(long, value_enum, default_value_t = ColorChoice::Auto)]
    pub color: ColorChoice,

    #[clap(subcommand)]
    pub command: Command,
}
