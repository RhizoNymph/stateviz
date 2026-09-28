//! `cascade simulate <file> [scenario] --interactive`: a small prompt for
//! playing a system one action at a time with a [`PlaySession`].
//!
//! Commands are read line by line from standard input (a prompt is shown
//! only on a terminal). A mistake prints `error: …` to standard error and
//! play goes on; end of input or `quit` ends the session.

use std::collections::BTreeMap;
use std::io::{BufRead, IsTerminal, Write};
use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context, bail};
use cascade_core::Model;
use cascade_core::parse::grammar::parse_trigger_ref;
use cascade_sim::{PlayAction, PlaySession, Scenario, SimError, SimRun, scenario_to_yaml};

use super::{report, step_lines, trace_text};

const HELP: &str = "\
Commands:
  add <Machine> [name] [@state] [field=value ...]   create an instance
  remove <name>                                     take an instance out of play
  fire <Source> <Machine.trigger> <target> [key=value ...]
                                                    an external source fires a trigger
  step [n]                                          deliver the queue head, or pending item n
  run                                               deliver until the queue is empty
  instances | pending | fires                       show instances, the queue, what can fire
  trace | timeline | branches                       show the trace, the actions, saved branches
  seek <n>                                          move to timeline position n (0 = start)
  branch <i>                                        switch to saved branch i
  save <path>                                       save the timeline so far as a scenario
  quit                                              leave
";

pub fn run(file: &Path, model: &Model, start: Option<(&Scenario, &Path)>) -> anyhow::Result<ExitCode> {
    let mut session = match start {
        None => PlaySession::new(model),
        Some((scenario, path)) => match PlaySession::from_scenario(model, scenario) {
            Ok(session) => session,
            Err(SimError::Scenario(err)) => return Ok(report(path, &err)),
            Err(err) => return Err(err.into()),
        },
    };

    let interactive = std::io::stdin().is_terminal();
    println!("Playing {}. Type `help` for the commands.", file.display());
    if start.is_some() {
        print!("{}", step_lines(model, &run_of(&session), 0..session.trace().steps.len()));
        print_pending_count(&session);
    }

    let stdin = std::io::stdin();
    let mut lines = stdin.lock().lines();
    loop {
        if interactive {
            print!("> ");
            std::io::stdout().flush().context("cannot write the prompt")?;
        }
        let Some(line) = lines.next() else { break };
        let line = line.context("cannot read a command")?;
        let words: Vec<&str> = line.split_whitespace().collect();
        match words.first() {
            None => {}
            Some(&("quit" | "exit")) => break,
            Some(_) => {
                if let Err(err) = command(model, &mut session, &words) {
                    eprintln!("error: {err:#}");
                }
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn run_of(session: &PlaySession) -> SimRun {
    SimRun { trace: session.trace().clone(), payloads: session.payloads().clone() }
}

fn command(model: &Model, session: &mut PlaySession, words: &[&str]) -> anyhow::Result<()> {
    match words {
        ["help"] => print!("{HELP}"),
        ["instances"] => {
            for instance in session.instances() {
                let fields = if instance.fields.is_empty() {
                    String::new()
                } else {
                    format!("  {}", cascade_sim::payload_text(&instance.fields))
                };
                println!(
                    "  {}  {}  {}{fields}",
                    instance.name,
                    model.machine(instance.machine).name,
                    model.state(instance.state).path
                );
            }
        }
        ["pending"] => {
            for (i, item) in session.pending().iter().enumerate() {
                println!("  {i}  {}", item.label);
            }
        }
        ["fires"] => {
            for fire in session.available_fires(model) {
                let note = if fire.accepted { "" } else { "  (would be dropped)" };
                println!("  {} {} {}{note}", fire.source, fire.trigger, fire.target);
            }
        }
        ["trace"] => print!("{}", trace_text(model, &run_of(session))),
        ["timeline"] => {
            let timeline = session.timeline();
            for (i, action) in timeline.actions.iter().enumerate() {
                println!("  {i}  {}", action_text(action));
            }
            println!("  at {} of {}", timeline.position, timeline.actions.len());
        }
        ["branches"] => {
            for (i, branch) in session.timeline().branches.iter().enumerate() {
                println!("  {i}  fork at {}: {} actions", branch.fork, branch.actions.len());
            }
        }
        ["seek" | "rewind", n] => {
            session.seek(model, n.parse().with_context(|| format!("`{n}` is not a timeline position"))?)?;
            print_position(session);
        }
        ["branch", i] => {
            session.switch_branch(model, i.parse().with_context(|| format!("`{i}` is not a branch number"))?)?;
            print_position(session);
        }
        ["save", path] => {
            let path = Path::new(path);
            let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("played").to_owned();
            let yaml = scenario_to_yaml(&session.to_scenario(&name)?);
            std::fs::write(path, yaml).with_context(|| format!("cannot write {}", path.display()))?;
            println!("saved {}", path.display());
        }
        _ => {
            let action = parse_action(words)?;
            let outcome = session.apply(model, action)?;
            match session.timeline().actions.get(session.timeline().position.saturating_sub(1)) {
                Some(PlayAction::AddInstance { name: Some(name), .. }) => println!("  added {name}"),
                Some(PlayAction::RemoveInstance { name }) => println!("  removed {name}"),
                _ => print!("{}", step_lines(model, &run_of(session), outcome.steps)),
            }
            print_pending_count(session);
        }
    }
    Ok(())
}

fn parse_action(words: &[&str]) -> anyhow::Result<PlayAction> {
    Ok(match words {
        ["add", machine, rest @ ..] => {
            let (name, rest) = match rest {
                [name, rest @ ..] if !name.contains('=') && !name.starts_with('@') => (Some((*name).to_owned()), rest),
                _ => (None, rest),
            };
            let state = rest.iter().find_map(|w| w.strip_prefix('@')).map(str::to_owned);
            let fields = pairs(rest.iter().copied().filter(|w| !w.starts_with('@')))?;
            PlayAction::AddInstance { name, machine: (*machine).to_owned(), fields, state }
        }
        ["remove", name] => PlayAction::RemoveInstance { name: (*name).to_owned() },
        ["fire", source, trigger, target, rest @ ..] => {
            let Some(trigger) = parse_trigger_ref(trigger) else {
                bail!("`{trigger}` is not a trigger reference; expected `Machine.trigger`");
            };
            PlayAction::Fire {
                source: (*source).to_owned(),
                trigger,
                target: (*target).to_owned(),
                payload: pairs(rest.iter().copied())?,
            }
        }
        ["step"] => PlayAction::Step { choice: None },
        ["step", n] => {
            PlayAction::Step { choice: Some(n.parse().with_context(|| format!("`{n}` is not a queue position"))?) }
        }
        ["run"] => PlayAction::RunUntilQuiet,
        [word, ..] if ["add", "remove", "fire", "step", "seek", "rewind", "branch", "save"].contains(word) => {
            bail!("wrong arguments for `{word}`; type `help` for the commands")
        }
        [word, ..] => bail!("unknown command `{word}`; type `help` for the commands"),
        [] => bail!("no command"),
    })
}

/// `key=value` words.
fn pairs<'a>(words: impl Iterator<Item = &'a str>) -> anyhow::Result<BTreeMap<String, String>> {
    words
        .map(|w| match w.split_once('=') {
            Some((k, v)) => Ok((k.to_owned(), v.to_owned())),
            None => bail!("`{w}` is not `key=value`"),
        })
        .collect()
}

fn action_text(action: &PlayAction) -> String {
    let pairs_text = |map: &BTreeMap<String, String>| map.iter().map(|(k, v)| format!(" {k}={v}")).collect::<String>();
    match action {
        PlayAction::AddInstance { name, machine, fields, state } => format!(
            "add {machine}{}{}{}",
            name.as_deref().map(|n| format!(" {n}")).unwrap_or_default(),
            state.as_deref().map(|s| format!(" @{s}")).unwrap_or_default(),
            pairs_text(fields)
        ),
        PlayAction::RemoveInstance { name } => format!("remove {name}"),
        PlayAction::Fire { source, trigger, target, payload } => {
            format!("fire {source} {trigger} {target}{}", pairs_text(payload))
        }
        PlayAction::Step { choice: None } => "step".to_owned(),
        PlayAction::Step { choice: Some(n) } => format!("step {n}"),
        PlayAction::RunUntilQuiet => "run until quiet".to_owned(),
    }
}

fn print_pending_count(session: &PlaySession) {
    let pending = session.pending().len();
    if pending > 0 {
        println!("  ({pending} pending)");
    }
}

fn print_position(session: &PlaySession) {
    let timeline = session.timeline();
    println!("  at {} of {}", timeline.position, timeline.actions.len());
    print_pending_count(session);
}
