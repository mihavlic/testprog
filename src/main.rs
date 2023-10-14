mod cli;
mod fs;
mod logger;

use clap::{ColorChoice, Parser};
use cli::{Arguments, Command, Os};
use fs::{report, report_io_error, AlreadyReported};
use nu_ansi_term::Color;
use std::{
    collections::HashMap,
    ffi::OsStr,
    os::unix::prelude::OsStrExt,
    path::{Path, PathBuf},
    process::ChildStdin,
};

#[macro_export]
macro_rules! bail {
    ($($a:tt)*) => {{
        log::error!($($a)*);
        return Err(AlreadyReported);
    }};
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Default)]
struct SerdeCacheEntry {
    source_hash: String,
    samples_hash: String,
}

#[derive(Clone, Default)]
struct CacheEntry {
    source_hash: u128,
    samples_hash: u128,
}

impl From<&SerdeCacheEntry> for CacheEntry {
    fn from(value: &SerdeCacheEntry) -> Self {
        Self {
            source_hash: u128::from_str_radix(&value.source_hash, 16).unwrap(),
            samples_hash: u128::from_str_radix(&value.samples_hash, 16).unwrap(),
        }
    }
}

impl From<&CacheEntry> for SerdeCacheEntry {
    fn from(value: &CacheEntry) -> Self {
        Self {
            source_hash: format!("{:032x}", value.source_hash),
            samples_hash: format!("{:032x}", value.samples_hash),
        }
    }
}

type SerdeCache = HashMap<PathBuf, SerdeCacheEntry>;
type Cache = HashMap<PathBuf, CacheEntry>;

struct EntryPaths {
    // input source code
    source: PathBuf,
    // input samples
    samples: Option<PathBuf>,
    // output binary
    binary: PathBuf,
    // uncompressed samples
    samples_out: Option<PathBuf>,
}

#[derive(Default)]
struct TestSample {
    input: Option<PathBuf>,
    output: Option<PathBuf>,
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

    let color = match args.color {
        clap::ColorChoice::Auto => anstyle_query::term_supports_ansi_color(),
        clap::ColorChoice::Always => true,
        clap::ColorChoice::Never => false,
    };

    if !color {
        args.color = clap::ColorChoice::Never;
    }

    logger::make_logger_from_env()
        .max_level(level)
        .print_level(true)
        .color(color)
        .install();
    args
}

fn main_() -> Result<(), AlreadyReported> {
    let args = init();

    log::trace!("{args:#?}");

    let out_path = args.root.join("out");
    let cache_path = args.root.join("out/cache.json");

    match args.command {
        Command::Run => {
            if args.targets.len() != 1 {
                bail!("The 'run' subcommand expects a single target");
            }
        }
        Command::Clean => {
            return fs::remove_dir_all(&out_path);
        }
        _ => {}
    };
    let needs_samples = matches!(args.command, Command::Test);

    if args.targets.is_empty() {
        log::info!("No targets provided");
        return Ok(());
    }

    if !out_path.exists() {
        fs::create_dir(&out_path)?;
    }

    let mut cache = load_cache(&cache_path).unwrap_or_default();

    let entry_paths = args
        .targets
        .iter()
        .flat_map(|source| prepare_target(source, &out_path, needs_samples, &args, &mut cache))
        .collect::<Vec<_>>();

    if entry_paths.len() != args.targets.len() {
        log::info!("Errors occured in previous step, exiting");
        return Err(AlreadyReported);
    }

    _ = save_cache(cache, &cache_path);

    match args.command {
        Command::Build => {}
        Command::Clean => {}
        Command::Run => {
            let entry = entry_paths.first().unwrap();
            log::info!("Running {}", entry.source.display());
            let status = std::process::Command::new(&entry.binary).status();
            check_status("child", status)?;
        }
        Command::Test => subcomand_test(&entry_paths, &args),
    }

    Ok(())
}

fn prepare_target(
    source: &Path,
    out: &Path,

    needs_samples: bool,
    args: &cli::Arguments,
    cache: &mut Cache,
) -> Result<EntryPaths, AlreadyReported> {
    if source.is_absolute() {
        report("path is absolute", source).to_result()?;
    }
    if source.extension() != Some(OsStr::from_bytes(b"c")) {
        report("must be a C source file", source).to_result()?;
    }

    let entry = cache.entry(source.to_path_buf()).or_default();
    let paths = make_paths(source, out);

    let source_hash = hash_file(&paths.source)?;
    if entry.source_hash != source_hash {
        entry.source_hash = source_hash;
        log::info!("building {}", paths.source.display());
        compile_file(&paths, args)?;
    } else {
        log::debug!("Skipping build `{}` unchanged", paths.source.display());
    }

    if needs_samples {
        if let Some(samples) = &paths.samples {
            let samples_hash = hash_file(samples)?;
            if entry.samples_hash != samples_hash {
                entry.samples_hash = samples_hash;
                extract_archive(&paths)?;
            } else {
                log::debug!("Skipping extract `{}` unchanged", samples.display());
            }
        } else {
            bail!("No sample found for `{}`", paths.source.display());
        }
    } else {
        log::debug!(
            "Skipping extract for `{}` needs_samples = false",
            paths.source.display()
        );
    }

    Ok(paths)
}

fn make_paths(source: &Path, out: &Path) -> EntryPaths {
    let samples = ["gz", "tgz", "tar.gz"]
        .iter()
        .map(|ext| source.with_extension(ext))
        .filter(|path| path.exists())
        .next();

    let output = out.join(&source);

    EntryPaths {
        source: source.to_owned(),
        binary: output.with_extension(""),
        samples_out: samples.as_ref().map(|_| output.with_extension("samples")),
        samples,
    }
}

fn load_cache(cache_path: &Path) -> fs::Result<Cache> {
    match std::fs::read_to_string(cache_path) {
        Ok(contents) => Ok(serde_json::from_str::<SerdeCache>(&contents)
            .map_err(|e| {
                log::trace!("failed to deserialize cache\n  {e}");
                AlreadyReported
            })?
            .drain()
            .map(|(k, v)| (k, CacheEntry::from(&v)))
            .collect()),
        Err(e) => {
            log::trace!("failed to load `{}`\n  {e}", cache_path.display());
            Err(AlreadyReported)
        }
    }
}

fn save_cache(cache: Cache, cache_path: &Path) -> fs::Result<()> {
    let ser = cache
        .into_iter()
        .map(|(k, v)| (k, SerdeCacheEntry::from(&v)))
        .collect::<SerdeCache>();
    let serialized = serde_json::ser::to_string_pretty(&ser).unwrap();

    let mut file = fs::options_open(
        cache_path,
        std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true),
    )?;

    fs::write_all(&mut file, cache_path, serialized.as_bytes())
}

fn subcomand_test(entry_paths: &[EntryPaths], args: &cli::Arguments) {
    let (w_sender, w_receiver) = std::sync::mpsc::channel::<(std::fs::File, ChildStdin)>();
    let join = std::thread::spawn(move || {
        while let Ok((mut file, mut stdin)) = w_receiver.recv() {
            if let Err(e) = std::io::copy(&mut file, &mut stdin) {
                _ = fs::report_custom("writing to child stdin failed", e);
            }
        }
    });

    for paths in entry_paths {
        let samples = collect_samples(paths.samples_out.as_deref().unwrap(), args);
        log::info!("Testing {}", paths.source.display());
        for (name, sample) in &samples {
            _ = test_samples(name, sample, paths, &w_sender, args);
        }
    }

    drop(w_sender);
    _ = join.join();
}

fn test_samples(
    name: &PathBuf,
    sample: &TestSample,
    paths: &EntryPaths,
    w_sender: &std::sync::mpsc::Sender<(std::fs::File, ChildStdin)>,
    args: &Arguments,
) -> Result<(), AlreadyReported> {
    let Some(input) = &sample.input else {
        bail!(
            "Sample {} is missing the corresponding input.",
            name.display()
        );
    };
    let Some(output) = &sample.output else {
        bail!(
            "Sample {} is missing the corresponding output.",
            name.display()
        );
    };

    let mut child = std::process::Command::new(&paths.binary)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .spawn()
        .map_err(|e| fs::report_io_error("luanching binary", &paths.binary, e))?;

    let stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();

    w_sender
        .send((fs::open(&input)?, stdin))
        .expect("Writing thread died!");

    let mut child_stdout = Vec::new();
    if let Err(e) = std::io::Read::read_to_end(&mut stdout, &mut child_stdout) {
        bail!("Failed to read from child stdout: {e}");
    }

    // we do not care about the exit status
    // TODO implement a timeout?
    _ = child.wait();

    let expected = fs::read(output)?;

    let display = input.display();

    // janky configurable color
    let (red, green) = match args.color == ColorChoice::Never {
        true => (Color::Default, Color::Default),
        false => (Color::LightRed, Color::LightGreen),
    };
    let err = red.paint("Err");
    let ok = green.paint("Ok");

    if child_stdout != expected {
        log::info!(" {display} {err}",);
        _ = diff_failed(output, &child_stdout, args);
    } else {
        log::info!(" {display} {ok}");
    }
    Ok(())
}

fn collect_samples(dir: &Path, args: &Arguments) -> Vec<(PathBuf, TestSample)> {
    let mut samples: HashMap<PathBuf, TestSample> = HashMap::new();
    let mut depth = 0;
    visit_files(dir, |event| {
        match event {
            TraversalEvent::EnterDirectory(dir) => {
                if depth == 0
                    && !args.sample_subdirs.is_empty()
                    && !args
                        .sample_subdirs
                        .iter()
                        .find(|s| dir.file_name().unwrap() == s.as_os_str())
                        .is_some()
                {
                    return TraversalResponse::Skip;
                }
                depth += 1;
            }
            TraversalEvent::LeaveDirectory => {
                depth -= 1;
            }
            TraversalEvent::File(file) => {
                let file = file.strip_prefix(&args.root).unwrap();
                let mut add = |name: &[u8], os: Os, input: bool| {
                    log::trace!(
                        "Found sample file {}: endings {os:?}, input {input}",
                        file.display()
                    );
                    if args.os == os {
                        let full = file.parent().unwrap().join(OsStr::from_bytes(name));
                        let entry = samples.entry(full).or_default();
                        if input {
                            if entry.input.is_some() {
                                log::error!("duplicate input file {}", file.display());
                            }
                            entry.input = Some(file.to_owned());
                        } else {
                            if entry.output.is_some() {
                                log::error!("duplicate output file {}", file.display());
                            }
                            entry.output = Some(file.to_owned());
                        }
                    } else {
                        log::trace!("skipping {}: endings do not match", file.display());
                    }
                };

                let name = file.file_name().unwrap().as_bytes();
                if let Some(name) = name.strip_suffix(b"_in.txt") {
                    add(name, Os::Unix, true);
                } else if let Some(name) = name.strip_suffix(b"_out.txt") {
                    add(name, Os::Unix, false);
                } else if let Some(name) = name.strip_suffix(b"_in_win.txt") {
                    add(name, Os::Windows, true);
                } else if let Some(name) = name.strip_suffix(b"_out_win.txt") {
                    add(name, Os::Windows, false);
                }
            }
        }
        TraversalResponse::Continue
    });

    let mut samples = samples.drain().collect::<Vec<_>>();
    samples.sort_by(|(a, _), (b, _)| a.cmp(b));
    samples
}

/// Appends a string to the original path's filename
fn append_filename(path: &Path, append: &str) -> Option<PathBuf> {
    let mut filename = path.file_name()?.as_bytes().to_vec();
    filename.extend_from_slice(append.as_bytes());
    let new = path.with_file_name(&OsStr::from_bytes(&filename));
    Some(new)
}

fn diff_failed(expected: &Path, actual: &[u8], args: &Arguments) -> fs::Result<()> {
    let save = append_filename(&expected, ".actual").unwrap();
    fs::write(&save, actual)?;

    let Some(diff) = &args.diff else {
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
            .env("EXPECTED", expected)
            .env("ACTUAL", save);

        print_args(&builder);
        _ = check_status("Diff command", builder.status());
    }

    Ok(())
}

fn check_status(
    command: &str,
    status: std::io::Result<std::process::ExitStatus>,
) -> fs::Result<()> {
    match status {
        Ok(status) => {
            if !status.success() {
                log::debug!("{command} exited with code {}", status.code().unwrap_or(-1));
            }
            Ok(())
        }
        Err(e) => {
            bail!("{command} failed: {e}");
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TraversalResponse {
    Continue,
    Skip,
    Stop,
}

enum TraversalEvent<'a> {
    EnterDirectory(&'a Path),
    LeaveDirectory,
    File(&'a Path),
}

fn visit_files(dir: &Path, mut fun: impl FnMut(TraversalEvent) -> TraversalResponse) {
    visit_files_impl(dir, &mut fun);
}

fn visit_files_impl(
    dir: &Path,
    fun: &mut dyn FnMut(TraversalEvent) -> TraversalResponse,
) -> TraversalResponse {
    let iter = fs::read_dir(dir).unwrap();
    for element in iter {
        let entry = match element {
            Ok(ok) => ok,
            Err(e) => {
                log::trace!("error listing {dir:?}: {e}");
                continue;
            }
        };
        let path = entry.path();
        match entry.metadata() {
            Ok(ty) => {
                if ty.is_symlink() && ty.is_dir() {
                    // do not follow symlinks to prevent infinite loops
                    log::trace!("{path:?} is a directory symlink, skipping")
                } else if ty.is_dir() {
                    match fun(TraversalEvent::EnterDirectory(&path)) {
                        TraversalResponse::Continue => {
                            visit_files_impl(&path, fun);
                            if fun(TraversalEvent::LeaveDirectory) == TraversalResponse::Stop {
                                return TraversalResponse::Stop;
                            }
                        }
                        TraversalResponse::Skip => continue,
                        TraversalResponse::Stop => return TraversalResponse::Stop,
                    }
                } else if ty.is_file() {
                    if fun(TraversalEvent::File(&path)) == TraversalResponse::Stop {
                        return TraversalResponse::Stop;
                    }
                }
            }
            Err(e) => {
                fs::report_io_error("DirEntry::metadata", &path, e);
            }
        }
    }
    TraversalResponse::Continue
}

fn extract_archive(paths: &EntryPaths) -> fs::Result<()> {
    let Some(path) = &paths.samples else {
        return Ok(());
    };
    let parent = paths.samples_out.as_ref().unwrap();

    fs::create_dir_all(parent)?;
    let mut builder = std::process::Command::new("tar");
    builder.arg("-xzf").arg(path).arg("-C").arg(parent);

    print_args(&builder);
    check_status("tar", builder.status())
}

fn compile_file(paths: &EntryPaths, args: &cli::Arguments) -> fs::Result<()> {
    _ = fs::create_dir_all(paths.binary.parent().unwrap());
    let mut builder = std::process::Command::new("g++");
    if args.override_compiler_args.is_none() {
        builder.args(&["-std=c++11", "-Wall", "-pedantic"]);
    }
    builder
        .args(
            args.compiler_args
                .as_deref()
                .unwrap_or("")
                .split_ascii_whitespace(),
        )
        .arg("-o")
        .arg(&paths.binary)
        .arg(&paths.source);

    print_args(&builder);
    check_status("g++", builder.status())
}

fn hash_file(path: &Path) -> fs::Result<u128> {
    let input = fs::open(path)?;
    let mut hasher = blake3::Hasher::new();

    hasher
        .update_reader(&input)
        .map_err(|e| report_io_error("failed to update_reader", path, e))?;

    let mut buf = [0; 16];
    hasher.finalize_xof().fill(&mut buf);
    Ok(u128::from_le_bytes(buf))
}

fn print_args(builder: &std::process::Command) {
    use std::io::Write as _;
    let mut buf = Vec::new();

    _ = buf.write_all(builder.get_program().as_bytes());
    for a in builder.get_args() {
        _ = buf.write_all(b" ");
        _ = buf.write_all(a.as_bytes());
    }

    log::trace!(
        "Running command `{}`",
        OsStr::from_bytes(buf.as_slice()).to_string_lossy()
    )
}
