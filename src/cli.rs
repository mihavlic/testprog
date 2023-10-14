use std::{ffi::OsString, path::PathBuf};

const HELP: &str = "\
Progtest runner

USAGE:
  runner [SUBCOMMAND] [OPTIONS] [INPUT]

SUBCOMMANDS:
  build                     Build multiple binaries
  test                      Run multiple binaries, then feed them test files extracted from their neighboring samples archive
  run                       Run a single binary which inherits stdin
  clean                     Delete the output directory

FLAGS:
  -h, --help                Prints help information
  -v, --verbose             Increse output verbosity, can be specified second time to get trace messages
  -q, --quiet               Print only errors
  --unix                    Use LF line endings (select sample files which do not end with '_win')
  --windows                 Use CRLF line endings (select sample files which end with '_win')

OPTIONS:
  --root PATH               Sets the root path of the project, otherwise PWD is used
  --args STRING             Additional options to pass to the compiler, split by whitespace
  --override-args STRING    Options to pass to the compiler, split by whitespace. This completely overrides the default progtest arguments '-std=c++11 -Wall -pedantic'
  --subdir STRING           Name of a subdirectory within the samples archive to include, by default is 'CZE'
  --diff STRING             The command to run to diff mismatched outputs, it is interpreted by the shell, the variables $EXPECTED and $ACTUAL are present
  --ask 'SKIP' 'NO' 'ASK'
                            What to do when a test fails
                             - SKIP informs of the failure and continues
                             - NO doesn't prompt and immediatelly shows the diff
                             - ASK prompts the user. 

INPUT:
                            The names of the source files to use, relative to the root directory. For example 'pa1/01.c'
";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Build,
    Run,
    Test,
    Clean,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
    Lf,
    CrLf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Interactivity {
    Skip,
    Confirm,
    #[default]
    Ask,
}

#[derive(Debug)]
pub struct AppArgs {
    pub command: Command,
    pub root: PathBuf,
    pub quiet: bool,
    pub verbosity: u32,
    pub compiler_args: Option<String>,
    pub override_compiler_args: Option<String>,
    pub diff: Option<String>,
    pub sample_subdirs: Vec<OsString>,
    pub sample_ending: LineEnding,
    pub ask: Interactivity,
    pub targets: Vec<PathBuf>,
}

pub fn parse_args() -> Result<AppArgs, pico_args::Error> {
    let mut pargs = pico_args::Arguments::from_env();
    if pargs.contains(["-h", "--help"]) {
        print!("{}", HELP);
        std::process::exit(0);
    };

    #[cfg(target_os = "windows")]
    let auto_endings = LineEnding::CrLf;
    #[cfg(not(target_os = "windows"))]
    let auto_endings = LineEnding::Lf;

    #[rustfmt::skip]
    let args = AppArgs {
        root: pargs.opt_value_from_str("--root")?.unwrap_or_else(|| std::env::current_dir().unwrap()),
        quiet: pargs.contains(["-q", "--quiet"]),
        verbosity: pargs.contains(["-v", "--verbose"]) as u32 + pargs.contains(["-v", "--verbose"]) as u32,
        compiler_args: pargs.opt_value_from_str("--args")?,
        override_compiler_args: pargs.opt_value_from_str("--override-args")?,
        diff: pargs.opt_value_from_str("--diff")?,
        sample_subdirs: pargs.values_from_str("--subdir")?,
        sample_ending: match (pargs.contains("--windows"), pargs.contains("--unix")) {
            (true, _) => LineEnding::CrLf,
            (_, true) => LineEnding::Lf,
            _ => auto_endings,
        },
        ask: match pargs
            .opt_value_from_str::<_, String>("--ask")?
            .unwrap_or_default()
            .as_str()
        {
            "skip" | "SKIP" => Interactivity::Skip,
            "no" | "NO" => Interactivity::Confirm,
            "ask" | "ASK" => Interactivity::Ask,
            _ => Interactivity::Ask,
        },
        // these two musy go at the end
        command: get_command(&mut pargs)?,
        targets: pargs
            .finish()
            .into_iter()
            .map(|str| PathBuf::from(str))
            .collect(),
    };

    Ok(args)
}

fn get_command(pargs: &mut pico_args::Arguments) -> Result<Command, pico_args::Error> {
    match pargs.subcommand()?.as_deref() {
        None => {
            eprintln!("No subcommand provided");
            Err(pico_args::Error::MissingArgument)
        }
        Some("build") => Ok(Command::Build),
        Some("run") => Ok(Command::Run),
        Some("test") => Ok(Command::Test),
        Some("clean") => Ok(Command::Clean),
        Some(other) => {
            eprintln!("Unknown subcommand `{other}`");
            Err(pico_args::Error::MissingArgument)
        }
    }
}
