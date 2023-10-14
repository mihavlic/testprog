use std::{fmt::Display, io::Write, path::Path};

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
                log::error!("{} `{}`\n    {e}", $message, $path.display());
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
pub fn write_all(file: &mut std::fs::File, path: &Path, contents: &[u8]) -> Result<()> {
    let res = file.write_all(contents);
    report!(res, "failed to write_all", path)
}
#[track_caller]
pub fn remove_dir_all(path: &Path) -> Result<()> {
    let res = std::fs::remove_dir_all(path);
    report!(res, "failed to remove_dir_all", path)
}
#[track_caller]
pub fn read(path: &Path) -> Result<Vec<u8>> {
    let res = std::fs::read(path);
    report!(res, "failed to read file", path)
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
    log::error!("{message}\n    {error}");
    AlreadyReported
}

pub fn report_io_error(
    message: impl Display,
    path: &Path,
    error: std::io::Error,
) -> AlreadyReported {
    log::error!("{message} `{}`\n    {error}", path.display());
    AlreadyReported
}

pub fn report(message: impl Display, path: &Path) -> AlreadyReported {
    log::error!("{} `{}`", message, path.display());
    AlreadyReported
}
