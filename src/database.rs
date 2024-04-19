use crate::cli::BuildOpts;
use crate::fs::{self, report, report_io_error, AlreadyReported};
use crate::{check_status, print_args};
use std::{
    cell::Cell,
    collections::HashMap,
    hash::{Hash, Hasher},
    os::unix::prelude::OsStrExt,
    path::{Path, PathBuf},
    rc::Rc,
};

#[derive(Clone)]
pub struct CacheEntry {
    source_hash: Cell<u128>,
    // input source code
    pub source: PathBuf,
    // output binary
    pub binary: PathBuf,
    // uncompressed samples
    pub samples_out: Option<PathBuf>,
}

impl CacheEntry {
    pub fn from_serialized(source_file: &Path, source_hash: u128, out_dir: &Path) -> CacheEntry {
        let samples_out = source_file.with_extension("samples");
        Self {
            source_hash: Cell::new(source_hash),
            source: source_file.to_owned(),
            binary: out_dir.join(&source_file).with_extension(""),
            samples_out: samples_out.is_file().then_some(samples_out),
        }
    }
}

pub struct Database {
    cache_file: PathBuf,
    out_dir: PathBuf,
    cache: HashMap<PathBuf, Rc<CacheEntry>>,
}

impl Database {
    pub fn new(cache_file: PathBuf, out_dir: PathBuf) -> fs::Result<Database> {
        let parsed = if cache_file.exists() {
            let loaded = fs::read_to_string(&cache_file)?;
            let deserialized =
                serde_json::from_str::<HashMap<PathBuf, String>>(&loaded).map_err(|e| {
                    log::error!("failed to deserialize cache\n  {e}");
                    AlreadyReported
                })?;
            deserialized
                .into_iter()
                .map(|(k, v)| {
                    let hash = u128::from_str_radix(&v, 16).unwrap();
                    let entry = CacheEntry::from_serialized(&k, hash, &out_dir);
                    (k, Rc::new(entry))
                })
                .collect()
        } else {
            HashMap::new()
        };

        Ok(Database {
            cache_file,
            out_dir,
            cache: parsed,
        })
    }
    pub fn new_empty(cache_file: PathBuf, out_dir: PathBuf) -> Database {
        Database {
            cache_file,
            out_dir,
            cache: HashMap::new(),
        }
    }
    pub fn build_file(
        &mut self,
        source_file: &Path,
        args: &BuildOpts,
    ) -> fs::Result<Rc<CacheEntry>> {
        if source_file.is_absolute() {
            report("path is absolute", source_file).to_result()?;
        }
        let extension = source_file.extension().unwrap_or_default().as_bytes();
        if !matches!(extension, b"c" | b"cpp") {
            report("must be a C/C++ source file", source_file).to_result()?;
        }

        let entry = self
            .cache
            .entry(source_file.to_owned())
            .or_insert_with(|| Rc::new(CacheEntry::from_serialized(source_file, 0, &self.out_dir)));

        let mut source_hash = hash_file(&entry.source)?;
        append_hash(&mut source_hash, &args.defines);
        append_hash(&mut source_hash, &args.compiler_args);
        append_hash(&mut source_hash, &args.no_default_args);

        if entry.source_hash.get() != source_hash {
            entry.source_hash.set(source_hash);
            log::info!("building {}", entry.source.display());
            compile_file(&entry, args)?;
        } else {
            log::debug!("Skipping build `{}` unchanged", entry.source.display());
        }

        Ok(entry.clone())
    }
    pub fn save_to_file(&self) -> fs::Result<()> {
        let raw = self
            .cache
            .iter()
            .map(|(k, v)| {
                let hash = format!("{:032x}", v.source_hash.get());
                (k.clone(), hash)
            })
            .collect::<HashMap<PathBuf, String>>();
        let serialized = serde_json::ser::to_string_pretty(&raw).unwrap();

        fs::write(&self.cache_file, serialized.as_bytes())
    }
}

fn compile_file(paths: &CacheEntry, args: &BuildOpts) -> fs::Result<()> {
    _ = fs::create_dir_all(paths.binary.parent().unwrap());
    if paths.binary.exists() {
        _ = fs::remove_file(&paths.binary);
    }
    let mut builder = std::process::Command::new("g++");
    if args.no_default_args {
        builder.args(&["-std=c++11", "-Wall", "-pedantic"]);
    }
    for define in &args.defines {
        builder.arg("-D");
        builder.arg(define);
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

fn append_hash<T: Hash>(prev_hash: &mut u128, value: &T) {
    let hash = {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    } as u128;
    let extended = hash | (hash << 32);
    *prev_hash ^= extended;
}
