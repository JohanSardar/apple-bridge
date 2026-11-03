#![forbid(unsafe_code)]
use ab_shortcuts::{Error, cli::open_pack, pack};
use std::path::PathBuf;

fn flags(
    args: &[String],
    names: &[&str],
    booleans: &[&str],
) -> Result<Vec<(String, String)>, Error> {
    let mut found = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let Some(flag) = args[index]
            .strip_prefix("--")
            .or_else(|| args[index].strip_prefix('-'))
        else {
            return Err(Error::message("unexpected argument"));
        };
        let (key, inline) = match flag.split_once('=') {
            Some((key, value)) => (key, Some(value.to_owned())),
            None => (flag, None),
        };
        if booleans.contains(&key) {
            found.push((key.to_owned(), inline.unwrap_or_else(|| "true".into())));
        } else if names.contains(&key) {
            let value = if let Some(value) = inline {
                value
            } else {
                index += 1;
                args.get(index)
                    .cloned()
                    .ok_or_else(|| Error::message("missing flag value"))?
            };
            found.push((key.to_owned(), value));
        } else {
            return Err(Error::message("unknown flag"));
        }
        index += 1;
    }
    Ok(found)
}

fn run(args: &[String]) -> Result<(), Error> {
    let usage = "usage: ab-shortcuts build [-source file.json] [-out directory] [-unsigned] [-mode auto|anyone|people-who-know-me] | open file.shortcut | install-pack [-pack directory]";
    match args.first().map(String::as_str) {
        None => Err(Error::message(usage)),
        Some("open") => {
            if args.len() != 2 {
                return Err(Error::message("open requires one signed .shortcut file"));
            }
            pack::open(&PathBuf::from(&args[1]))
        }
        Some("install-pack") => {
            let mut directory = PathBuf::from(".build/shortcuts");
            for (key, value) in flags(&args[1..], &["pack"], &[])? {
                if key == "pack" {
                    directory = PathBuf::from(value);
                }
            }
            open_pack(&directory)
        }
        Some("build") => {
            let mut options = pack::BuildOptions::default();
            for (key, value) in flags(&args[1..], &["source", "out", "mode"], &["unsigned"])? {
                match key.as_str() {
                    "source" => options.source = value,
                    "out" => options.out = PathBuf::from(value),
                    "mode" => options.mode = value,
                    _ => options.unsigned = value == "true",
                }
            }
            print!("{}", pack::build(&options)?);
            Ok(())
        }
        Some(other) => Err(Error::Message(format!("unknown command {other:?}"))),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(error) = run(&args) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
