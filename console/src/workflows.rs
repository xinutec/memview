//! A workflow run, read from what the harness writes beside the session's
//! transcript: `<session>/subagents/workflows/<run>/`, holding `journal.jsonl` —
//! a line when each agent starts and when it returns — and, per agent, its
//! transcript `agent-<id>.jsonl` and `agent-<id>.meta.json`.
//!
//! The journal says which agents exist and which have returned. Where a `started`
//! line carries no label or phase, the agent's meta file does. Nothing marks the
//! run itself finished: that is the session's background task ending.

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

/// The run as its journal says, with `meta` answering for the agents whose
/// `started` line did not name their label or phase. Pure: `latest` is left empty.
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

/// Whether `run` has the shape the harness gives run ids, `wf_` and hex with
/// dashes — checked before it becomes part of a path.
pub fn is_run(run: &str) -> bool {
    run.strip_prefix("wf_").is_some_and(|rest| {
        !rest.is_empty() && rest.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    })
}

/// Whether `agent` has the shape of an agent id — checked before it becomes part of a path.
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

/// The run in `dir`, with each working agent's last tool call. `None` when the run
/// has no journal; an error when an agent's meta file cannot be read.
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
