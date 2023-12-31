#![allow(dead_code)]

use std::{ffi::OsString, fmt::Display, fs::OpenOptions, io::Read, path::Path};

use crate::bail;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AlreadyReported;

impl AlreadyReported {
    pub fn to_result(self) -> std::result::Result<(), AlreadyReported> {
        Err(self)
    }
}

impl Display for AlreadyReported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self, f)
    }
}

impl std::error::Error for AlreadyReported {}

macro_rules! report {
    ($result:expr, $message:literal, $path:expr) => {
        match $result {
            Ok(ok) => Ok(ok),
            Err(e) => {
                log::error!("{} `{}`\n  {e}", $message, $path.display());
                Err(AlreadyReported)
            }
        }
    };
}

pub type Result<T> = std::result::Result<T, AlreadyReported>;

#[track_caller]
pub fn open(path: &Path) -> Result<std::fs::File> {
    let res = std::fs::File::open(path);
    report!(res, "failed to open file", path)
}
#[track_caller]
pub fn write(path: &Path, contents: &[u8]) -> Result<()> {
    let res = std::fs::write(path, contents);
    report!(res, "failed to write file", path)
}
#[track_caller]
pub fn remove_dir_all(path: &Path) -> Result<()> {
    let res = std::fs::remove_dir_all(path);
    report!(res, "failed to remove_dir_all", path)
}
#[track_caller]
pub fn remove_file(path: &Path) -> Result<()> {
    let res = std::fs::remove_file(path);
    report!(res, "failed to remove_file", path)
}
#[track_caller]
pub fn read(path: &Path) -> Result<Vec<u8>> {
    let res = std::fs::read(path);
    report!(res, "failed to read file", path)
}
#[track_caller]
pub fn read_into(path: &Path, buf: &mut Vec<u8>) -> Result<usize> {
    let mut file = options_open(path, OpenOptions::new().read(true))?;
    let res = file.read_to_end(buf);

    report!(res, "failed to read file", path)
}
#[track_caller]
pub fn read_to_string(path: &Path) -> Result<String> {
    let res = std::fs::read_to_string(path);
    report!(res, "failed to read to string", path)
}
#[track_caller]
pub fn read_dir(path: &Path) -> Result<std::fs::ReadDir> {
    let res = std::fs::read_dir(path);
    report!(res, "failed to read_dir", path)
}
#[track_caller]
pub fn create_dir_all(path: &Path) -> Result<()> {
    let res = std::fs::create_dir_all(path);
    report!(res, "failed to create_dir_all", path)
}
#[track_caller]
pub fn create_dir(path: &Path) -> Result<()> {
    let res = std::fs::create_dir(path);
    report!(res, "failed to create_dir", path)
}
#[track_caller]
pub fn options_open(path: &Path, options: &std::fs::OpenOptions) -> Result<std::fs::File> {
    let res = options.open(path);
    report!(res, "failed to open file", path)
}

pub fn report_custom(message: impl Display, error: impl Display) -> AlreadyReported {
    log::error!("{message}\n  {error}");
    AlreadyReported
}

pub fn report_io_error(
    message: impl Display,
    path: &Path,
    error: std::io::Error,
) -> AlreadyReported {
    log::error!("{message} `{}`\n  {error}", path.display());
    AlreadyReported
}

pub fn report(message: impl Display, path: &Path) -> AlreadyReported {
    log::error!("{} `{}`", message, path.display());
    AlreadyReported
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TraversalResponse {
    Continue,
    Skip,
    Stop,
}

pub enum TraversalEvent<'a> {
    EnterDirectory(&'a Path),
    LeaveDirectory,
    File(&'a Path),
}

pub fn visit_files(dir: &Path, mut fun: impl FnMut(TraversalEvent) -> TraversalResponse) {
    visit_files_impl(dir, &mut fun);
}

fn visit_files_impl(
    dir: &Path,
    fun: &mut dyn FnMut(TraversalEvent) -> TraversalResponse,
) -> TraversalResponse {
    let iter = read_dir(dir).unwrap();
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
                report_io_error("DirEntry::metadata", &path, e);
            }
        }
    }
    TraversalResponse::Continue
}

pub fn check_status(
    command: &str,
    status: std::io::Result<std::process::ExitStatus>,
) -> Result<()> {
    match status {
        Ok(status) => {
            if !status.success() {
                log::debug!("{command} exited with code {}", status.code().unwrap_or(-1));
                return Err(AlreadyReported);
            }
            Ok(())
        }
        Err(e) => {
            bail!("{command} failed: {e}");
        }
    }
}

pub fn print_args(builder: &std::process::Command) {
    let mut buf = OsString::new();

    buf.push(builder.get_program());
    for a in builder.get_args() {
        buf.push(" ");
        buf.push(a);
    }

    log::trace!("Running command `{}`", buf.to_string_lossy());
}
