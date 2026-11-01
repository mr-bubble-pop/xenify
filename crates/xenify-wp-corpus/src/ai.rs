//! Generating and verifying corpus items through OpenRouter.

use crate::taxonomy::{ERROR_TYPES, REGISTERS, SETTINGS, error_type, register, setting};
use crate::{FeatureUse, Item, Mistake, append_items, read_items};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use xenify_openrouter::{AiSettings, OpenRouter};

pub struct GenOptions {
    pub out: PathBuf,
    pub model: String,
    pub batch: usize,
    pub rounds: usize,
    pub first_round: usize,
    pub only: Option<Vec<String>>,
    pub clean: bool,
    pub budget: f64,
    pub threads: usize,
}

/// One generation request.
#[derive(Clone)]
enum Job {
    Mistakes { setting: &'static str, error: &'static str, voice: &'static str, round: usize },
    Clean { setting: &'static str, register: &'static str, round: usize },
}

impl Job {
    fn key(&self) -> String {
        match self {
            Job::Mistakes { setting, error, voice, round } => format!("err:{setting}:{error}:{voice}:{round}"),
            Job::Clean { setting, register, round } => format!("clean:{setting}:{register}:{round}"),
        }
    }
}

pub(crate) fn client() -> Result<OpenRouter, String> {
    OpenRouter::new(&AiSettings::load()).map_err(|e| e.to_string())
}

/// Calls `tool` with `system`/`user` messages and returns its arguments and the cost.
pub(crate) fn call(
    c: &OpenRouter,
    model: &str,
    system: &str,
    user: &str,
    tool: Value,
    temperature: f64,
) -> Result<(Value, f64), String> {
    let name = tool["function"]["name"].as_str().unwrap_or_default().to_string();
    let body = json!({
        "model": model,
        "messages": [{"role": "system", "content": system}, {"role": "user", "content": user}],
        "tools": [tool],
        "tool_choice": {"type": "function", "function": {"name": name}},
        "temperature": temperature,
        "usage": {"include": true},
    });
    // Some providers refuse a forced tool call; then let the model choose (it still answers with the tool or JSON).
    let resp = match c.chat(&body) {
        Err(xenify_openrouter::AiError::Api { status: 400 | 404, .. }) => {
            let mut auto = body.clone();
            auto["tool_choice"] = json!("auto");
            match c.chat(&auto) {
                Ok(r) => r,
                // No tool support at all: ask for the same JSON in plain text.
                Err(_) => {
                    let mut plain = body.clone();
                    let schema = plain["tools"][0]["function"]["parameters"].clone();
                    plain.as_object_mut().map(|o| {
                        o.remove("tools");
                        o.remove("tool_choice")
                    });
                    let note =
                        format!("\n\nReply with only a JSON object matching this schema, no other text:\n{schema}");
                    if let Some(m) = plain["messages"].as_array_mut().and_then(|m| m.last_mut()) {
                        let text = format!("{}{note}", m["content"].as_str().unwrap_or_default());
                        m["content"] = json!(text);
                    }
                    c.chat(&plain).map_err(|e| e.to_string())?
                }
            }
        }
        r => r.map_err(|e| e.to_string())?,
    };
    let cost = resp["usage"]["cost"].as_f64().unwrap_or(0.0);
    let args = tool_args(&resp, &name).ok_or_else(|| format!("no {name} call in the reply"))?;
    Ok((args, cost))
}

/// The arguments of the first call to `tool` (or JSON in the message text).
fn tool_args(resp: &Value, tool: &str) -> Option<Value> {
    let msg = &resp["choices"][0]["message"];
    if let Some(calls) = msg["tool_calls"].as_array() {
        for c in calls {
            if c["function"]["name"].as_str() == Some(tool) {
                return match &c["function"]["arguments"] {
                    Value::String(s) => serde_json::from_str(s).ok(),
                    v => Some(v.clone()),
                };
            }
        }
    }
    let text = msg["content"].as_str()?;
    serde_json::from_str(&text[text.find('{')?..=text.rfind('}')?]).ok()
}

/// Runs `work` over `jobs` on `threads` threads until the budget is spent.
/// `work` returns its cost; failures are reported and skipped.
pub(crate) fn run_pool<J: Send + Clone>(
    jobs: Vec<J>,
    threads: usize,
    budget: f64,
    label: impl Fn(&J) -> String + Sync,
    work: impl Fn(&J) -> Result<f64, String> + Sync,
) -> f64 {
    let total = jobs.len();
    let queue = Mutex::new(jobs.into_iter().enumerate().collect::<Vec<_>>());
    let spent = Mutex::new(0.0f64);
    let done = Mutex::new(0usize);
    std::thread::scope(|s| {
        for _ in 0..threads.max(1) {
            s.spawn(|| {
                loop {
                    if *spent.lock().unwrap() >= budget {
                        return;
                    }
                    let Some((_, job)) = queue.lock().unwrap().pop() else { return };
                    let r = work(&job);
                    let mut n = done.lock().unwrap();
                    *n += 1;
                    match r {
                        Ok(cost) => {
                            let mut sp = spent.lock().unwrap();
                            *sp += cost;
                            eprintln!("[{}/{total}] {} (${:.4} total)", *n, label(&job), *sp);
                        }
                        Err(e) => eprintln!("[{}/{total}] {} failed: {e}", *n, label(&job)),
                    }
                }
            });
        }
    });
    let spent = *spent.lock().unwrap();
    if spent >= budget {
        eprintln!("budget of ${budget:.2} reached; stopped early (rerun to continue)");
    }
    spent
}

const GEN_SYSTEM: &str = "You write realistic test data for an English grammar checker. Follow the instructions exactly and vary topics, names, places and sentence shapes widely. Never copy the example given.";

fn mistakes_tool() -> Value {
    json!({"type": "function", "function": {
        "name": "report_passages",
        "description": "Return the passages.",
        "parameters": {"type": "object", "required": ["passages"], "properties": {"passages": {"type": "array", "items": {
            "type": "object", "required": ["text", "wrong", "fix", "corrected"],
            "properties": {
                "text": {"type": "string", "description": "The passage, containing exactly one mistake"},
                "wrong": {"type": "string", "description": "The wrong words copied exactly from the passage (as few words as possible, but enough to be unique)"},
                "fix": {"type": "string", "description": "What those words should be"},
                "corrected": {"type": "string", "description": "The whole passage with only that mistake fixed"}
            }}}}}
    }})
}

fn clean_tool() -> Value {
    let regs: Vec<&str> = REGISTERS.iter().map(|r| r.id).filter(|r| *r != "standard").collect();
    json!({"type": "function", "function": {
        "name": "report_passages",
        "description": "Return the passages.",
        "parameters": {"type": "object", "required": ["passages"], "properties": {"passages": {"type": "array", "items": {
            "type": "object", "required": ["text", "features"],
            "properties": {
                "text": {"type": "string"},
                "features": {"type": "array", "description": "Every informal or non-standard feature used on purpose (empty for standard English)", "items": {
                    "type": "object", "required": ["quote", "register"],
                    "properties": {
                        "quote": {"type": "string", "description": "The words copied exactly from the passage"},
                        "register": {"type": "string", "enum": regs},
                        "standard": {"type": "string", "description": "The standard written English equivalent"}
                    }}}
            }}}}}
    }})
}

fn job_prompt(job: &Job, batch: usize) -> String {
    match job {
        Job::Mistakes { setting: s, error: e, voice, .. } => {
            let (s, e) = (setting(s).unwrap(), error_type(e).unwrap());
            let who = if *voice == "learner" {
                "a fluent but non-native English writer"
            } else {
                "a native English speaker writing quickly"
            };
            format!(
                "Write {batch} different short passages (1 to 3 sentences each) from {}, written by {who}.\n\
                 Each passage must contain exactly ONE mistake of this kind: {}.\n\
                 Example of the kind (do not reuse it): \"{}\" should be \"{}\".\n\
                 Apart from that one mistake, each passage must be correct standard written English, with normal punctuation and capitals. \
                 Use a different specific word or construction in each passage. Make the mistakes the kind real people actually make.",
                s.describe, e.describe, e.example.0, e.example.1
            )
        }
        Job::Clean { setting: s, register: r, .. } => {
            let (s, r) = (setting(s).unwrap(), register(r).unwrap());
            let rule = if r.id == "standard" {
                "They must be flawless standard written English: no spelling, grammar or punctuation mistakes. Use a natural variety of sentence structures, including some long ones, quotations, numbers, names and lists.".to_string()
            } else {
                format!(
                    "Write them in this register: {}. The register's features are deliberate and must be consistent and authentic, the way a real fluent user of it writes. \
                     Do NOT include typos or mistakes that a user of the register would also consider mistakes. List every deliberate feature you used.",
                    r.describe
                )
            };
            format!("Write {batch} different short passages (1 to 3 sentences each) from {}.\n{rule}", s.describe)
        }
    }
}

fn all_jobs(o: &GenOptions) -> Vec<Job> {
    let mut jobs = Vec::new();
    for round in o.first_round..o.first_round + o.rounds {
        for e in ERROR_TYPES {
            if o.only.as_ref().is_some_and(|only| !only.iter().any(|x| x == e.id)) {
                continue;
            }
            for (i, s) in SETTINGS.iter().enumerate() {
                if !e.only.is_empty() && !e.only.contains(&s.id) {
                    continue;
                }
                // Alternate voices across settings so both are covered.
                let voice = if (i + round) % 3 == 2 { "learner" } else { "native" };
                jobs.push(Job::Mistakes { setting: s.id, error: e.id, voice, round });
            }
        }
        if o.clean && o.only.is_none() {
            for s in SETTINGS {
                jobs.push(Job::Clean { setting: s.id, register: "standard", round });
                for r in REGISTERS.iter().filter(|r| r.id != "standard") {
                    // Informal registers where they're natural, plus a few in
                    // formal settings (casual email, emoji in a work post…).
                    let formal_ok = matches!(
                        r.id,
                        "contractions_first_person" | "emoji_exclaim" | "hedging_filler" | "texting" | "slang"
                    );
                    if s.informal || formal_ok {
                        jobs.push(Job::Clean { setting: s.id, register: r.id, round });
                    }
                }
            }
        }
    }
    jobs
}

pub fn generate(o: &GenOptions) -> Result<(), String> {
    let have: HashSet<String> = read_items(&o.out)?.into_iter().map(|i| i.job).collect();
    let jobs: Vec<Job> = all_jobs(o).into_iter().filter(|j| !have.contains(&j.key())).collect();
    eprintln!("{} requests to make ({} already done)", jobs.len(), have.len());
    let c = client()?;
    let file = Mutex::new(());
    let spent = run_pool(
        jobs,
        o.threads,
        o.budget,
        |j| j.key(),
        |job| {
            let tool = if matches!(job, Job::Mistakes { .. }) { mistakes_tool() } else { clean_tool() };
            let (args, cost) = call(&c, &o.model, GEN_SYSTEM, &job_prompt(job, o.batch), tool, 0.9)?;
            let items = parse_passages(job, &args, &o.model);
            if items.is_empty() {
                return Err("no usable passages".into());
            }
            let _g = file.lock().unwrap();
            append_items(&o.out, &items)?;
            Ok(cost)
        },
    );
    eprintln!("spent ${spent:.4}");
    Ok(())
}

fn parse_passages(job: &Job, args: &Value, model: &str) -> Vec<Item> {
    let key = job.key();
    let empty = Vec::new();
    let passages = args["passages"].as_array().unwrap_or(&empty);
    let mut out = Vec::new();
    for (i, p) in passages.iter().enumerate() {
        let text = p["text"].as_str().unwrap_or_default().trim().to_string();
        if text.is_empty() {
            continue;
        }
        let mut item = Item {
            id: format!("{key}#{i}"),
            job: key.clone(),
            setting: String::new(),
            register: "standard".into(),
            error_type: None,
            voice: None,
            text,
            corrected: None,
            mistakes: Vec::new(),
            features: Vec::new(),
            generator: model.to_string(),
            verified_by: None,
        };
        match job {
            Job::Mistakes { setting, error, voice, .. } => {
                let wrong = p["wrong"].as_str().unwrap_or_default().to_string();
                let fix = p["fix"].as_str().unwrap_or_default().to_string();
                let corrected = p["corrected"].as_str().unwrap_or_default().trim().to_string();
                // Mechanical checks: the quote is in the text and the fix changes it.
                if wrong.is_empty() || !item.text.contains(&wrong) || corrected.is_empty() || corrected == item.text {
                    continue;
                }
                item.setting = setting.to_string();
                item.error_type = Some(error.to_string());
                item.voice = Some(voice.to_string());
                item.corrected = Some(corrected);
                item.mistakes = vec![Mistake { wrong, fix }];
            }
            Job::Clean { setting, register, .. } => {
                item.setting = setting.to_string();
                item.register = register.to_string();
                for f in p["features"].as_array().unwrap_or(&empty) {
                    let quote = f["quote"].as_str().unwrap_or_default().to_string();
                    if quote.is_empty() || !item.text.contains(&quote) {
                        continue;
                    }
                    item.features.push(FeatureUse {
                        quote,
                        register: f["register"].as_str().unwrap_or(register).to_string(),
                        standard: f["standard"].as_str().map(str::to_string),
                    });
                }
            }
        }
        out.push(item);
    }
    out
}

const VERIFY_SYSTEM: &str = "You are an expert English editor checking test data for a grammar checker. Judge strictly but fairly: \
an item passes only if it is exactly as described. Informal registers and dialects (slang, texting, African American English, regional dialects) \
follow their own consistent rules and are not mistakes in themselves.";

fn verdict_tool() -> Value {
    json!({"type": "function", "function": {
        "name": "report_verdicts",
        "description": "One verdict per item, in order.",
        "parameters": {"type": "object", "required": ["verdicts"], "properties": {"verdicts": {"type": "array", "items": {
            "type": "object", "required": ["n", "ok"],
            "properties": {
                "n": {"type": "integer", "description": "The item number"},
                "ok": {"type": "boolean"},
                "reason": {"type": "string", "description": "If not ok: why, briefly"}
            }}}}}
    }})
}

fn verify_prompt(items: &[Item]) -> String {
    let mut s = String::new();
    let mistakes = items[0].error_type.is_some();
    if mistakes {
        s.push_str(
            "Each item has a passage that is supposed to contain exactly one mistake of the stated kind, the wrong words, the fix, and the corrected passage.\n\
             An item is ok only if ALL hold: (1) the wrong words really are a mistake in standard written English for that setting (not an acceptable variant or style choice); \
             (2) the mistake is of the stated kind; (3) the corrected passage is flawless standard written English; \
             (4) the passage has no other mistakes besides that one.\n\n",
        );
    } else {
        s.push_str(
            "Each item is a passage that is supposed to have no mistakes judged by the rules of its stated register. \
             For 'standard', it must be flawless standard written English. For an informal register, deliberate features of that register \
             (as a fluent user would write them) are fine, but typos and genuine slips are not. An item is ok only if it has no mistakes.\n\n",
        );
    }
    for (n, it) in items.iter().enumerate() {
        s.push_str(&format!("Item {n} (setting: {}", it.setting));
        if let Some(e) = &it.error_type {
            let d = error_type(e).map(|e| e.describe).unwrap_or(e);
            s.push_str(&format!("; kind of mistake: {d})\nPassage: {}\n", it.text));
            if let Some(m) = it.mistakes.first() {
                s.push_str(&format!("Wrong words: \"{}\" → fix: \"{}\"\n", m.wrong, m.fix));
            }
            s.push_str(&format!("Corrected: {}\n\n", it.corrected.as_deref().unwrap_or("")));
        } else {
            let d = register(&it.register).map(|r| r.describe).unwrap_or(&it.register);
            s.push_str(&format!("; register: {d})\nPassage: {}\n\n", it.text));
        }
    }
    s
}

pub fn verify(input: &Path, out: &Path, model: &str, budget: f64, threads: usize) -> Result<(), String> {
    let items = read_items(input)?;
    let done: HashSet<String> = read_items(out)?.into_iter().map(|i| i.id).collect();
    let rejected_path = out.with_extension("rejected.jsonl");
    let rejected: HashSet<String> = read_items(&rejected_path)?.into_iter().map(|i| i.id).collect();
    // Batches of up to 10 from the same request (same kind of item).
    let mut batches: Vec<Vec<Item>> = Vec::new();
    for it in items.into_iter().filter(|i| !done.contains(&i.id) && !rejected.contains(&i.id)) {
        match batches.last_mut() {
            Some(b) if b.len() < 10 && b[0].job == it.job => b.push(it),
            _ => batches.push(vec![it]),
        }
    }
    eprintln!("{} batches to verify ({} already verified, {} rejected)", batches.len(), done.len(), rejected.len());
    let c = client()?;
    let file = Mutex::new((0usize, 0usize));
    let spent = run_pool(
        batches,
        threads,
        budget,
        |b| b[0].job.clone(),
        |batch| {
            let (args, cost) = call(&c, model, VERIFY_SYSTEM, &verify_prompt(batch), verdict_tool(), 0.0)?;
            let empty = Vec::new();
            let verdicts = args["verdicts"].as_array().unwrap_or(&empty);
            let (mut keep, mut drop) = (Vec::new(), Vec::new());
            for (n, it) in batch.iter().enumerate() {
                let v = verdicts.iter().find(|v| v["n"].as_u64() == Some(n as u64));
                let mut it = it.clone();
                it.verified_by = Some(model.to_string());
                match v {
                    Some(v) if v["ok"].as_bool() == Some(true) => keep.push(it),
                    Some(v) => {
                        // Keep the reason for inspection.
                        it.verified_by = Some(format!("{model}: {}", v["reason"].as_str().unwrap_or("rejected")));
                        drop.push(it)
                    }
                    None => {} // no verdict: retried on the next run
                }
            }
            let mut g = file.lock().unwrap();
            append_items(out, &keep)?;
            append_items(&rejected_path, &drop)?;
            g.0 += keep.len();
            g.1 += drop.len();
            Ok(cost)
        },
    );
    let g = file.lock().unwrap();
    eprintln!("kept {}, rejected {} (see {}); spent ${spent:.4}", g.0, g.1, rejected_path.display());
    Ok(())
}
