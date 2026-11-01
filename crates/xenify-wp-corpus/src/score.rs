//! Scoring the checker against the corpus.

use crate::read_items;
use serde_json::json;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::ops::Range;
use std::path::Path;
use xenify_wp_check::{Category, Checker, Ignores, Issue, Settings};
use xenify_wp_doc::NodeId;

/// Counts for one row of the scoreboard.
#[derive(Default, Clone)]
struct Tally {
    n: usize,
    caught: usize,
    fixed: usize,
    misses: Vec<String>,
}

#[derive(Default)]
struct RuleStats {
    hits: usize,
    false_alarms: usize,
    examples: Vec<String>,
    /// How often each marked word was a false alarm.
    words: BTreeMap<String, usize>,
}

/// Grammar/spelling issues are what a reader calls "errors"; style and
/// readability notes are counted separately.
fn is_error(i: &Issue) -> bool {
    matches!(i.category, Category::Spelling | Category::Grammar)
}

fn overlaps(a: &Range<usize>, b: &Range<usize>) -> bool {
    // A one-byte margin: missing-comma and missing-word issues often mark
    // the neighbouring word or the gap.
    a.start <= b.end + 1 && b.start <= a.end + 1
}

fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn pct(a: usize, b: usize) -> String {
    if b == 0 { "–".into() } else { format!("{:.0}%", 100.0 * a as f64 / b as f64) }
}

fn mark(text: &str, r: &Range<usize>) -> String {
    let r = r.start.min(text.len())..r.end.min(text.len());
    format!("{}⟦{}⟧{}", &text[..r.start], &text[r.clone()], &text[r.end..])
}

/// Spotter findings by passage text: merged runs of tokens above the threshold, as issues.
fn spotter_issues(path: &Path, threshold: f64) -> Result<std::collections::HashMap<String, Vec<Issue>>, String> {
    let mut out = std::collections::HashMap::new();
    for line in std::fs::read_to_string(path).map_err(|e| e.to_string())?.lines() {
        let v: serde_json::Value = serde_json::from_str(line).map_err(|e| e.to_string())?;
        let text = v["text"].as_str().unwrap_or_default().to_string();
        // The model gives char offsets; turn them into byte offsets.
        let bytes: Vec<usize> = text.char_indices().map(|(i, _)| i).chain([text.len()]).collect();
        let mut issues: Vec<Issue> = Vec::new();
        for f in v["found"].as_array().into_iter().flatten() {
            let (s, e, p, code) = (
                f[0].as_u64().unwrap_or(0) as usize,
                f[1].as_u64().unwrap_or(0) as usize,
                f[2].as_f64().unwrap_or(0.0),
                f[3].as_str().unwrap_or(""),
            );
            if p < threshold {
                continue;
            }
            let (s, e) = (bytes[s.min(bytes.len() - 1)], bytes[e.min(bytes.len() - 1)]);
            match issues.last_mut() {
                Some(last) if s <= last.range.end + 1 => last.range.end = last.range.end.max(e),
                _ => issues.push(Issue {
                    para: NodeId(0),
                    range: s..e,
                    category: Category::Grammar,
                    rule: format!("Spotter:{code}"),
                    message: String::new(),
                    suggestions: Vec::new(),
                    feature: None,
                    strong: false,
                }),
            }
        }
        out.insert(text, issues);
    }
    Ok(out)
}

#[allow(clippy::too_many_arguments)] // a dev tool's options, passed straight through
pub fn score(
    input: &Path,
    out: &Path,
    harper_only: bool,
    profile: &str,
    spotter: Option<(&Path, f64)>,
    spotter_model: Option<(&Path, f64)>,
    using: &str,
    dialect: xenify_wp_check::Dialect,
) -> Result<(), String> {
    let items = read_items(input)?;
    if items.is_empty() {
        return Err(format!("{}: no items", input.display()));
    }
    let settings = if harper_only {
        Settings {
            disabled_rules: xenify_wp_check::own_rules().into_iter().map(str::to_string).collect(),
            ignore_names: false,
            dialect,
            ..Settings::default()
        }
    } else {
        Settings { profile: xenify_wp_check::Profiles::default().get(profile), dialect, ..Settings::default() }
    };
    let mut checker = Checker::new(settings, &[]);
    let ignore = Ignores::default();
    let spotted = match spotter {
        Some((p, t)) => spotter_issues(p, t)?,
        None => Default::default(),
    };
    let model = match spotter_model {
        Some((p, t)) => Some((xenify_grammar_spotter::Spotter::load(p)?, t as f32)),
        None => None,
    };
    let optional_commas = checker.settings().profile.optional_commas;
    let mut run = xenify_wp_check::SpotterRun::default();
    let (use_rules, use_spotter) = match using {
        "spotter" => (false, true),
        "both" => (true, true),
        _ => (true, false),
    };
    let mut check = |text: &str| {
        let mut v = if use_rules { checker.check_paragraph(NodeId(0), text, &ignore) } else { Vec::new() };
        if use_spotter {
            v.extend(spotted.get(text).cloned().unwrap_or_default());
            if let Some((m, t)) = &model {
                // As Write does it: where no rule marked the words, with tested suggestions.
                let found = run.paragraph(
                    m,
                    xenify_wp_check::SpotOptions {
                        threshold: *t,
                        optional_commas,
                        sentence_sense: std::env::var_os("XENIFY_SENTENCE_SENSE").is_some(),
                    },
                    NodeId(0),
                    text,
                    &v,
                    &ignore,
                );
                v.extend(found);
            }
        }
        v
    };

    let mut by_type: BTreeMap<String, Tally> = BTreeMap::new();
    let mut by_setting: BTreeMap<String, Tally> = BTreeMap::new();
    let mut rules: BTreeMap<String, RuleStats> = BTreeMap::new();
    // Clean text: passages, passages with an error flag, style notes.
    let mut clean: BTreeMap<String, (usize, usize, usize, usize)> = BTreeMap::new();
    let mut register_rules: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    // Every error flag on correct standard text, for sorting false alarms into groups.
    let mut dump: Vec<serde_json::Value> = Vec::new();

    for it in &items {
        if let (Some(kind), Some(m)) = (&it.error_type, it.mistakes.first()) {
            let Some(start) = it.text.find(&m.wrong) else { continue };
            let span = start..start + m.wrong.len();
            let issues = check(&it.text);
            let hits: Vec<&Issue> = issues
                .iter()
                .filter(|i| {
                    !matches!(i.category, Category::Readability | Category::Register) && overlaps(&i.range, &span)
                })
                .collect();
            let fixed = hits.iter().any(|i| {
                i.suggestions.iter().any(|s| {
                    let mut t = it.text.clone();
                    t.replace_range(i.range.clone(), s);
                    it.corrected.as_deref().is_some_and(|c| squash(&t) == squash(c))
                })
            });
            for h in &hits {
                rules.entry(h.rule.clone()).or_default().hits += 1;
            }
            for t in [by_type.entry(kind.clone()).or_default(), by_setting.entry(it.setting.clone()).or_default()] {
                t.n += 1;
                if !hits.is_empty() {
                    t.caught += 1;
                }
                if fixed {
                    t.fixed += 1;
                }
            }
            if hits.is_empty() {
                let t = by_type.get_mut(kind).unwrap();
                {
                    t.misses.push(format!("{} → `{}`", mark(&it.text, &span), m.fix));
                }
            }
            // The corrected passage is clean standard text too.
            if let Some(c) = &it.corrected {
                tally_clean(
                    &check(c),
                    c,
                    "standard (corrected mistakes)",
                    &mut clean,
                    &mut rules,
                    &mut register_rules,
                    &mut dump,
                );
            }
        } else {
            let label =
                if it.register == "standard" { "standard (written clean)".to_string() } else { it.register.clone() };
            tally_clean(&check(&it.text), &it.text, &label, &mut clean, &mut rules, &mut register_rules, &mut dump);
        }
    }

    // ---- Markdown -------------------------------------------------------
    let mut md = String::new();
    let total = by_type.values().fold(Tally::default(), |mut a, t| {
        a.n += t.n;
        a.caught += t.caught;
        a.fixed += t.fixed;
        a
    });
    let std_clean = clean
        .iter()
        .filter(|(k, _)| k.starts_with("standard"))
        .fold((0, 0, 0), |a, (_, v)| (a.0 + v.0, a.1 + v.1, a.2 + v.2));
    let _ = profile;
    let _ = writeln!(md, "# Proofing scoreboard\n");
    let _ = writeln!(md, "Corpus: `{}` ({} passages).\n", input.display(), items.len());
    let _ = writeln!(md, "| | |\n|---|---|");
    let _ = writeln!(md, "| Mistakes caught | **{}** of {} ({}) |", total.caught, total.n, pct(total.caught, total.n));
    let _ = writeln!(md, "| …with the right fix offered | {} ({}) |", total.fixed, pct(total.fixed, total.n));
    let _ = writeln!(
        md,
        "| False alarms on correct standard text | {} of {} passages ({}) |",
        std_clean.1,
        std_clean.0,
        pct(std_clean.1, std_clean.0)
    );
    let _ =
        writeln!(md, "| Style notes on correct standard text | {} notes on {} passages |\n", std_clean.2, std_clean.0);

    let _ = writeln!(md, "## By kind of mistake\n\n| Kind | Passages | Caught | Right fix |\n|---|---:|---:|---:|");
    let mut types: Vec<_> = by_type.iter().collect();
    types.sort_by(|a, b| {
        (a.1.caught as f64 / a.1.n.max(1) as f64).total_cmp(&(b.1.caught as f64 / b.1.n.max(1) as f64))
    });
    for (k, t) in &types {
        let _ = writeln!(md, "| {k} | {} | {} | {} |", t.n, pct(t.caught, t.n), pct(t.fixed, t.n));
    }
    let _ = writeln!(md, "\n## By setting\n\n| Setting | Passages | Caught |\n|---|---:|---:|");
    for (k, t) in &by_setting {
        let _ = writeln!(md, "| {k} | {} | {} |", t.n, pct(t.caught, t.n));
    }
    let _ = writeln!(
        md,
        "\n## Clean text (should not be flagged as errors)\n\n| Register | Passages | With an error flag | Style notes | Tone notes | Most frequent rules |\n|---|---:|---:|---:|---:|---|"
    );
    for (k, (n, flagged, style, tone)) in &clean {
        let mut top: Vec<_> = register_rules.get(k).map(|m| m.iter().collect()).unwrap_or_default();
        top.sort_by(|a, b| b.1.cmp(a.1));
        let top: Vec<String> = top.iter().take(4).map(|(r, c)| format!("{r} ×{c}")).collect();
        let _ = writeln!(md, "| {k} | {n} | {} | {style} | {tone} | {} |", pct(*flagged, *n), top.join(", "));
    }
    let _ = writeln!(
        md,
        "\n## Rules\n\nHits are mistakes found; false alarms are error flags on correct standard text.\n\n| Rule | Hits | False alarms | Example false alarm |\n|---|---:|---:|---|"
    );
    let mut rl: Vec<_> = rules.iter().collect();
    rl.sort_by(|a, b| b.1.false_alarms.cmp(&a.1.false_alarms).then(b.1.hits.cmp(&a.1.hits)));
    for (k, r) in &rl {
        let ex = r.examples.first().map(|e| e.replace('|', "\\|")).unwrap_or_default();
        let _ = writeln!(md, "| {k} | {} | {} | {ex} |", r.hits, r.false_alarms);
    }
    let _ = writeln!(md, "\n## Sample misses\n");
    for (k, t) in &types {
        if t.misses.is_empty() {
            continue;
        }
        let _ = writeln!(md, "**{k}**\n");
        for m in t.misses.iter().take(6) {
            let _ = writeln!(md, "- {m}");
        }
        let _ = writeln!(md);
    }

    // ---- JSON -----------------------------------------------------------
    let j = json!({
        "passages": items.len(),
        "mistakes": total.n, "caught": total.caught, "fixed": total.fixed,
        "clean_standard": std_clean.0, "clean_standard_flagged": std_clean.1,
        "by_type": by_type.iter().map(|(k, t)| (k.clone(), json!({"n": t.n, "caught": t.caught, "fixed": t.fixed}))).collect::<serde_json::Map<_, _>>(),
        "clean": clean.iter().map(|(k, v)| (k.clone(), json!({"n": v.0, "flagged": v.1, "style": v.2, "tone": v.3}))).collect::<serde_json::Map<_, _>>(),
        "rules": rules.iter().map(|(k, r)| (k.clone(), json!({"hits": r.hits, "false_alarms": r.false_alarms}))).collect::<serde_json::Map<_, _>>(),
    });
    // Every recorded miss, for choosing the next rules.
    let mut misses = String::from("# Misses\n");
    for (k, t) in &types {
        let _ = writeln!(misses, "\n## {k}\n");
        for m in &t.misses {
            let _ = writeln!(misses, "- {m}");
        }
    }
    std::fs::write(out.with_extension("misses.md"), misses).map_err(|e| e.to_string())?;
    // The words each rule wrongly marked, most frequent first.
    let mut fa = String::from("# False alarms by rule\n");
    for (k, r) in rl.iter().filter(|(_, r)| r.false_alarms > 0) {
        let mut w: Vec<_> = r.words.iter().collect();
        w.sort_by(|a, b| b.1.cmp(a.1));
        let list: Vec<String> = w.iter().map(|(t, c)| format!("`{t}` ×{c}")).collect();
        let _ = writeln!(fa, "\n## {k} ({})\n\n{}", r.false_alarms, list.join(", "));
    }
    std::fs::write(out.with_extension("false-alarms.md"), fa).map_err(|e| e.to_string())?;
    let lines: String = dump.iter().map(|v| format!("{v}\n")).collect();
    std::fs::write(out.with_extension("false-alarms.jsonl"), lines).map_err(|e| e.to_string())?;
    let md_path = out.with_extension("md");
    std::fs::write(&md_path, &md).map_err(|e| e.to_string())?;
    std::fs::write(out.with_extension("json"), serde_json::to_string_pretty(&j).unwrap()).map_err(|e| e.to_string())?;
    println!(
        "caught {}/{} ({}), right fix {}, false alarms on clean standard text {}/{} ({}) → {}",
        total.caught,
        total.n,
        pct(total.caught, total.n),
        pct(total.fixed, total.n),
        std_clean.1,
        std_clean.0,
        pct(std_clean.1, std_clean.0),
        md_path.display()
    );
    Ok(())
}

fn tally_clean(
    issues: &[Issue],
    text: &str,
    label: &str,
    clean: &mut BTreeMap<String, (usize, usize, usize, usize)>,
    rules: &mut BTreeMap<String, RuleStats>,
    register_rules: &mut BTreeMap<String, BTreeMap<String, usize>>,
    dump: &mut Vec<serde_json::Value>,
) {
    let e = clean.entry(label.to_string()).or_default();
    e.0 += 1;
    let errors: Vec<&Issue> = issues.iter().filter(|i| is_error(i)).collect();
    if !errors.is_empty() {
        e.1 += 1;
    }
    e.2 += issues.iter().filter(|i| i.category == Category::Style).count();
    e.3 += issues.iter().filter(|i| i.category == Category::Register).count();
    for i in &errors {
        *register_rules.entry(label.to_string()).or_default().entry(i.rule.clone()).or_default() += 1;
        if label.starts_with("standard") {
            let r = rules.entry(i.rule.clone()).or_default();
            r.false_alarms += 1;
            dump.push(json!({"text": text, "rule": i.rule, "flagged": &text[i.range.clone()], "start": i.range.start, "end": i.range.end}));
            *r.words.entry(text[i.range.clone()].to_string()).or_default() += 1;
            if r.examples.len() < 3 {
                r.examples.push(mark(text, &i.range));
            }
        }
    }
}
