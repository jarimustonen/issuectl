//! `issuectl docs [topic]` — bundled long-form documentation.
//!
//! Each topic is an embedded markdown file under `templates/docs/`.
//! Add new topics by dropping a file in that directory and listing it
//! in `TOPICS`.

use anyhow::Result;

const KANBAN: &str = include_str!("../templates/docs/kanban.md");

/// (topic name, one-line summary, body). The summary is shown by the
/// topic-list output; the body is what `issuectl docs <topic>` prints.
const TOPICS: &[(&str, &str, &str)] = &[(
    "kanban",
    "Read-only web board (`issuectl serve`)",
    KANBAN,
)];

pub fn run(topic: Option<String>) -> Result<()> {
    match topic {
        None => {
            print_index();
            Ok(())
        }
        Some(t) => match TOPICS.iter().find(|(name, _, _)| *name == t) {
            Some((_, _, body)) => {
                print!("{body}");
                Ok(())
            }
            None => {
                eprintln!(
                    "Error: unknown docs topic {t:?}. Run `issuectl docs` to list topics."
                );
                std::process::exit(1);
            }
        },
    }
}

fn print_index() {
    println!("issuectl docs — bundled documentation");
    println!();
    println!("Usage: issuectl docs <topic>");
    println!();
    println!("Topics:");
    let width = TOPICS.iter().map(|(n, _, _)| n.len()).max().unwrap_or(0);
    for (name, summary, _) in TOPICS {
        println!("  {name:width$}  {summary}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kanban_topic_is_registered() {
        assert!(TOPICS.iter().any(|(n, _, _)| *n == "kanban"));
    }

    #[test]
    fn kanban_body_mentions_serve_command() {
        let (_, _, body) = TOPICS.iter().find(|(n, _, _)| *n == "kanban").unwrap();
        assert!(body.contains("issuectl serve"));
    }
}
