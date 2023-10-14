use log::{Metadata, Record};

#[derive(Clone, Debug)]
pub struct CustomLogger {
    pub max_level: log::LevelFilter,
    pub color: bool,
    pub print_level: bool,
    pub print_file: bool,
}

impl CustomLogger {
    pub fn with_level(&mut self, level: log::LevelFilter) -> &mut CustomLogger {
        self.max_level = self.max_level.max(level);
        self
    }
    pub fn install(&self) {
        log::set_boxed_logger(Box::new(self.clone())).expect("Called set_logger twice");
        log::set_max_level(self.max_level);
    }
}

impl log::Log for CustomLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= self.max_level
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }

        let level = record.level();
        let color = match level {
            log::Level::Error => nu_ansi_term::Color::Red,
            log::Level::Warn => nu_ansi_term::Color::Yellow,
            log::Level::Debug => nu_ansi_term::Color::Blue,
            log::Level::Info => nu_ansi_term::Color::Green,
            log::Level::Trace => nu_ansi_term::Color::Magenta,
        };
        let (pre, post) = (color.prefix(), color.suffix());

        if self.print_level {
            if self.color {
                eprint!("{pre}{level:5?}{post} ");
            } else {
                eprint!("{level:5?} ")
            }
        }

        if self.print_file {
            if let (Some(file), Some(line)) = (record.file(), record.line()) {
                if self.color {
                    let gray = nu_ansi_term::Color::LightGray;
                    eprint!("{}{file}:{line}{} ", gray.prefix(), gray.suffix());
                } else {
                    eprint!("{file}:{line} ")
                }
            }
        }

        eprintln!("{}", record.args());
    }
    fn flush(&self) {}
}

pub fn make_logger_from_env() -> CustomLogger {
    use std::io::IsTerminal as _;
    let env = std::env::var("RUST_LOG").unwrap_or(String::new());

    let mut color = std::io::stderr().is_terminal();
    let mut print_level = false;
    let mut print_file = false;
    let mut max_level = log::LevelFilter::Error;
    for str in env.split(',') {
        match str.trim() {
            "error" => max_level = max_level.max(log::LevelFilter::Error),
            "warn" => max_level = max_level.max(log::LevelFilter::Warn),
            "debug" => max_level = max_level.max(log::LevelFilter::Debug),
            "info" => max_level = max_level.max(log::LevelFilter::Info),
            "trace" => max_level = max_level.max(log::LevelFilter::Trace),
            "color" => color = true,
            "level" => print_level = true,
            "file" => print_file = true,
            _ => {}
        }
    }

    CustomLogger {
        max_level,
        color,
        print_level,
        print_file,
    }
}
