//! ⚠ **A measurement, not a repair — memview#1503.** `blame::attribute` calls
//! the EARLIEST write a creation, and a write is not a creation: `>>`,
//! `sed -i`, an `Edit` and a `cp` onto an existing path all write a file that
//! already existed. This asks how much of the corpus's recorded authorship
//! rests on a write that could only have been an EDIT.
//!
//! ⚠ **It does NOT correct anything.** That ticket is explicit: half of every
//! earlier pass dissolved on being read, and bulk-correcting from a proxy is
//! what it exists to warn against.

use std::collections::BTreeMap;
use std::path::Path;

/// Could this Bash command have CREATED the file, or does it require one?
///
/// ⚠ **A text test, and it is named as one.** `FileUse` carries `write` and not
/// whether the write truncates, so the append/create distinction the syntax tree
/// holds (`RedirectOp::Append`) is gone by the time the reader answers. Fixing
/// that properly means a field on `FileUse`; this is the cheap proxy that says
/// whether the field is worth adding.
fn bash_could_create(cmd: &str, name: &str) -> bool {
    // Anything that appends or edits in place requires the file to exist.
    let needs_existing = [">>", "sed -i", "perl -pi", "tee -a"];
    let mentions_append = needs_existing.iter().any(|marker| {
        cmd.match_indices(marker).any(|(at, _)| {
            // the marker has to be near this file's name to be about it
            cmd[at..].contains(name)
                || cmd[..at]
                    .rsplit('\n')
                    .next()
                    .is_some_and(|l| l.contains(name))
        })
    });
    !mentions_append
}

fn main() -> anyhow::Result<()> {
    let home = std::env::var("HOME")?;
    let dir = format!("{home}/.claude/projects/-Users-pippijn-Code/memory");
    let root = format!("{home}/.claude/projects");

    let mut wanted = Vec::new();
    let mut recorded: BTreeMap<String, String> = BTreeMap::new();
    for entry in std::fs::read_dir(&dir)?.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(".md") || name == "MEMORY.md" {
            continue;
        }
        let text = std::fs::read_to_string(entry.path()).unwrap_or_default();
        if let Some(line) = text
            .lines()
            .find(|l| l.trim_start().starts_with("originSessionId:"))
        {
            recorded.insert(
                name.clone(),
                line.split(':').nth(1).unwrap_or("").trim().to_string(),
            );
        }
        wanted.push(name);
    }

    // The earliest write per memory, with the TOOL that made it.
    let mut earliest: BTreeMap<String, (String, String, String, String)> = BTreeMap::new();
    for path in memview::blame::transcripts(Path::new(&root)) {
        let session = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for line in text.lines() {
            let Some(name) = wanted.iter().find(|n| line.contains(n.as_str())) else {
                continue;
            };
            let Ok(row) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            let (Some(at), Some(content)) = (
                row["timestamp"].as_str(),
                row["message"]["content"].as_array(),
            ) else {
                continue;
            };
            for item in content {
                if item["type"] != "tool_use" {
                    continue;
                }
                let tool = item["name"].as_str().unwrap_or("");
                let hit = match tool {
                    "Write" | "Edit" | "MultiEdit" | "NotebookEdit" => item["input"]["file_path"]
                        .as_str()
                        .is_some_and(|p| p.ends_with(name.as_str())),
                    "Bash" => item["input"]["command"].as_str().is_some_and(|c| {
                        memview::blame::writes(
                            c,
                            row["cwd"].as_str().filter(|c| !c.is_empty()),
                            &home,
                            name,
                        )
                    }),
                    _ => false,
                };
                if !hit {
                    continue;
                }
                let cmd = item["input"]["command"].as_str().unwrap_or("").to_string();
                if earliest
                    .get(name.as_str())
                    .is_none_or(|(a, ..)| at < a.as_str())
                {
                    earliest.insert(
                        name.clone(),
                        (at.to_string(), session.clone(), tool.to_string(), cmd),
                    );
                }
            }
        }
    }

    let (mut creating, mut editing, mut none) = (0, 0, 0);
    // ⚠ **The split that matters, and the first run buried it.** An origin
    // resting on an edit is only a DEFECT where the corpus recorded that
    // editing session; where it records somebody else the answer came from
    // elsewhere and is already right. Reporting only "36 rest on an edit" makes
    // the reader do that arithmetic, and the two halves mean opposite things.
    let (mut live, mut already_right) = (0, 0);
    let mut suspect: Vec<(String, String, String)> = Vec::new();
    for name in &wanted {
        let Some((_, session, tool, cmd)) = earliest.get(name) else {
            none += 1;
            continue;
        };
        let creates = match tool.as_str() {
            "Write" => true,
            "Edit" | "MultiEdit" | "NotebookEdit" => false,
            _ => bash_could_create(cmd, name),
        };
        if creates {
            creating += 1
        } else {
            editing += 1;
            let took_the_editor = recorded.get(name).is_some_and(|r| r == session);
            if took_the_editor {
                live += 1
            } else {
                already_right += 1
            }
            if took_the_editor {
                suspect.push((
                    name.clone(),
                    tool.clone(),
                    cmd.replace('\n', " ⏎ ")
                        .chars()
                        .take(64)
                        .collect::<String>(),
                ));
            }
        }
    }
    println!("{} memories\n", wanted.len());
    println!("  earliest write could CREATE      {creating}");
    println!("  earliest write REQUIRES the file {editing}   <- an edit, not a creation");
    println!("      of those, the corpus TOOK that editor as the origin  {live}   <- the defect");
    println!(
        "      of those, the corpus records somebody else           {already_right}   <- already right"
    );
    println!("  no write found at all            {none}\n");
    println!("the {live} whose recorded origin rests on an edit:\n");
    suspect.sort();
    for (name, tool, note) in suspect.iter().take(20) {
        println!("  {:52} {tool:10} {note}", name.trim_end_matches(".md"));
    }
    if suspect.len() > 20 {
        println!("  … and {} more", suspect.len() - 20)
    }
    Ok(())
}
