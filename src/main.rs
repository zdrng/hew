use std::{env, error::Error, ffi::OsString, io, path::Path, process::ExitCode};

use hew::{config, service};

const USAGE: &str = concat!(
    "usage: hew [--flush]\n",
    "\n",
    "Formats JSON log lines from stdin onto stdout; other lines pass through.\n",
    "\n",
    "  --flush     Flush stdout after every line even when it is not a\n",
    "              terminal. Needed when piping into a pager, as in\n",
    "              `… | hew --flush | less -R +F`; without it output is\n",
    "              buffered in 64 KiB blocks and a slow producer looks frozen.\n",
    "  -h, --help  Show this message.\n",
);

#[derive(Debug)]
struct Options {
    force_flush: bool,
}

fn main() -> ExitCode {
    let options = match parse_args(env::args_os().skip(1)) {
        Ok(Some(options)) => options,
        Ok(None) => {
            print_usage();
            return ExitCode::SUCCESS;
        }
        Err(arg) => {
            report_unknown(&arg);
            return ExitCode::FAILURE;
        }
    };

    let config = match config::load(Path::new(config::DEFAULT_PATH)) {
        Ok(config) => config,
        Err(err) => {
            report(&err);
            return ExitCode::FAILURE;
        }
    };

    match service::run(&config, options.force_flush) {
        Ok(()) => ExitCode::SUCCESS,
        // Rust ignores SIGPIPE, so a reader closing early (`hew | head`)
        // surfaces here and is a success, not a failure.
        Err(err) if err.kind() == io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(err) => {
            report(&err);
            ExitCode::FAILURE
        }
    }
}

/// `Ok(None)` means help was requested; `Err` carries the first unknown
/// argument.
fn parse_args<I>(args: I) -> Result<Option<Options>, OsString>
where
    I: IntoIterator<Item = OsString>,
{
    let mut options = Options { force_flush: false };

    for arg in args {
        match arg.to_str() {
            Some("--flush") => options.force_flush = true,
            Some("-h" | "--help") => return Ok(None),
            _ => return Err(arg),
        }
    }

    Ok(Some(options))
}

#[expect(
    clippy::print_stdout,
    reason = "`--help` is the output of the run, not part of the data stream; nothing is being filtered when it is asked for"
)]
fn print_usage() {
    print!("{USAGE}");
}

#[expect(
    clippy::print_stderr,
    reason = "the sole diagnostic channel of a CLI filter; stdout carries the data stream and must stay clean"
)]
fn report_unknown(arg: &OsString) {
    eprintln!("hew: unrecognised argument: {}", arg.to_string_lossy());
    eprint!("{USAGE}");
}

#[expect(
    clippy::print_stderr,
    reason = "the sole diagnostic channel of a CLI filter; stdout carries the data stream and must stay clean"
)]
fn report(err: &dyn Error) {
    eprint!("hew: {err}");
    let mut cause = err.source();
    while let Some(inner) = cause {
        eprint!(": {inner}");
        cause = inner.source();
    }
    eprintln!();
}
