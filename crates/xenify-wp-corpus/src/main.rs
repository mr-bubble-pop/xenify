//! `xenify-wp-corpus`: builds the proofing test corpus with an AI model,
//! has a second model check it, and scores the checker against it.
//!
//! ```text
//! cargo run --release -p xenify-wp-corpus -- generate -o raw.jsonl
//! cargo run --release -p xenify-wp-corpus -- verify raw.jsonl -o verified.jsonl
//! cargo run --release -p xenify-wp-corpus -- score verified.jsonl -o scoreboard
//! ```
//!
//! Uses the OpenRouter key from Xenify's AI settings (or OPENROUTER_API_KEY).

mod ai;
mod bench;
mod breaker;
mod judge;
mod score;
mod sounds;
mod suspect;
mod suspect_model;
mod taxonomy;

use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Generator default: cheap, MIT-licensed weights, has a ZDR endpoint.
pub const GENERATOR: &str = "deepseek/deepseek-v4-flash";
/// Verifier default: a different, larger model (Apache-2.0, ZDR).
pub const VERIFIER: &str = "qwen/qwen3-235b-a22b-2507";

#[derive(Parser)]
#[command(name = "xenify-wp-corpus", about = "Proofing test corpus: generate, verify, score")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Generates passages with mistakes, and clean passages in many registers.
    /// Appends to the output; jobs already in it are skipped (resumable).
    Generate {
        #[arg(short, long)]
        out: PathBuf,
        #[arg(long, default_value = GENERATOR)]
        model: String,
        /// Passages per request.
        #[arg(long, default_value_t = 8)]
        batch: usize,
        /// Requests per (setting, mistake) pair.
        #[arg(long, default_value_t = 1)]
        rounds: usize,
        /// Number the rounds from here (rounds 0 and 1 are the dev and held-out sets).
        #[arg(long, default_value_t = 0)]
        first_round: usize,
        /// Only these mistake kinds (comma-separated ids); default all.
        #[arg(long)]
        only: Option<String>,
        /// Skip the clean passages.
        #[arg(long)]
        no_clean: bool,
        /// Stop starting requests once this many US dollars are spent.
        #[arg(long, default_value_t = 5.0)]
        budget: f64,
        #[arg(long, default_value_t = 8)]
        threads: usize,
    },
    /// Has a second model check every item; keeps the ones it confirms.
    Verify {
        input: PathBuf,
        #[arg(short, long)]
        out: PathBuf,
        #[arg(long, default_value = VERIFIER)]
        model: String,
        #[arg(long, default_value_t = 5.0)]
        budget: f64,
        #[arg(long, default_value_t = 8)]
        threads: usize,
    },
    /// Scores language models as proofreaders on the frozen test set (alone and with the rules).
    Bench {
        #[arg(default_value = "assets/proofing/test-set.jsonl")]
        input: PathBuf,
        /// Models to try (comma-separated OpenRouter ids).
        #[arg(long, value_delimiter = ',')]
        models: Vec<String>,
        #[arg(short, long, default_value = "data/proofing-corpus/bench.md")]
        out: PathBuf,
        /// Passages per request.
        #[arg(long, default_value_t = 10)]
        batch: usize,
        /// Budget per model (US dollars).
        #[arg(long, default_value_t = 3.0)]
        budget: f64,
        #[arg(long, default_value_t = 8)]
        threads: usize,
        /// Tell the model each passage may contain a mistake and ask for the corrected passage.
        #[arg(long)]
        hint: bool,
        /// Send whole documents of this many passages instead, the way Write's AI check does.
        #[arg(long)]
        doc: Option<usize>,
        /// How many documents (with --doc).
        #[arg(long, default_value_t = 10)]
        docs: usize,
        /// Any OpenAI-style address instead of OpenRouter, e.g. http://localhost:8080/v1 (with --doc).
        #[arg(long)]
        url: Option<String>,
        /// The environment variable holding the key for --url.
        #[arg(long)]
        key_env: Option<String>,
        /// Show examples from a saved --doc run's .replies.jsonl instead (asks no model).
        #[arg(long)]
        samples: Option<PathBuf>,
        /// Score every saved --doc run in this folder fairly, with judge models (writes judged.md and review.md there).
        #[arg(long)]
        judge: Option<PathBuf>,
        /// With --doc: send each document in parts of about this many words, all at once (as Write does).
        #[arg(long)]
        parts: Option<usize>,
        /// With --doc: ask models not to think before answering.
        #[arg(long)]
        no_thinking: bool,
        /// The judges for --judge (two from different makers).
        #[arg(long, value_delimiter = ',', default_value = "qwen/qwen3-235b-a22b-2507,openai/gpt-oss-120b")]
        judges: Vec<String>,
    },
    /// The "something looks wrong" detector: train on correct sentences, evaluate on the corpus.
    Suspect {
        #[command(subcommand)]
        action: SuspectAction,
    },
    /// Sound-alike words: generate example sentences, verify, train, evaluate.
    Sounds {
        #[command(subcommand)]
        action: SoundsAction,
    },
    /// Damages correct passages in known ways (missing words, swapped words,
    /// wrong verb forms…), so the mistake and fix are known without a model.
    Break {
        /// Corpus files to take correct passages from.
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        #[arg(short, long)]
        out: PathBuf,
        /// Different breakages per passage.
        #[arg(long, default_value_t = 2)]
        per_source: usize,
        #[arg(long, default_value_t = 1)]
        seed: u64,
    },
    /// Runs the checker over the corpus and writes a scoreboard (.md and .json).
    Score {
        input: PathBuf,
        /// Output path without extension.
        #[arg(short, long)]
        out: PathBuf,
        /// Score Harper alone (Xenify's own rules and filters off), for comparison.
        #[arg(long)]
        harper_only: bool,
        /// Writing profile to score with (id); default standard.
        #[arg(long, default_value = "standard")]
        profile: String,
        /// Spotter findings (from train/spotter_predict.py) to score too.
        #[arg(long)]
        spotter: Option<PathBuf>,
        /// Or a spotter model folder, run through the Rust crate.
        #[arg(long)]
        spotter_model: Option<PathBuf>,
        #[arg(long, default_value_t = xenify_wp_check::DEFAULT_THRESHOLD as f64)]
        spotter_threshold: f64,
        /// What to score: rules, spotter, or both.
        #[arg(long, default_value = "rules")]
        using: String,
        /// Spelling dialect: us, uk, au, ca, in.
        #[arg(long, default_value = "us")]
        dialect: String,
    },
}

#[derive(Subcommand)]
enum SuspectAction {
    /// Learns sequence counts from correct sentences (sound-alike example files).
    Train {
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        #[arg(short, long)]
        out: PathBuf,
        #[arg(long, default_value_t = 1.0)]
        min: f64,
        /// Keep this many of the most common words as words.
        #[arg(long, default_value_t = 0)]
        top: usize,
    },
    /// Flag rates on a corpus at several thresholds.
    Eval {
        corpus: PathBuf,
        #[arg(long)]
        model: PathBuf,
        #[arg(long, value_delimiter = ',', default_values_t = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0])]
        thresholds: Vec<f64>,
    },
}

#[derive(Subcommand)]
enum SoundsAction {
    /// Correct sentences for every word of every confusable set.
    Generate {
        #[arg(short, long)]
        out: PathBuf,
        #[arg(long, default_value = GENERATOR)]
        model: String,
        #[arg(long, default_value_t = 60)]
        per_word: usize,
        #[arg(long, default_value_t = 20)]
        batch: usize,
        #[arg(long, default_value_t = 2.0)]
        budget: f64,
        #[arg(long, default_value_t = 12)]
        threads: usize,
    },
    /// A second model checks each sentence uses its word correctly.
    Verify {
        input: PathBuf,
        #[arg(short, long)]
        out: PathBuf,
        #[arg(long, default_value = VERIFIER)]
        model: String,
        #[arg(long, default_value_t = 2.0)]
        budget: f64,
        #[arg(long, default_value_t = 12)]
        threads: usize,
    },
    /// Learns context counts from 4/5 of the verified sentences.
    Train {
        input: PathBuf,
        #[arg(short, long)]
        out: PathBuf,
        #[arg(long, default_value_t = 2)]
        min_count: u32,
    },
    /// Scores a model on the held-out fifth (and on swapped copies of it).
    Eval {
        input: PathBuf,
        #[arg(long)]
        model: PathBuf,
        #[arg(long, value_delimiter = ',', default_values_t = [2.0, 4.0, 6.0, 8.0, 10.0])]
        margins: Vec<f64>,
        #[arg(long, default_value_t = 3)]
        support: usize,
    },
}

/// One mistake in a passage.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Mistake {
    /// The wrong words, exactly as in the passage.
    pub wrong: String,
    pub fix: String,
}

/// An informal feature used on purpose in a clean passage.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FeatureUse {
    pub quote: String,
    pub register: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub standard: Option<String>,
}

/// One corpus passage.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Item {
    pub id: String,
    /// The request it came from (for resuming).
    pub job: String,
    pub setting: String,
    /// "standard" or an informal register id.
    pub register: String,
    /// The kind of mistake (None: a clean passage).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_type: Option<String>,
    /// "native" (typing quickly) or "learner".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice: Option<String>,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corrected: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mistakes: Vec<Mistake>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub features: Vec<FeatureUse>,
    pub generator: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified_by: Option<String>,
}

pub fn read_items(path: &Path) -> Result<Vec<Item>, String> {
    let f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let mut out = Vec::new();
    for (n, line) in std::io::BufReader::new(f).lines().enumerate() {
        let line = line.map_err(|e| e.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        out.push(serde_json::from_str(&line).map_err(|e| format!("{}:{}: {e}", path.display(), n + 1))?);
    }
    Ok(out)
}

/// Appends items as JSON lines.
pub fn append_items(path: &Path, items: &[Item]) -> Result<(), String> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path).map_err(|e| e.to_string())?;
    for it in items {
        writeln!(f, "{}", serde_json::to_string(it).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn main() -> ExitCode {
    let r = match Cli::parse().command {
        Command::Generate { out, model, batch, rounds, first_round, only, no_clean, budget, threads } => {
            let only: Option<Vec<String>> = only.map(|s| s.split(',').map(|x| x.trim().to_string()).collect());
            ai::generate(&ai::GenOptions {
                out,
                model,
                batch,
                rounds,
                first_round,
                only,
                clean: !no_clean,
                budget,
                threads,
            })
        }
        Command::Verify { input, out, model, budget, threads } => ai::verify(&input, &out, &model, budget, threads),
        Command::Bench {
            input,
            models,
            out,
            batch,
            budget,
            threads,
            hint,
            doc,
            docs,
            url,
            key_env,
            samples,
            judge,
            judges,
            parts,
            no_thinking,
        } => {
            if let Some(dir) = judge {
                judge::rescore(&input, &dir, doc.unwrap_or(40), docs, &judges, budget)
            } else if let Some(replies) = samples {
                bench::samples(&input, &replies, doc.unwrap_or(40), docs)
            } else if let Some(size) = doc {
                bench::bench_doc(
                    &input,
                    &models,
                    &out,
                    size,
                    docs,
                    budget,
                    threads,
                    &bench::Endpoint { url, key_env, parts, no_thinking },
                )
            } else if hint {
                bench::bench_hint(&input, &models, &out, batch, budget, threads)
            } else {
                bench::bench(&input, &models, &out, batch, budget, threads)
            }
        }
        Command::Suspect { action } => match action {
            SuspectAction::Train { inputs, out, min, top } => suspect::train(&inputs, &out, min, top),
            SuspectAction::Eval { corpus, model, thresholds } => suspect::eval(&corpus, &model, &thresholds),
        },
        Command::Sounds { action } => match action {
            SoundsAction::Generate { out, model, per_word, batch, budget, threads } => {
                sounds::generate(&out, &model, per_word, batch, budget, threads)
            }
            SoundsAction::Verify { input, out, model, budget, threads } => {
                sounds::verify(&input, &out, &model, budget, threads)
            }
            SoundsAction::Train { input, out, min_count } => sounds::train(&input, &out, min_count),
            SoundsAction::Eval { input, model, margins, support } => sounds::eval(&input, &model, &margins, support),
        },
        Command::Break { inputs, out, per_source, seed } => breaker::run(&inputs, &out, per_source, seed),
        Command::Score {
            input,
            out,
            harper_only,
            profile,
            spotter,
            spotter_model,
            spotter_threshold,
            using,
            dialect,
        } => match xenify_wp_check::Dialect::parse(&dialect) {
            None => Err(format!("unknown dialect {dialect}")),
            Some(dialect) => score::score(
                &input,
                &out,
                harper_only,
                &profile,
                spotter.as_deref().map(|p| (p, spotter_threshold)),
                spotter_model.as_deref().map(|p| (p, spotter_threshold)),
                &using,
                dialect,
            ),
        },
    };
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("xenify-wp-corpus: {e}");
            ExitCode::FAILURE
        }
    }
}
