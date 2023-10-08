use std::path::PathBuf;

const HELP: &str = "\
Progtest runner

USAGE:
  runner [SUBCOMMAND] [OPTIONS] [INPUT]

SUBCOMMANDS:
  build                 Builds multiple binaries.
  test                  Runs multiple binaries, then feeds them all test files extracted from the neighboring .tgz archive.
  run                   Runs a single binary which inherits stdin.
  clean                 Deletes the output directory.

FLAGS:
  -h, --help            Prints help information

OPTIONS:
  --root PATH           Sets the root path of the project, otherwise PWD is used.
  --no-default          Do not pass the default options to the compiler.
  --options STRING      String to pass to the compiler, split by whitespace.
  --diff STRING         The command to run to diff mismatched outputs, the paths of the sample and actual output will be appended as arguments.
                        If the string is empty, diffing is skipped without prompting the user.

ARGS:
  <INPUT>               The name of the programs/samples pair to run, for example 'pa1/01'
                        will use '$PWD/pa1/01.c' and '$PWD/pa1/01.tgz'
";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Build,
    Run,
    Test,
    Clean,
}

#[derive(Debug)]
pub struct AppArgs {
    pub command: Command,
    pub root: PathBuf,
    pub no_default: bool,
    pub compiler_args: Option<String>,
    pub diff: Option<String>,
    pub targets: Vec<PathBuf>,
}

pub fn parse_args() -> Result<AppArgs, pico_args::Error> {
    let mut pargs = pico_args::Arguments::from_env();
    if pargs.contains(["-h", "--help"]) {
        print!("{}", HELP);
        std::process::exit(0);
    };

    let command = match pargs.subcommand()?.as_deref() {
        None => crate::bail!("No subcommand provided"),
        Some("build") => Command::Build,
        Some("run") => Command::Run,
        Some("test") => Command::Test,
        Some("clean") => Command::Clean,
        Some(other) => crate::bail!("Unknown subcommand {other:?}"),
    };

    let args = AppArgs {
        command,
        root: pargs
            .opt_value_from_str("--root")?
            .unwrap_or_else(|| std::env::current_dir().unwrap()),
        no_default: pargs.contains("--no-default"),
        compiler_args: pargs.opt_value_from_str("--options")?,
        diff: pargs.opt_value_from_str("--diff")?,
        targets: pargs
            .finish()
            .into_iter()
            .map(|str| PathBuf::from(str))
            .collect(),
    };

    Ok(args)
}
