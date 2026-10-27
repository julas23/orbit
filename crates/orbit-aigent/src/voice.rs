//! The local voice: a small language model on this machine (Gemma 3 4B by
//! default) that only talks. Each request is first sorted (`route`): a
//! task, for the agent, which the voice only announces; or conversation,
//! which it answers itself (`answer`), sentence by sentence, for Piper to
//! start on the first while the rest is written. The agent's answers are
//! said as the agent wrote them (it is told they are spoken), and kept in
//! the voice's conversation, for "what did it change?". Summed up by the
//! voice instead, they came out with numbers changed ("861 MB free of
//! 100 GB" for "92 GB, 100% used"), and seconds later.
//!
//! The model runs in llama.cpp's `llama-server`, started by the daemon on
//! a free local port and stopped with it. Two slots, each with a prompt
//! that never changes, so each is read once and kept: slot 0 sorts, slot 1
//! talks (its conversation grows, and only the new part is read). Gemma 3
//! attends through a sliding window, which keeps no reusable prefix unless
//! the whole cache is kept (`--swa-full`).
//!
//! Sorting is a question of its own, with one word for the answer
//! (forced by a grammar) and examples as turns of a conversation: a small
//! model sorts far better so than while answering. Measured on 52 requests
//! (2026-10-27): no task taken for conversation; a few conversations
//! (concepts, opinions) sent to the agent, which only takes longer. The
//! date, the time and the language go with each turn, not in the prompt:
//! a clock there would make it read anew every minute.

use std::{
    io::{BufRead, BufReader},
    net::TcpListener,
    path::PathBuf,
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

use anyhow::{Context, bail};
use serde_json::{Value, json};

use crate::config;

/// Sorting's prompt: what each kind of request is.
const ROUTE: &str = "Classifique o pedido falado do usuário ao assistente do desktop.
Responda só com uma palavra: tarefa ou conversa.
tarefa: algo precisa ser feito, visto ou consultado no computador, nos arquivos, nos programas, no código, no sistema, na rede ou na internet; ou informação atual (clima, notícias, cotações, estado de máquinas); ou lembretes; ou continuar ou repetir uma ação. Todo pedido para fazer algo é tarefa, mesmo curto. Perguntas sobre o que o assistente consegue fazer são tarefa. Na dúvida, tarefa.
conversa: cumprimentos, agradecimentos, despedidas, conversa leve, explicar um conceito geral, uma conta, uma opinião, a data ou a hora, ou repetir e esclarecer o que o assistente acabou de dizer.";

/// Sorting's examples, as turns. None of them is one of the requests it
/// was measured on.
const EXAMPLES: [(&str, &str); 19] = [
    ("Oi, tudo certo por aí?", "conversa"),
    ("Abre o gerenciador de arquivos.", "tarefa"),
    ("O que é uma thread?", "conversa"),
    ("Desliga o som.", "tarefa"),
    ("Corrige o bug do painel que não mostra a bateria.", "tarefa"),
    ("Vai chover hoje?", "tarefa"),
    ("Quanto é cem dividido por oito?", "conversa"),
    ("Minimiza tudo.", "tarefa"),
    ("O servidor de backup está respondendo?", "tarefa"),
    ("Muito obrigado, era isso.", "conversa"),
    ("Compila o projeto e vê se tem warning.", "tarefa"),
    ("Qual é melhor pra scripts, Python ou Bash?", "conversa"),
    ("Mostra o que mudou desde ontem no repositório.", "tarefa"),
    ("Fala de novo, não ouvi.", "conversa"),
    ("Agenda uma reunião pra sexta.", "tarefa"),
    ("Thanks, that's all.", "conversa"),
    ("Show me the CPU usage.", "tarefa"),
    ("O que significa esse aviso que apareceu?", "tarefa"),
    ("Quanto é doze vezes quinze?", "conversa"),
];
const ROUTE_GRAMMAR: &str = r#"root ::= "tarefa" | "conversa""#;

/// Talking's prompt.
const TALK: &str = "Você é a voz do Orbit-Aigent, o assistente falado do desktop Linux Orbit.
Responda em uma ou duas frases curtas, como texto para ser lido em voz alta: sem listas, sem Markdown, sem emoji, sem caminhos ou comandos por extenso.
Responda na língua indicada no pedido.
Você não faz nada no computador e não tem informação atual: não diga que fez ou viu algo. Se não sabe, diga que não sabe.
O que o agente (Claude Code) fez e respondeu aparece na conversa como suas falas anteriores.";

/// The longest answer, in tokens.
const ANSWER_TOKENS: u32 = 120;
/// The conversation kept for talking, in turns (a request and its answer
/// are two).
const KEPT: usize = 16;
/// How long the server may take to load the model (from a network home,
/// 2.5 GB take a while).
const LOADING: Duration = Duration::from_secs(180);

/// What a request is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// For the agent.
    Task,
    /// For the voice.
    Talk,
}

/// One turn of the conversation, as it was sent (so it is found in the
/// cache next time).
#[derive(Debug, Clone)]
pub struct Turn {
    pub user: bool,
    pub text: String,
}

/// A running server, to talk to.
pub struct Local {
    http: ureq::Agent,
    base: String,
}

/// Starts the server for `voice` and gives it to `ready` once it answers;
/// then waits on it, on the calling thread, which must live as long as the
/// daemon (the server is told to end when that thread does).
pub fn run(voice: &config::Voice, ready: impl FnOnce(Local)) -> anyhow::Result<()> {
    let model = voice.model_path();
    anyhow::ensure!(model.is_file(), "no model at {}", model.display());
    // A free port, given up just before the server takes it.
    let port = TcpListener::bind(("127.0.0.1", 0))?.local_addr()?.port();
    let mut command = Command::new(&voice.server);
    command
        .arg("-m")
        .arg(&model)
        .args(["--host", "127.0.0.1", "--port", &port.to_string()])
        .args(["-ngl", &voice.gpu_layers.to_string(), "-c", &voice.context.to_string()])
        .args(["-np", "2", "--swa-full", "--no-webui"])
        .envs(&voice.env)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    // Gone with the daemon, even killed: the signal comes when this thread ends.
    // Safety: only an async-signal-safe call between fork and exec.
    unsafe {
        std::os::unix::process::CommandExt::pre_exec(&mut command, || {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
            Ok(())
        });
    }
    let mut child = command.spawn().with_context(|| format!("starting {}", voice.server.display()))?;
    // Its log, kept short in ours: what went wrong, and where it runs.
    let stderr = child.stderr.take().expect("piped");
    let (said, lines) = mpsc::channel::<String>();
    std::thread::Builder::new().name("llama-log".into()).spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            let _ = said.send(line);
        }
    })?;
    let local = Local { http: ureq::Agent::config_builder().timeout_global(None).build().into(), base: format!("http://127.0.0.1:{port}") };
    let start = Instant::now();
    loop {
        for line in lines.try_iter() {
            if line.contains("error") || line.contains("no usable GPU") || line.contains("using device") || line.contains("offloaded") {
                log::info!("aigent: llama-server: {}", line.trim());
            }
        }
        if let Some(status) = child.try_wait()? {
            bail!("llama-server ended while loading ({status})");
        }
        if local.healthy() {
            break;
        }
        if start.elapsed() > LOADING {
            let _ = child.kill();
            bail!("llama-server did not load {} in {} s", model.display(), LOADING.as_secs());
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    // Both prompts read once now, not on the first request.
    let warm = Instant::now();
    let _ = local.route("oi");
    let _ = local.talk(&[], "oi", 8, |_| true);
    log::info!(
        "aigent: the local voice is up ({}, loaded in {:.1} s, warmed in {:.1} s)",
        model.file_name().unwrap_or_default().to_string_lossy(),
        warm.duration_since(start).as_secs_f32(),
        warm.elapsed().as_secs_f32()
    );
    ready(local);
    let status = child.wait()?;
    bail!("llama-server ended ({status})")
}

impl Local {
    fn healthy(&self) -> bool {
        self.http.get(format!("{}/health", self.base)).call().is_ok_and(|r| r.status() == 200)
    }

    fn post(&self, body: &Value) -> anyhow::Result<ureq::Body> {
        let response = self
            .http
            .post(format!("{}/v1/chat/completions", self.base))
            .content_type("application/json")
            .send(body.to_string())
            .context("the local voice")?;
        Ok(response.into_body())
    }

    /// What `request` is.
    pub fn route(&self, request: &str) -> anyhow::Result<Route> {
        let mut messages = vec![json!({ "role": "system", "content": ROUTE })];
        for (asked, kind) in EXAMPLES {
            messages.push(json!({ "role": "user", "content": asked }));
            messages.push(json!({ "role": "assistant", "content": kind }));
        }
        messages.push(json!({ "role": "user", "content": request }));
        let body = json!({ "messages": messages, "id_slot": 0, "cache_prompt": true, "temperature": 0, "max_tokens": 3, "grammar": ROUTE_GRAMMAR });
        let reply: Value = serde_json::from_reader(self.post(&body)?.into_reader())?;
        let word = reply["choices"][0]["message"]["content"].as_str().unwrap_or_default();
        Ok(if word.trim() == "conversa" { Route::Talk } else { Route::Task })
    }

    /// The voice's answer to `request`, after `before`, in `language`:
    /// each sentence to `phrase` as soon as it is whole (false from it
    /// stops). The whole answer, and the turn as sent, are returned.
    pub fn answer(&self, before: &[Turn], request: &str, language: &str, phrase: impl FnMut(String) -> bool) -> anyhow::Result<(Turn, String)> {
        let turn = asked(request, language);
        let text = self.talk(before, &turn.text, ANSWER_TOKENS, phrase)?;
        Ok((turn, text))
    }

    /// A turn of talking (slot 1), streamed.
    fn talk(&self, before: &[Turn], said: &str, tokens: u32, mut phrase: impl FnMut(String) -> bool) -> anyhow::Result<String> {
        let mut messages = vec![json!({ "role": "system", "content": TALK })];
        for turn in before {
            messages.push(json!({ "role": if turn.user { "user" } else { "assistant" }, "content": turn.text }));
        }
        messages.push(json!({ "role": "user", "content": said }));
        let body = json!({ "messages": messages, "id_slot": 1, "cache_prompt": true, "temperature": 0.3, "max_tokens": tokens, "stream": true });
        let reader = BufReader::new(self.post(&body)?.into_reader());
        let (mut all, mut pending) = (String::new(), String::new());
        for line in reader.lines() {
            let line = line?;
            let Some(data) = line.strip_prefix("data: ") else { continue };
            if data.trim() == "[DONE]" {
                break;
            }
            let chunk: Value = serde_json::from_str(data).unwrap_or_default();
            let Some(piece) = chunk["choices"][0]["delta"]["content"].as_str() else { continue };
            all.push_str(piece);
            pending.push_str(piece);
            while let Some(end) = sentence_end(&pending) {
                let sentence: String = pending.drain(..end).collect();
                let sentence = clean(&sentence);
                if !sentence.is_empty() && !phrase(sentence) {
                    return Ok(clean(&all));
                }
            }
        }
        let rest = clean(&pending);
        if !rest.is_empty() {
            phrase(rest);
        }
        Ok(clean(&all))
    }
}

/// The turn's note: the language to answer in, and the date and time, for
/// when they are asked.
fn with_note(text: &str, language: &str) -> String {
    let now = now_said(language);
    if language == "en" {
        format!("{text}\n\n(Answer in English. Only if asked: it is {now}.)")
    } else {
        format!("{text}\n\n(Responda em português do Brasil. Só se perguntarem: agora é {now}.)")
    }
}

/// The local date and time, said, from the system's clock and zone.
fn now_said(language: &str) -> String {
    let out = Command::new("date").arg("+%u %-d %-m %Y %H:%M").output().ok();
    let text = out.map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    let parts: Vec<&str> = text.split_whitespace().collect();
    let [weekday, day, month, year, time] = parts.as_slice() else { return String::new() };
    let index = |s: &str, max: usize| s.parse::<usize>().ok().filter(|n| (1..=max).contains(n)).map_or(0, |n| n - 1);
    if language == "en" {
        const DAYS: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
        const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
        format!("{}, {} {day}, {year}, {time}", DAYS[index(weekday, 7)], MONTHS[index(month, 12)])
    } else {
        const DAYS: [&str; 7] = ["segunda-feira", "terça-feira", "quarta-feira", "quinta-feira", "sexta-feira", "sábado", "domingo"];
        const MONTHS: [&str; 12] =
            ["janeiro", "fevereiro", "março", "abril", "maio", "junho", "julho", "agosto", "setembro", "outubro", "novembro", "dezembro"];
        format!("{}, {day} de {} de {year}, {time}", DAYS[index(weekday, 7)], MONTHS[index(month, 12)])
    }
}

/// Where the first whole sentence of `text` ends (after its mark and the
/// space after it), if one is: a number's point ("1.2") has no space
/// after it.
fn sentence_end(text: &str) -> Option<usize> {
    let mut chars = text.char_indices().peekable();
    while let Some((_, c)) = chars.next() {
        if matches!(c, '.' | '!' | '?' | '\n')
            && let Some(&(j, next)) = chars.peek()
            && next.is_whitespace()
        {
            return Some(j + next.len_utf8());
        }
    }
    None
}

/// A sentence as it should be said: no emoji, no surrounding space.
fn clean(text: &str) -> String {
    text.chars()
        .filter(|&c| !matches!(c, '\u{1F000}'..='\u{1FAFF}' | '\u{2600}'..='\u{27BF}' | '\u{FE0F}' | '\u{200D}'))
        .collect::<String>()
        .trim()
        .to_owned()
}

/// `request` as the conversation keeps it, with its note.
pub fn asked(request: &str, language: &str) -> Turn {
    Turn { user: true, text: with_note(request, language) }
}

/// The conversation, kept to its last `KEPT` turns.
pub fn keep(talk: &mut Vec<Turn>, asked: Turn, answered: String) {
    talk.push(asked);
    talk.push(Turn { user: false, text: answered });
    if talk.len() > KEPT {
        let extra = talk.len() - KEPT;
        talk.drain(..extra);
    }
}

impl config::Voice {
    /// `model` as a file: a path, or a name in the models' folder.
    pub fn model_path(&self) -> PathBuf {
        if self.model.contains('/') {
            config::expand(std::path::Path::new(&self.model))
        } else {
            crate::stt::data_dir().join("models").join(format!("{}.gguf", self.model))
        }
    }
}

/// Runs the voice alone on `request`, as the daemon would, with its times:
/// `orbit-aigent voice [--en] <request>` (Whisper gives the language in the
/// daemon; here it is Portuguese unless told).
pub fn try_alone(request: &str) -> i32 {
    let (language, request) = match request.strip_prefix("--en ") {
        Some(rest) => ("en", rest),
        None => ("pt", request),
    };
    let config = config::Config::load();
    let Some(voice) = config.voice.clone() else {
        eprintln!("orbit-aigent: no `voice:` in aigent.yaml");
        return 1;
    };
    let request = request.to_owned();
    let result = run(&voice, move |local| {
        let start = Instant::now();
        let route = local.route(&request);
        println!("route {:?} in {:.2} s", route, start.elapsed().as_secs_f32());
        if let Ok(Route::Talk) = route {
            let start = Instant::now();
            let answer = local.answer(&[], &request, language, |sentence| {
                println!("  {:.2} s  {sentence}", start.elapsed().as_secs_f32());
                true
            });
            if let Err(err) = answer {
                eprintln!("{err:#}");
            }
        }
        std::process::exit(0);
    });
    if let Err(err) = result {
        eprintln!("orbit-aigent: {err:#}");
    }
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sentences_end_at_a_mark_and_a_space() {
        assert_eq!(sentence_end("Bom dia. Tudo"), Some(9));
        assert_eq!(sentence_end("Versão 1.2 saiu"), None);
        assert_eq!(sentence_end("Pronto!"), None);
    }

    #[test]
    fn emoji_are_not_said() {
        assert_eq!(clean(" Tudo bem! 😊 "), "Tudo bem!");
    }

    #[test]
    fn the_conversation_keeps_its_last_turns() {
        let mut talk = Vec::new();
        for i in 0..20 {
            keep(&mut talk, Turn { user: true, text: format!("q{i}") }, format!("a{i}"));
        }
        assert_eq!(talk.len(), KEPT);
        assert_eq!(talk[0].text, "q12");
    }
}
