//! A workflow run, from `<session>/subagents/workflows/<run>/`: its `journal.jsonl`
//! of agents starting and returning, and each agent's transcript and meta file.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::protocol::{Event, Timed};

/// One run: its agents, grouped by phase in the order the phases began.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Run {
    pub phases: Vec<Phase>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Phase {
    /// `None` for agents the script started outside any phase.
    pub title: Option<String>,
    pub agents: Vec<Agent>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Agent {
    pub id: String,
    /// The label the script gave it.
    pub label: Option<String>,
    /// Whether it has returned its result.
    pub done: bool,
    /// The last tool call of an agent still working.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub latest: Option<Timed>,
}

/// One journal line. Other kinds (`launched`) and unreadable lines are skipped.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum Journaled {
    Started {
        #[serde(rename = "agentId")]
        agent: String,
        label: Option<String>,
        phase: Option<String>,
    },
    Result {
        #[serde(rename = "agentId")]
        agent: String,
    },
}

/// What an agent's meta file says about it.
#[derive(Debug, Default, Deserialize)]
pub struct Meta {
    pub description: Option<String>,
    #[serde(rename = "workflowPhase")]
    pub phase: Option<String>,
}

/// The run as its journal says, with `meta` filling in labels and phases.
pub fn read(journal: &str, meta: impl Fn(&str) -> anyhow::Result<Meta>) -> anyhow::Result<Run> {
    let lines: Vec<Journaled> = journal
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    let done: BTreeSet<&str> = lines
        .iter()
        .filter_map(|line| match line {
            Journaled::Result { agent } => Some(agent.as_str()),
            Journaled::Started { .. } => None,
        })
        .collect();
    let mut seen = BTreeSet::new();
    let mut phases: Vec<Phase> = Vec::new();
    for line in &lines {
        let Journaled::Started {
            agent,
            label,
            phase,
        } = line
        else {
            continue;
        };
        if !seen.insert(agent.as_str()) {
            continue;
        }
        let (label, phase) = match (label, phase) {
            (Some(label), Some(phase)) => (Some(label.clone()), Some(phase.clone())),
            _ => {
                let meta = meta(agent)?;
                (
                    label.clone().or(meta.description),
                    phase.clone().or(meta.phase),
                )
            }
        };
        let agent = Agent {
            id: agent.clone(),
            label,
            done: done.contains(agent.as_str()),
            latest: None,
        };
        match phases.iter_mut().find(|known| known.title == phase) {
            Some(known) => known.agents.push(agent),
            None => phases.push(Phase {
                title: phase,
                agents: vec![agent],
            }),
        }
    }
    Ok(Run { phases })
}

/// Whether `run` looks like a run id — checked before it joins a path.
// dev-lint: sanitises path — `wf_` then letters, digits and `-`
pub fn is_run(run: &str) -> bool {
    run.strip_prefix("wf_").is_some_and(|rest| {
        !rest.is_empty() && rest.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    })
}

/// Whether `agent` looks like an agent id — checked before it joins a path.
// dev-lint: sanitises path — letters and digits only
pub fn is_agent(agent: &str) -> bool {
    !agent.is_empty() && agent.chars().all(|c| c.is_ascii_alphanumeric())
}

/// Where a run's files are, beside the transcript of the session that launched it.
pub fn dir(transcript: &Path, run: &str) -> PathBuf {
    transcript
        .with_extension("")
        .join("subagents")
        .join("workflows")
        .join(run)
}

/// One agent's transcript.
pub fn transcript(dir: &Path, agent: &str) -> PathBuf {
    dir.join(format!("agent-{agent}.jsonl"))
}

/// The run in `dir`, with each working agent's last tool call; `None` with no journal.
pub fn run(dir: &Path) -> anyhow::Result<Option<Run>> {
    let journal = match std::fs::read_to_string(dir.join("journal.jsonl")) {
        Ok(journal) => journal,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err.into()),
    };
    let mut run = read(&journal, |agent| {
        match std::fs::read(dir.join(format!("agent-{agent}.meta.json"))) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            // Not written yet: the journal alone says the agent exists.
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Meta::default()),
            Err(err) => Err(err.into()),
        }
    })?;
    for agent in run.phases.iter_mut().flat_map(|phase| &mut phase.agents) {
        if !agent.done {
            agent.latest = crate::past::page(&transcript(dir, &agent.id), None)
                .events
                .into_iter()
                .rfind(|timed| matches!(timed.event, Event::Tool { .. }));
        }
    }
    Ok(Some(run))
}
