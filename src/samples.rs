use std::{
    collections::HashMap,
    ffi::OsString,
    io::Write,
    path::{Path, PathBuf},
};

use bstr::ByteSlice;

use crate::{
    cli::{Arguments, Os},
    fs::{self, check_status, print_args, visit_files, TraversalEvent, TraversalResponse},
};

pub struct Sample<'a> {
    pub header: &'a [u8],
    pub body: &'a [u8],
}

pub struct SampleIterator<'a> {
    inner: bstr::Split<'a, 'a>,
}

impl<'a> SampleIterator<'a> {
    pub fn new(str: &'a [u8]) -> Option<SampleIterator<'a>> {
        // can't use bstr::split because we need the splitting slice to include the newline
        let newline_index = str.bytes().position(|c| c == b'\n')?;
        let (separator, remaining) = str.split_at(newline_index + 1);
        let iterator = SampleIterator {
            inner: remaining.split_str(separator),
        };
        Some(iterator)
    }
}

impl<'a> Iterator for SampleIterator<'a> {
    type Item = Sample<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        let header = self.inner.next()?.trim_end();
        let body = self.inner.next()?;
        Some(Sample { header, body })
    }
}

#[derive(Default)]
struct SampleFiles {
    input: Option<PathBuf>,
    output: Option<PathBuf>,
}

fn collect_sample_files(
    dir: &Path,
    target_os: Os,
    sample_subdirs: &[OsString],
) -> Vec<(String, SampleFiles)> {
    let mut samples: HashMap<String, SampleFiles> = HashMap::new();
    let mut depth = 0;
    visit_files(dir, |event| {
        match event {
            TraversalEvent::EnterDirectory(dir) => {
                if depth == 0
                    && !sample_subdirs.is_empty()
                    && !sample_subdirs
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
                let relative = file.strip_prefix(dir).unwrap();
                let mut add = |name: &str, os: Os, input: bool| {
                    log::trace!(
                        "Found sample file {}: endings {os:?}, input {input}",
                        relative.display()
                    );
                    if target_os == os {
                        let raw_key = relative.parent().unwrap().join(name);
                        let key = raw_key.to_str().unwrap().to_owned().replace('/', "_");

                        let entry = samples.entry(key).or_default();
                        if input {
                            if entry.input.is_some() {
                                log::error!("duplicate input file {}", relative.display());
                            }
                            entry.input = Some(file.to_owned());
                        } else {
                            if entry.output.is_some() {
                                log::error!("duplicate output file {}", relative.display());
                            }
                            entry.output = Some(file.to_owned());
                        }
                    } else {
                        log::trace!("skipping {}: line endings do not match", relative.display());
                    }
                };

                let name = file.file_name().unwrap().to_str().unwrap();
                if let Some(name) = name.strip_suffix("_in.txt") {
                    add(name, Os::Unix, true);
                } else if let Some(name) = name.strip_suffix("_out.txt") {
                    add(name, Os::Unix, false);
                } else if let Some(name) = name.strip_suffix("_in_win.txt") {
                    add(name, Os::Windows, true);
                } else if let Some(name) = name.strip_suffix("_out_win.txt") {
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

fn make_samples_string(collected: &[(String, SampleFiles)]) -> fs::Result<Vec<u8>> {
    let mut buf = Vec::new();
    for (file, sample) in collected {
        let input = sample.input.as_ref().unwrap();
        let output = sample.output.as_ref().unwrap();

        _ = writeln!(buf, "---");
        _ = writeln!(buf, "{file} in");
        _ = writeln!(buf, "---");
        fs::read_into(input, &mut buf)?;
        if !buf.ends_with(b"\n") {
            buf.push(b'\n');
        }

        _ = writeln!(buf, "---");
        _ = writeln!(buf, "{file} out");
        _ = writeln!(buf, "---");
        fs::read_into(&output, &mut buf)?;
        if !buf.ends_with(b"\n") {
            buf.push(b'\n');
        }
    }

    Ok(buf)
}

fn extract_archive(archive: &Path, extract_dir: &Path) -> fs::Result<()> {
    let mut builder = std::process::Command::new("tar");
    builder.arg("-xzf").arg(archive).arg("-C").arg(extract_dir);

    print_args(&builder);
    check_status("tar", builder.status())
}

pub fn subcommand_convert(
    out_dir: &Path,
    archive: &Path,
    converted_file: &Path,
    args: &Arguments,
    sample_subdirs: &[OsString],
) -> fs::Result<()> {
    let extract_dir = out_dir.join("extract");
    if extract_dir.exists() {
        _ = fs::remove_dir_all(&extract_dir);
    }
    fs::create_dir_all(&extract_dir)?;
    extract_archive(archive, &extract_dir)?;

    let collected = collect_sample_files(&extract_dir, args.os, sample_subdirs);
    if collected.is_empty() {
        log::info!("archive contains no sample files");
        return Ok(());
    }

    let contents = make_samples_string(&collected)?;

    log::trace!("writing converted file to `{}`", converted_file.display());
    fs::write(converted_file, &contents)
}
