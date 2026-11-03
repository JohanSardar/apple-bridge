use ab_native::messages::Reader;
use std::{
    io::{self, BufRead, Write},
    path::Path,
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().collect::<Vec<_>>();
    let mut reader = Reader::open(
        Path::new(args.get(1).ok_or("database argument required")?),
        Path::new(args.get(2).ok_or("synthetic home argument required")?),
    )?;
    for line in io::stdin().lock().lines() {
        let line = line?;
        if let Some(args) = line.strip_prefix("resume ") {
            let (collection, key) = args.split_once(' ').ok_or("resume arguments")?;
            reader.resume_candidates(collection, [key.to_string()])?;
            println!("END");
            io::stdout().flush()?;
            continue;
        }
        if let Some(args) = line.strip_prefix("blob ") {
            let fields = args.split_whitespace().collect::<Vec<_>>();
            let bytes = reader.read_blob(fields[0], fields[1].parse()?, fields[2].parse()?);
            println!(
                "BLOB {}",
                serde_json::json!({"ok":bytes.is_ok(),"text":bytes.ok().map(|b|String::from_utf8_lossy(&b).into_owned())})
            );
            println!("END");
            io::stdout().flush()?;
            continue;
        }
        let batches = if line == "begin" || line == "page" {
            reader.step(line == "begin")?
        } else if args.iter().any(|a| a == "--paged") {
            let mut batches = reader.step(line == "complete")?;
            while reader.reconciliation_pending() {
                batches.extend(reader.step(false)?);
            }
            batches
        } else {
            reader.poll(line == "complete")?
        };
        let mut observations = batches
            .iter()
            .flat_map(|b| b.observations.iter())
            .collect::<Vec<_>>();
        observations.sort_by_key(|o| (&o.collection, &o.native_key, &o.kind));
        for observation in observations {
            io::stdout().write_all(&observation.bytes()?)?;
            println!();
        }
        let (rows, hashes) = reader.work_counts();
        println!(
            "END {}",
            serde_json::json!({"generation":reader.generation,"rows":rows,"hashes":hashes,"complete":batches.iter().any(|b|b.complete)})
        );
        io::stdout().flush()?;
    }
    Ok(())
}
