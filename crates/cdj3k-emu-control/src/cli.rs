//! `cdj3k-emu ctl`: drive a slot's control port from a shell or a script.
//!
//! ```text
//! cdj3k-emu ctl [--slot N] [--host HOST] <request>     send one request, print the reply
//! cdj3k-emu ctl lcd main|jog <out.png>                  save a screen
//! cdj3k-emu ctl shot <out.png>                          save the window
//! cdj3k-emu ctl match main|jog <ref.png> [diff options] [--timeout MS]
//!                                                       wait until a screen matches ref.png
//! cdj3k-emu ctl diff <ref.png> <actual.png> [diff options]
//! cdj3k-emu ctl changes main|jog <region> [--timeout MS]  wait until a region changes
//! cdj3k-emu ctl still main|jog <region> [--for MS]       fail if a region changes
//! cdj3k-emu ctl sh <command>                            run a guest command, print its output
//! cdj3k-emu ctl run <file>                              run one command per line
//! cdj3k-emu ctl e2e [options] <file|dir>...             run scenarios and report
//! cdj3k-emu ctl watch <topic> [filters] [--for MS] [--count N]
//!                                                       print events
//! ```
//!
//! The diff options are `--threshold N` (the largest channel difference that
//! counts as equal), `--max N` (the changed pixels allowed), `--mask
//! x0,y0,x1,y1` (a region to leave out, in fractions; repeatable), `--region
//! x0,y0,x1,y1` (compare only inside it) and `--out diff.png` (a picture of
//! the difference). A reference such as `ref.png` also takes the regions
//! listed in `ref.mask`.
//!
//! Each command prints one JSON line, and `watch` one line per event. The
//! exit status is 0 on success, 1 on a failure and 2 on a usage error.

use std::path::Path;

use crate::steps::{exec, script_words, Failure, Session};

const USAGE: &str = "\
usage: cdj3k-emu ctl [--slot N] [--host HOST] <command>
  <request>                         send a request, such as `press PLAY`
  lcd main|jog <out.png>            save a screen
  shot <out.png>                    save the window
  match main|jog <ref.png> [opts]   wait until a screen matches ref.png
  diff <ref.png> <actual.png> [opts]
  changes main|jog <x0,y0,x1,y1> [--timeout MS]
  still main|jog <x0,y0,x1,y1> [--for MS]
  sh <command>                      run a guest command and print its output
  run <file>                        run one command per line
  e2e [options] <file|dir>...       run scenarios; see docs/e2e.md
  watch <topic> [filters] [--for MS] [--count N]
  sleep <ms>
diff options: --threshold N  --max N  --mask x0,y0,x1,y1  --region x0,y0,x1,y1
              --out diff.png  --timeout MS  --settle MS";

/// Run `ctl` and exit when it is the first argument.
pub fn run_if_asked(args: &[String]) {
    if args.get(1).map(String::as_str) == Some("ctl") {
        std::process::exit(run(&args[2..]));
    }
}

/// Run `ctl` with the arguments after `ctl`, and return the exit status.
pub fn run(args: &[String]) -> i32 {
    let mut session = Session::new("127.0.0.1", 1);
    let words = match global_options(&mut session, args) {
        Ok(words) => words,
        Err(e) => return usage(&e),
    };
    match words.first().map(String::as_str) {
        None | Some("help" | "--help" | "-h") => usage(""),
        Some("e2e") => match crate::e2e::run(&session.host, &words[1..]) {
            Ok(status) => status,
            Err(e) => usage(&e),
        },
        Some("run") => match words.get(1) {
            Some(file) => run_file(&mut session, Path::new(file)),
            None => usage("run needs a file"),
        },
        Some(_) => match exec(&mut session, words) {
            Ok(()) => 0,
            Err(Failure::Usage(e)) => usage(&e),
            Err(Failure::Failed(e)) => {
                eprintln!("cdj3k-emu ctl: {e}");
                1
            }
        },
    }
}

/// Apply `--slot` and `--host` to `session`, and return the words after them.
fn global_options<'a>(session: &mut Session, args: &'a [String]) -> Result<&'a [String], String> {
    let mut words = args;
    loop {
        match words {
            [flag, value, rest @ ..] if flag == "--slot" => {
                session.slot = value
                    .parse()
                    .map_err(|_| format!("{value:?} is not a slot number"))?;
                words = rest;
            }
            [flag, value, rest @ ..] if flag == "--host" => {
                session.host = value.clone();
                words = rest;
            }
            _ => return Ok(words),
        }
    }
}

fn usage(why: &str) -> i32 {
    if !why.is_empty() {
        eprintln!("cdj3k-emu ctl: {why}");
    }
    eprintln!("{USAGE}");
    2
}

/// Run each line of `file` as a command, over one connection. It stops at
/// the first failure.
fn run_file(s: &mut Session, file: &Path) -> i32 {
    let text = match std::fs::read_to_string(file) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("cdj3k-emu ctl: {}: {e}", file.display());
            return 1;
        }
    };
    for (n, line) in text.lines().enumerate() {
        let words = script_words(line);
        if words.is_empty() {
            continue;
        }
        if let Err(Failure::Usage(e) | Failure::Failed(e)) = exec(s, &words) {
            eprintln!("cdj3k-emu ctl: {} line {}: {e}", file.display(), n + 1);
            return 1;
        }
    }
    0
}
