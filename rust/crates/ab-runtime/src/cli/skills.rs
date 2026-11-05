use super::{Failure, Outcome};

pub const SKILLS: &[(&str, &str, &str)] = &[
    (
        "core",
        "What Apple Bridge is, how to read safely, and the tools to start with",
        include_str!("../../skills/core.md"),
    ),
    (
        "setup",
        "How to set up Apple Bridge on this Mac and connect an agent",
        include_str!("../../skills/setup.md"),
    ),
];

pub fn list() {
    for (name, summary, _) in SKILLS {
        println!("{name:<8} {summary}");
    }
}

pub fn get(name: &str) -> Outcome {
    let (_, _, text) = SKILLS
        .iter()
        .find(|(skill, _, _)| *skill == name)
        .ok_or_else(|| {
            Failure::new(format!("There is no skill called `{name}`."))
                .next("Run `bridge skills list`.")
        })?;
    print!("{text}");
    Ok(())
}
