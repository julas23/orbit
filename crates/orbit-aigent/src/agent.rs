//! The coding agent: one run per request, headless, its answer read back.
//!
//! Claude Code runs as `claude -p <request> --output-format stream-json`,
//! the same session from one request to the next (`--resume`), told that
//! its answer will be spoken. What only reads (files, searches, a few
//! commands that change nothing) it may do at once; anything else it asks
//! for through `--permission-prompt-tool`, which is `orbit-aigent approve`
//! as an MCP server (`approve.rs`): the daemon asks aloud and waits for a
//! spoken yes or a click.
//!
//! Codex CLI (OpenAI's), Gemini CLI, OpenCode and Kimi Code are not driven yet.

use std::{
    io::{BufRead, BufReader},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
};

use crate::config::{Agent, Kind, expand};

/// What the agent is told about how its answer is used.
const SPOKEN: &str = "You are Orbit-Aigent, the voice assistant of the user's Linux desktop (Orbit). \
The user talks to you by voice and your reply is read aloud by a text-to-speech voice, so: \
answer in the language of the request (Brazilian Portuguese or English); \
be brief, one to three short sentences unless asked for more; \
write plain spoken text, with no Markdown, lists, tables, code blocks or emoji; \
do not read out long paths, URLs or command lines, describe them instead; \
write numbers as they are said. \
The request was transcribed from speech and may have small mistakes: take the likeliest meaning, \
and ask back briefly only when it is truly unclear. \
When asked to do something on the computer, do it with your tools and then say briefly what you did. \
The desktop is Orbit: its compositor, orbit-core, speaks sway's IPC, so swaymsg (or orbitctl) lists the \
outputs, workspaces and windows and runs commands such as `swaymsg exec <program>` or moving a window \
to an output. Each command that changes something is confirmed by the user by voice first, so use few \
and well-chosen ones.";

/// What it may do without asking: tools that only read, and commands that
/// only look.
const READ_ONLY: &[&str] = &[
    "Read",
    "Grep",
    "Glob",
    "LS",
    "WebSearch",
    "WebFetch",
    "Bash(date:*)",
    "Bash(cal:*)",
    "Bash(uptime:*)",
    "Bash(df:*)",
    "Bash(du:*)",
    "Bash(free:*)",
    "Bash(ls:*)",
    "Bash(pwd)",
    "Bash(cat:*)",
    "Bash(head:*)",
    "Bash(tail:*)",
    "Bash(wc:*)",
    "Bash(which:*)",
    "Bash(uname:*)",
    "Bash(lsblk:*)",
    "Bash(ip addr:*)",
    "Bash(ps:*)",
    "Bash(git status:*)",
    "Bash(git log:*)",
    "Bash(git diff:*)",
    "Bash(echo:*)",
    "Bash(hostname)",
    "Bash(whoami)",
    "Bash(id)",
    "Bash(command -v:*)",
    "Bash(type:*)",
    "Bash(printenv:*)",
    "Bash(pgrep:*)",
    "Bash(grep:*)",
    "Bash(jq:*)",
    "Bash(sort:*)",
    "Bash(uniq:*)",
    "Bash(cut:*)",
    "Bash(tr:*)",
    "Bash(wpctl status)",
    "Bash(wpctl get-volume:*)",
    "Bash(swaymsg -t get_outputs:*)",
    "Bash(swaymsg -t get_workspaces:*)",
    "Bash(swaymsg -t get_tree:*)",
    "Bash(swaymsg -t get_inputs:*)",
    "Bash(swaymsg -t get_version:*)",
    "Bash(orbitctl -t get_outputs:*)",
    "Bash(orbitctl -t get_workspaces:*)",
    "Bash(orbitctl -t get_tree:*)",
];

/// The agent's answer.
pub struct Answer {
    pub text: String,
    /// To resume the conversation with the next request.
    pub session: Option<String>,
}

/// The run under way, to stop it.
pub type Running = Arc<Mutex<Option<Child>>>;

/// Runs `request` with `agent` (named `name`), on the calling thread, until
/// it answers.
pub fn ask(name: &str, agent: &Agent, request: &str, session: Option<&str>, running: &Running) -> anyhow::Result<Answer> {
    let kind = agent.kind(name).ok_or_else(|| anyhow::anyhow!("agent {name} has no kind"))?;
    let program = agent
        .find_program(kind)
        .ok_or_else(|| anyhow::anyhow!("{} is not installed", kind.name()))?;
    match kind {
        Kind::Claude => claude(&program, agent, request, session, running),
        other => anyhow::bail!("{} is not driven yet", other.name()),
    }
}

fn claude(program: &PathBuf, agent: &Agent, request: &str, session: Option<&str>, running: &Running) -> anyhow::Result<Answer> {
    let me = std::env::current_exe()?;
    let mcp = serde_json::json!({
        "mcpServers": { "orbit_aigent": { "command": me, "args": ["approve"] } }
    });
    let mut command = Command::new(program);
    command
        .arg("-p")
        .arg(request)
        .args(["--output-format", "stream-json", "--verbose"])
        .args(["--append-system-prompt", SPOKEN])
        .arg("--mcp-config")
        .arg(mcp.to_string())
        .args(["--permission-prompt-tool", "mcp__orbit_aigent__approve"])
        .arg("--allowedTools")
        .args(READ_ONLY);
    if let Some(session) = session {
        command.args(["--resume", session]);
    }
    command
        .args(&agent.args)
        .current_dir(agent.dir.clone().map_or_else(home, |dir| expand(&dir)))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let stdout = child.stdout.take().expect("piped");
    // Read meanwhile, so a chatty stderr cannot fill its pipe and stall it.
    let stderr = child.stderr.take().expect("piped");
    let errors = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = std::io::Read::read_to_string(&mut BufReader::new(stderr), &mut text);
        text
    });
    running.lock().unwrap().replace(child);

    let mut answer = None;
    let mut session = session.map(str::to_owned);
    for line in BufReader::new(stdout).lines() {
        let Ok(line) = line else { break };
        let Ok(message) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
        if let Some(id) = message["session_id"].as_str() {
            session = Some(id.to_owned());
        }
        match message["type"].as_str() {
            Some("assistant") => {
                for part in message["message"]["content"].as_array().into_iter().flatten() {
                    if part["type"] == "tool_use" {
                        log::info!("aigent: claude uses {} {}", part["name"], part["input"]);
                    }
                }
            }
            Some("result") => {
                let text = message["result"].as_str().unwrap_or_default().to_owned();
                if message["is_error"].as_bool() == Some(true) {
                    log::warn!("aigent: claude: {line}");
                }
                answer = Some(text);
            }
            _ => {}
        }
    }
    let child = running.lock().unwrap().take();
    let status = child.map(|mut child| child.wait()).transpose()?;
    match answer {
        Some(text) => Ok(Answer { text, session }),
        None => {
            let error = errors.join().unwrap_or_default();
            match status {
                // Stopped on purpose.
                None => anyhow::bail!("stopped"),
                Some(status) => anyhow::bail!("claude ended ({status}) without an answer: {}", error.trim()),
            }
        }
    }
}

/// Stops the run under way, if any.
pub fn stop(running: &Running) {
    if let Some(mut child) = running.lock().unwrap().take() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| "/".into()))
}
