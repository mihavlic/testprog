mod cli;

use crate::cli::parse_args;
use std::{
    collections::HashMap,
    ffi::OsStr,
    fs::File,
    io::Write,
    os::unix::prelude::OsStrExt,
    path::{Path, PathBuf},
    process::ChildStdin,
};

#[macro_export]
macro_rules! bail {
    ($($a:tt)*) => {{
        eprintln!($($a)*);
        std::process::exit(1);
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

type SerdeDatabase = HashMap<PathBuf, SerdeCacheEntry>;
type Database = HashMap<PathBuf, CacheEntry>;

struct EntryPaths {
    // input source code
    source: PathBuf,
    // input samples
    samples: PathBuf,
    // output binary
    binary: PathBuf,
    // uncompressed samples
    samples_out: PathBuf,
}

#[derive(Default)]
struct SampleInOut {
    input: Option<PathBuf>,
    output: Option<PathBuf>,
}

fn main() {
    let args = match parse_args() {
        Ok(v) => v,
        Err(e) => {
            bail!("Error: {}", e);
        }
    };
    let out = args.root.join("out");

    let (needs_build, needs_samples) = match args.command {
        cli::Command::Build => (true, false),
        cli::Command::Run => {
            if args.targets.len() != 1 {
                bail!("The 'run' subcomand expects a single target");
            }
            (true, false)
        }
        cli::Command::Test => (true, true),
        cli::Command::Clean => {
            _ = std::fs::remove_dir_all(&out);
            std::process::exit(0);
        }
    };

    if args.targets.is_empty() {
        bail!("No targets provided");
    }

    if !out.exists() {
        if std::fs::create_dir(&out).is_err() {
            bail!("Failed to create 'out' directory. ({out:?})");
        }
    }

    let cache_path = args.root.join("out/cache.ron");
    let mut cache = if cache_path.exists() {
        let contents = std::fs::read_to_string(&cache_path).expect("Couldn't read database file");
        ron::from_str::<SerdeDatabase>(&contents)
            .expect("Failed to deserialize database")
            .drain()
            .map(|(k, v)| (k, CacheEntry::from(&v)))
            .collect()
    } else {
        Database::new()
    };

    let mut err = false;
    let mut entry_paths = Vec::new();
    for source in &args.targets {
        {
            let mut skip = false;
            if source.is_absolute() {
                skip = true;
                eprintln!("{source:?} path is absolute");
            }
            if !needs_build && !source.exists() {
                skip = true;
                eprintln!("{source:?} doesn't exist");
            }
            if source.extension() != Some(OsStr::from_bytes(b"c")) {
                skip = true;
                eprintln!("{source:?} must be a C source file");
            }

            if skip {
                err = true;
                continue;
            }
        }

        let entry = cache.entry(source.clone()).or_default();

        let binary = out.join(&source).with_extension("");
        let paths = EntryPaths {
            source: source.to_owned(),
            samples: source.with_extension("gz"),
            samples_out: binary.with_extension("samples"),
            binary,
        };

        if !needs_build {
            eprintln!("Skipping build: needs_build = false");
        } else if paths.source.exists() {
            let source_hash = hash_file(&paths.source).unwrap();
            if entry.source_hash != source_hash {
                entry.source_hash = source_hash;
                compile_file(
                    &paths,
                    &mut err,
                    args.compiler_args.as_deref(),
                    !args.no_default,
                );
            } else {
                eprintln!("Skipping build: {:?} unchanged", &paths.source);
            }
        } else {
            eprintln!("needs_build = true, but {:?} doesn't exist", paths.source);
            err = true;
            continue;
        }

        if !needs_samples {
            eprintln!("Skipping extraction: needs_samples = false");
        } else if paths.samples.exists() {
            let samples_hash = hash_file(&paths.samples).unwrap();
            if entry.samples_hash != samples_hash {
                entry.samples_hash = samples_hash;
                extract_archive(&paths, &mut err);
            } else {
                eprintln!("Skipping extraction: {:?} unchanged", paths.samples);
            }
        } else {
            eprintln!(
                "needs_samples = true, but {:?} doesn't exist",
                paths.samples
            );
            err = true;
            continue;
        }

        entry_paths.push(paths);
    }

    if err {
        bail!("Errors occured in previous steps, exiting");
    }

    save_db(cache, &cache_path);

    match args.command {
        cli::Command::Build => {}
        cli::Command::Clean => {}
        cli::Command::Run => {
            let entry = entry_paths.pop().unwrap();
            let status = std::process::Command::new(&entry.binary).status();
            check_status("child", status);
        }
        cli::Command::Test => subcomand_test(&args, &entry_paths),
    }
}

fn save_db(cache: Database, cache_path: &Path) {
    let ser = cache
        .into_iter()
        .map(|(k, v)| (k, SerdeCacheEntry::from(&v)))
        .collect::<SerdeDatabase>();
    let serialized = ron::ser::to_string_pretty(&ser, ron::ser::PrettyConfig::default()).unwrap();

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&cache_path)
        .unwrap();

    file.write_all(serialized.as_bytes()).unwrap();
}

fn subcomand_test(args: &cli::AppArgs, entry_paths: &[EntryPaths]) {
    let (w_sender, w_receiver) = std::sync::mpsc::channel::<(File, ChildStdin)>();
    let join = std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        while let Ok((mut file, mut stdin)) = w_receiver.recv() {
            if let Err(e) = redirect_bytes(&mut file, &mut stdin, &mut buf) {
                eprintln!("Writing to child stdin failed: {e}");
            }
        }
    });

    let diff = args.diff.as_deref().unwrap_or("").trim();

    let mut samples: HashMap<PathBuf, SampleInOut> = HashMap::new();
    for paths in entry_paths {
        visit_files(&paths.samples_out, |file| {
            let name = &file.file_name().unwrap().as_bytes();
            if let Some(name) = name.strip_suffix(b"_in.txt") {
                let full = file.parent().unwrap().join(OsStr::from_bytes(name));
                let entry = samples.entry(full).or_default();
                assert!(entry.input.is_none());
                eprintln!("Found input {file:?}");
                entry.input = Some(file);
            } else if let Some(name) = name.strip_suffix(b"_out.txt") {
                let full = file.parent().unwrap().join(OsStr::from_bytes(name));
                let entry = samples.entry(full).or_default();
                assert!(entry.output.is_none());
                eprintln!("Found output {file:?}");
                entry.output = Some(file);
            }
        });

        let mut samples = samples.drain().collect::<Vec<_>>();
        // we sort the samples because it looks nice
        samples.sort_by(|(a, _), (b, _)| a.cmp(b));

        for (name, sample) in &samples {
            let Some(input) = &sample.input else {
                eprintln!("Sample {name:?} is missing the corresponding input.");
                continue;
            };
            let Some(output) = &sample.output else {
                eprintln!("Sample {name:?} is missing the corresponding output.");
                continue;
            };

            let child = std::process::Command::new(&paths.binary)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::inherit())
                .spawn()
                .unwrap();

            let stdin = child.stdin.unwrap();
            let mut stdout = child.stdout.unwrap();

            let Some(input) = open_file(&input) else {
                continue;
            };

            w_sender.send((input, stdin)).expect("Writing thread died");

            let mut child_stdout = String::new();
            if let Err(e) = std::io::Read::read_to_string(&mut stdout, &mut child_stdout) {
                eprintln!("Failed to read from child stdout: {e}");
                continue;
            }

            let expected = match std::fs::read_to_string(output) {
                Ok(ok) => ok,
                Err(e) => {
                    eprintln!("Failed to read {output:?}: {e}");
                    continue;
                }
            };

            if child_stdout != expected {
                ask_diff(name, output, child_stdout, diff);
            } else {
                eprintln!("{name:?} Ok");
            }
        }
    }

    drop(w_sender);
    _ = join.join();
}

fn ask_diff(name: &PathBuf, output: &PathBuf, child_stdout: String, diff_command: &str) {
    _ = std::fs::create_dir_all(output.parent().unwrap());
    let save = output.with_extension("actual.txt");
    eprintln!("Sample {name:?} doesn't match (saved to {save:?})");

    if let Err(e) = std::fs::write(&save, &child_stdout) {
        eprintln!("Failed to save {save:?}: {e}");
        return;
    }

    if diff_command.is_empty() {
        return;
    }

    let mut line = String::new();
    let should_diff = loop {
        eprintln!("View diff? [Y/n]");
        std::io::stdin().read_line(&mut line).unwrap();
        match line.trim_start().chars().next() {
            Some('Y' | 'y') | None => break true,
            Some('N' | 'n') => break false,
            _ => {}
        }
    };

    if should_diff {
        let mut builder = std::process::Command::new(diff_command);
        builder.arg(output).arg(&save);

        print_args(&builder);

        check_status("Diff command", builder.status());
    }
}

fn check_status(command: &str, status: std::io::Result<std::process::ExitStatus>) -> bool {
    let status = status.unwrap();
    if !status.success() {
        eprintln!("{command} exited with code {}", status.code().unwrap_or(-1));
        true
    } else {
        false
    }
}

fn open_file(path: &Path) -> Option<File> {
    match File::open(path) {
        Ok(ok) => Some(ok),
        Err(e) => {
            eprintln!("Failed to open {path:?}: {e}");
            None
        }
    }
}

fn redirect_bytes(
    src: &mut impl std::io::Read,
    stdin: &mut impl std::io::Write,
    buf: &mut [u8],
) -> std::io::Result<u64> {
    assert!(buf.len() > 0);
    let mut total = 0;
    loop {
        match src.read(buf) {
            Ok(0) => return Ok(total),
            Ok(n) => {
                stdin.write_all(&buf[..n])?;
                total += n as u64;
            }
            // see test_update_reader_interrupted
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
}

fn visit_files(dir: &Path, mut fun: impl FnMut(PathBuf)) {
    visit_files_impl(dir, &mut fun);
}

fn visit_files_impl(dir: &Path, fun: &mut dyn FnMut(PathBuf)) {
    let iter = std::fs::read_dir(dir).unwrap();
    for element in iter {
        let entry = match element {
            Ok(ok) => ok,
            Err(e) => {
                eprintln!("Error listing {dir:?}: {e}");
                continue;
            }
        };
        if let Ok(ty) = entry.metadata() {
            let path = entry.path();
            if ty.is_symlink() && ty.is_dir() {
                // do not follow symlinks to prevent infinite loops
                eprintln!("{path:?} is a directory symlink, skipping")
            } else if ty.is_dir() {
                visit_files_impl(&path, fun);
            } else if ty.is_file() {
                fun(path);
            }
        } else {
            eprintln!("Aaaaaaaaaaaa");
        }
    }
}

fn compile_file(
    paths: &EntryPaths,
    err: &mut bool,
    compiler_args: Option<&str>,
    default_options: bool,
) {
    _ = std::fs::create_dir_all(paths.binary.parent().unwrap());
    let mut builder = std::process::Command::new("g++");
    if default_options {
        builder.args(&["-std=c++11", "-Wall", "-pedantic"]);
    }
    builder
        .args(compiler_args.unwrap_or("").split_ascii_whitespace())
        .arg("-o")
        .arg(&paths.binary)
        .arg(&paths.source);

    print_args(&builder);

    if check_status("g++", builder.status()) {
        *err = true;
    }
}

fn extract_archive(paths: &EntryPaths, err: &mut bool) {
    _ = std::fs::create_dir_all(&paths.samples_out);
    let mut builder = std::process::Command::new("tar");
    builder
        .arg("-xzf")
        .arg(&paths.samples)
        .arg("-C")
        .arg(&paths.samples_out);

    print_args(&builder);

    if check_status("tar", builder.status()) {
        *err = true;
    }
}

fn hash_file(path: &PathBuf) -> std::io::Result<u128> {
    let input = File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    hasher.update_reader(&input)?;
    let mut buf = [0; 16];
    hasher.finalize_xof().fill(&mut buf);

    Ok(u128::from_le_bytes(buf))
}

fn print_args(builder: &std::process::Command) {
    let mut stderr = std::io::stderr();

    _ = stderr.write_all(builder.get_program().as_bytes());
    for a in builder.get_args() {
        _ = stderr.write_all(b" ");
        _ = stderr.write_all(a.as_bytes());
    }
    _ = stderr.write_all(b"\n");
}
