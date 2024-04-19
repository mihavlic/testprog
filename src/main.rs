mod cli;
mod database;
mod fs;
mod logger;
mod samples;

use bstr::ByteSlice;
use clap::{ColorChoice, Parser};
use cli::{Arguments, Command};
use database::CacheEntry;
use fs::{check_status, print_args, AlreadyReported};
use nu_ansi_term::Color;
use std::{
    ffi::OsString, io::Write, os::unix::prelude::OsStrExt, path::Path, process::ChildStdin, rc::Rc,
};

use crate::{database::Database, samples::subcommand_convert};

#[macro_export]
macro_rules! bail {
    ($($a:tt)*) => {{
        log::error!($($a)*);
        return Err(AlreadyReported);
    }};
}

fn main() {
    let result = main_();
    if result.is_err() {
        std::process::exit(1);
    }
}

fn init() -> Arguments {
    let mut args = Arguments::parse();

    let level = match (args.quiet, args.verbose) {
        (true, _) => log::LevelFilter::Error,
        (false, 0) => log::LevelFilter::Info,
        (false, 1) => log::LevelFilter::Debug,
        (false, _) => log::LevelFilter::Trace,
    };

    if let clap::ColorChoice::Auto = args.color {
        let supported = anstyle_query::term_supports_ansi_color();
        args.color = match supported {
            true => clap::ColorChoice::Always,
            false => clap::ColorChoice::Never,
        };
    };

    logger::make_logger_from_env()
        .max_level(level)
        .print_level(true)
        .color(matches!(args.color, clap::ColorChoice::Always))
        .install();
    args
}

fn main_() -> Result<(), AlreadyReported> {
    let args = init();

    log::trace!("{args:#?}");

    let out_dir = args.root.join("out");
    let cache_file = out_dir.join("cache.json");

    match &args.command {
        Command::Run { build_options } => {
            if build_options.targets.len() != 1 {
                bail!("The 'run' subcommand expects a single target");
            }
        }
        Command::Clean => {
            return fs::remove_dir_all(&out_dir);
        }
        _ => {}
    };

    if !out_dir.exists() {
        fs::create_dir_all(&out_dir)?;
    }

    let mut cache = if args.no_cache {
        Database::new_empty(cache_file, out_dir.clone())
    } else {
        Database::new(cache_file, out_dir.clone())?
    };

    let binaries = match args.command.get_build_options() {
        Some(options) => {
            if options.targets.is_empty() {
                log::info!("No targets provided");
                return Ok(());
            }

            let mut errors = false;
            let binaries = options
                .targets
                .iter()
                .filter_map(|file| {
                    cache
                        .build_file(&file, options)
                        .map_err(|_| {
                            errors = true;
                            ()
                        })
                        .ok()
                })
                .collect();

            if !args.no_cache {
                _ = cache.save_to_file();
            }
            binaries
        }
        _ => vec![],
    };

    match &args.command {
        Command::Build { .. } => {}
        Command::With { with, .. } => subcommand_with(&binaries, with),
        Command::Run { .. } => {
            let entry = binaries.first().unwrap();
            log::info!("Running {}", entry.source.display());
            exec(&mut std::process::Command::new(&entry.binary))?;
        }
        Command::Test { diff, .. } => subcomand_test(&binaries, &out_dir, &args, diff.as_deref()),
        Command::Convert {
            archive,
            output,
            sample_subdirs,
        } => {
            let output = output.clone().unwrap_or_else(|| {
                let mut path = archive.clone();
                if let Some(str) = archive.file_name().unwrap().to_str() {
                    let found = [".gz", ".tgz", ".tar.gz"]
                        .iter()
                        .flat_map(|ext| str.strip_suffix(*ext))
                        .next();
                    if let Some(found) = found {
                        path = archive.with_file_name(found);
                    }
                }
                path.with_extension("samples")
            });

            subcommand_convert(&out_dir, &archive, &output, &args, &sample_subdirs)?;
        }
        Command::Clean => unreachable!(),
    }

    Ok(())
}

fn subcommand_with(entry_paths: &[Rc<CacheEntry>], arguments: &[OsString]) {
    let artifacts = entry_paths.iter().map(|p| p.binary.as_os_str().to_owned());
    let mut arguments = arguments.to_owned();

    let mut bin_subsituted = false;
    let mut i = 0;
    while i < arguments.len() {
        let current = &arguments[i];
        if let b"{bin}" = current.as_bytes() {
            arguments.splice(i..=i, artifacts.clone());
            i += entry_paths.len();
            bin_subsituted = true;
        } else {
            i += 1;
        }
    }
    if !bin_subsituted {
        arguments.extend(artifacts);
    }

    _ = exec(std::process::Command::new(&arguments[0]).args(&arguments[1..]));
}

fn exec(command: &mut std::process::Command) -> fs::Result<()> {
    print_args(command);

    use std::os::unix::process::CommandExt;
    #[cfg(unix)]
    let status = Err(command.exec());
    #[cfg(not(unix))]
    let status = command.status();

    check_status("child", status)
}

fn subcomand_test(
    entry_paths: &[Rc<CacheEntry>],
    out_dir: &Path,
    args: &cli::Arguments,
    diff_command: Option<&str>,
) {
    let (w_sender, w_receiver) = std::sync::mpsc::channel::<(Box<[u8]>, ChildStdin)>();
    let join = std::thread::spawn(move || {
        while let Ok((file, mut stdin)) = w_receiver.recv() {
            if let Err(e) = stdin.write_all(&file) {
                _ = fs::report_custom("writing to child stdin failed", e);
            }
        }
    });

    for paths in entry_paths {
        log::info!("Testing {}", paths.source.display());
        test_binary(paths, out_dir, args, &w_sender, diff_command);
    }

    drop(w_sender);
    _ = join.join();
}

fn test_binary(
    paths: &CacheEntry,
    out_dir: &Path,
    args: &Arguments,
    w_sender: &std::sync::mpsc::Sender<(Box<[u8]>, ChildStdin)>,
    diff_command: Option<&str>,
) -> Option<()> {
    let samples_out = paths.samples_out.as_ref()?;
    let contents = fs::read(samples_out).ok()?;

    let diff_path = out_dir.join("diff");
    _ = fs::create_dir_all(&diff_path);
    let file_name = paths.source.file_name().unwrap();

    let mut sections = samples::SampleIterator::new(&contents)?;
    while let Some(input) = sections.next() {
        let input_header = input.header.to_str().ok();
        let test_name = input_header.and_then(|s| s.strip_suffix(" in"));
        let output = sections.next();

        if input_header.is_none() {
            log::error!("input header `{}` isn't UTF8", input.header.to_str_lossy());
        };
        if input_header.is_some() && test_name.is_none() {
            log::error!(
                "input header `{}` doesn't end with ` in`",
                input.header.to_str_lossy()
            );
        };
        if output.is_none() {
            log::error!(
                "input header `{}` doesn't have an output section",
                input.header.to_str_lossy()
            );
        }
        if input_header.is_none() || test_name.is_none() || output.is_none() {
            continue;
        }

        let mut name = file_name.to_owned();
        name.push("_");
        name.push(test_name.unwrap());
        let test_diff_path = diff_path.join(name);
        _ = test_samples(
            test_name.unwrap().as_bytes(),
            input.body,
            output.unwrap().body,
            &test_diff_path,
            paths,
            w_sender,
            diff_command,
            args,
        );
    }

    Some(())
}

fn test_samples(
    name: &[u8],
    input: &[u8],
    output: &[u8],
    save_text_path: &Path,
    paths: &CacheEntry,
    w_sender: &std::sync::mpsc::Sender<(Box<[u8]>, ChildStdin)>,
    diff_command: Option<&str>,
    args: &Arguments,
) -> Result<(), AlreadyReported> {
    let mut child = std::process::Command::new(&paths.binary)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .spawn()
        .map_err(|e| fs::report_io_error("luanching binary", &paths.binary, e))?;

    let stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();

    w_sender
        .send((input.to_owned().into_boxed_slice(), stdin))
        .expect("Writing thread died!");

    let mut child_stdout = Vec::new();
    if let Err(e) = std::io::Read::read_to_end(&mut stdout, &mut child_stdout) {
        bail!("Failed to read from child stdout: {e}");
    }

    // we do not care about the exit status
    // TODO implement a timeout?
    _ = child.wait();

    let source = paths.source.display();
    let display = name.to_str_lossy();

    // janky configurable color
    let (red, green) = match args.color == ColorChoice::Never {
        true => (Color::Default, Color::Default),
        false => (Color::LightRed, Color::LightGreen),
    };
    let err = red.paint("Err");
    let ok = green.paint("Ok");

    if child_stdout != output {
        log::info!("{source} {display} {err}");
        _ = diff_failed(
            save_text_path,
            input,
            output,
            &child_stdout,
            args,
            diff_command,
        );
    } else {
        log::info!("{source} {display} {ok}");
    }
    Ok(())
}

fn diff_failed(
    path: &Path,
    input: &[u8],
    expected: &[u8],
    actual: &[u8],
    args: &Arguments,
    diff_command: Option<&str>,
) -> fs::Result<()> {
    let input_path = path.with_extension("in");
    let output_path = path.with_extension("out");
    let actual_path = path.with_extension("out.actual");

    fs::write(&input_path, input)?;
    fs::write(&output_path, expected)?;
    fs::write(&actual_path, actual)?;

    let Some(diff) = diff_command else {
        return Ok(());
    };

    let should_diff = match args.ask {
        cli::Interactivity::Skip => false,
        cli::Interactivity::No => true,
        cli::Interactivity::Yes => {
            let mut line = String::new();
            let should_diff = loop {
                eprint!("View diff? [Y/n] ");
                line.clear();
                if std::io::stdin().read_line(&mut line).is_err() {
                    break false;
                }
                match line.trim_start().chars().next() {
                    Some('Y' | 'y') | None => break true,
                    Some('N' | 'n') => break false,
                    _ => {}
                }
            };
            should_diff
        }
    };

    if should_diff {
        let mut builder = std::process::Command::new("sh");
        builder
            .arg("-c")
            .arg(diff)
            .env("INPUT", input_path)
            .env("EXPECTED", output_path)
            .env("ACTUAL", actual_path);

        print_args(&builder);
        _ = check_status("Diff command", builder.status());
    }

    Ok(())
}
