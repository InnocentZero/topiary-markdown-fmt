use std::fs;
use std::io::{self, BufRead, Read, Write};
use std::process::ExitCode;

use serde::Deserialize;
use serde_json::json;
use topiary_markdown::{Formatter, strict_violations, verification, violations};

#[derive(Deserialize)]
struct Request {
    source: String,
    original: Option<String>,
    #[serde(default)]
    audit: bool,
}

fn worker(formatter: &Formatter, upstream: bool) -> Result<(), String> {
    let output = io::stdout();
    let mut out = output.lock();
    for line in io::stdin().lock().lines() {
        let line = line.map_err(|err| err.to_string())?;
        let request: Request = serde_json::from_str(&line).map_err(|err| err.to_string())?;
        let format = |source| {
            if upstream {
                formatter.upstream(source)
            } else {
                formatter.format(source)
            }
        };
        let response = match format(&request.source) {
            Ok(first) if request.audit => {
                let verified = verification(&request.source, &first);
                let second = format(&first);
                json!({"output": first, "ast": verified.ast,
                       "protected": verified.protected, "verification": verified.issue,
                       "style": violations(&first),
                       "idempotent": second.as_ref().is_ok_and(|value| value == &first),
                       "second_error": second.err()})
            }
            Ok(first) => json!({"output": first}),
            Err(error) => json!({"error": error}),
        };
        writeln!(out, "{response}").map_err(|err| err.to_string())?;
        out.flush().map_err(|err| err.to_string())?;
    }
    Ok(())
}

fn run() -> Result<ExitCode, String> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() == 1 && (args[0] == "--style-worker" || args[0] == "--verify-worker") {
        let mut out = io::stdout().lock();
        for line in io::stdin().lock().lines() {
            let line = line.map_err(|error| error.to_string())?;
            let request: Request =
                serde_json::from_str(&line).map_err(|error| error.to_string())?;
            let verdict = if args[0] == "--style-worker" {
                json!({"style": strict_violations(&request.source)})
            } else if let Some(original) = &request.original {
                let checked = verification(original, &request.source);
                json!({"ast": checked.ast, "protected": checked.protected, "verification": checked.issue})
            } else {
                json!({"error": "missing original source"})
            };
            writeln!(out, "{verdict}")
                .and_then(|_| out.flush())
                .map_err(|error| error.to_string())?;
        }
        return Ok(ExitCode::SUCCESS);
    }
    let format = Formatter::new()?;
    match args.as_slice() {
        [] => {
            let mut input = String::new();
            io::stdin()
                .read_to_string(&mut input)
                .map_err(|err| err.to_string())?;
            let formatted = format.format(&input)?;
            io::stdout()
                .write_all(formatted.as_bytes())
                .map_err(|err| err.to_string())?;
        }
        [flag] if flag == "--worker" || flag == "--upstream-worker" => {
            worker(&format, flag == "--upstream-worker")?;
        }
        [flag, path] if flag == "--check" || flag == "--write" => {
            let source = fs::read_to_string(path).map_err(|err| err.to_string())?;
            let output = format.format(&source)?;
            if output != source {
                if flag == "--check" {
                    return Ok(ExitCode::from(1));
                }
                fs::write(path, output).map_err(|err| err.to_string())?;
            }
        }
        [flag] if flag == "--help" || flag == "-h" => {
            println!(
                "Usage: topiary-markdown [--check FILE | --write FILE]\nWithout arguments, format standard input to standard output."
            );
        }
        _ => return Err("usage: topiary-markdown [--check FILE | --write FILE]".into()),
    }
    Ok(ExitCode::SUCCESS)
}

fn main() -> ExitCode {
    match run() {
        Ok(status) => status,
        Err(err) => {
            eprintln!("topiary-markdown: {err}");
            ExitCode::from(2)
        }
    }
}
