//! The error spotter: a small token-classification model (DeBERTa-v3-xsmall,
//! fine-tuned by `crates/xenify-wp-corpus/train/spotter.py`) that marks the
//! words of a passage that look wrong and names the kind of mistake with an
//! error code. It runs offline on the CPU with candle (about 25 ms a paragraph with the 6-layer model).
//!
//! It finds problems; it doesn't fix them. Fixes come from the rules, from
//! candidate words tested against it ([`Spotter::best_of`]), or from an
//! optional local language model.

use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::debertav2::{Config, DebertaV2NERModel};
use std::ops::Range;
use std::path::Path;
use tokenizers::Tokenizer;

/// Words that look wrong.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Finding {
    /// Byte range in the text.
    pub range: Range<usize>,
    /// The kind of mistake, e.g. `missing_word`, `article_missing`.
    pub code: String,
    /// How sure the model is that something's wrong here (0–1).
    pub confidence: f32,
}

pub struct Spotter {
    model: DebertaV2NERModel,
    bias: Tensor,
    tokenizer: Tokenizer,
    labels: Vec<String>,
    device: Device,
}

fn overlaps(a: &Range<usize>, b: &Range<usize>) -> bool {
    a.start < b.end && b.start < a.end
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

impl Spotter {
    /// Loads a model folder (`config.json`, `tokenizer.json` and
    /// `model.f16.safetensors` or `model.safetensors`).
    pub fn load(dir: &Path) -> Result<Self, String> {
        let device = Device::Cpu;
        let config: Config =
            serde_json::from_str(&std::fs::read_to_string(dir.join("config.json")).map_err(err)?).map_err(err)?;
        let weights = ["model.f16.safetensors", "model.safetensors"]
            .iter()
            .map(|f| dir.join(f))
            .find(|p| p.exists())
            .ok_or("no model weights")?;
        // Safety: the file is memory-mapped read-only and not modified while loaded.
        let vb = unsafe { VarBuilder::from_mmaped_safetensors(&[weights], DType::F32, &device).map_err(err)? };
        let tokenizer = Tokenizer::from_file(dir.join("tokenizer.json")).map_err(err)?;
        Self::build(config, vb, tokenizer, device)
    }

    /// Loads a model from the bytes of its files (where there are no
    /// files, as in the browser): `config.json`, `tokenizer.json` and the
    /// weights (`model.f16.safetensors`).
    pub fn from_bytes(config: &[u8], tokenizer: &[u8], weights: Vec<u8>) -> Result<Self, String> {
        let device = Device::Cpu;
        let config: Config = serde_json::from_slice(config).map_err(err)?;
        let vb = VarBuilder::from_buffered_safetensors(weights, DType::F32, &device).map_err(err)?;
        let tokenizer = Tokenizer::from_bytes(tokenizer).map_err(err)?;
        Self::build(config, vb, tokenizer, device)
    }

    fn build(config: Config, vb: VarBuilder, tokenizer: Tokenizer, device: Device) -> Result<Self, String> {
        let mut labels: Vec<(u32, String)> =
            config.id2label.clone().ok_or("config.json has no id2label")?.into_iter().collect();
        labels.sort();
        let labels: Vec<String> = labels.into_iter().map(|(_, l)| l).collect();
        let model = DebertaV2NERModel::load(vb.pp("deberta"), &config, None).map_err(err)?;
        // candle's classifier has no bias; ours does.
        let bias = vb.get(labels.len(), "classifier.bias").map_err(err)?;
        Ok(Self { model, bias, tokenizer, labels, device })
    }

    /// The error codes the model knows (without `OK`).
    pub fn codes(&self) -> &[String] {
        &self.labels[1..]
    }

    /// Per word-piece: (byte range, probability of a mistake, most likely
    /// code). Codes in `hidden` don't count: their probability goes to
    /// neither "mistake" nor any code, so the next most likely code shows.
    fn scores(&self, text: &str, hidden: &[String]) -> Result<Vec<(Range<usize>, f32, usize)>, String> {
        let hidden: Vec<usize> = (1..self.labels.len()).filter(|i| hidden.contains(&self.labels[*i])).collect();
        let enc = self.tokenizer.encode(text, true).map_err(err)?;
        let ids: Vec<u32> = enc.get_ids().iter().copied().take(512).collect();
        let input = Tensor::new(ids.as_slice(), &self.device).map_err(err)?.unsqueeze(0).map_err(err)?;
        let logits = self.model.forward(&input, None, None).map_err(err)?;
        let logits = logits.broadcast_add(&self.bias).map_err(err)?.squeeze(0).map_err(err)?;
        let probs = candle_nn::ops::softmax_last_dim(&logits).map_err(err)?.to_vec2::<f32>().map_err(err)?;
        let mut out = Vec::new();
        for (k, (s, e)) in enc.get_offsets().iter().take(ids.len()).enumerate() {
            if s == e || enc.get_special_tokens_mask()[k] == 1 {
                continue;
            }
            let p = &probs[k];
            let shown = || (1..p.len()).filter(|i| !hidden.contains(i));
            let best = shown().max_by(|a, b| p[*a].total_cmp(&p[*b])).unwrap_or(0);
            out.push((*s..*e, shown().map(|i| p[i]).sum(), best));
        }
        Ok(out)
    }

    /// The spots in `text` that look wrong with at least `threshold`
    /// confidence; neighbouring word-pieces are joined into one finding.
    pub fn spot(&self, text: &str, threshold: f32) -> Result<Vec<Finding>, String> {
        self.spot_hiding(text, threshold, &[])
    }

    /// Like [`Spotter::spot`], without the codes in `hidden` (the user
    /// turned those kinds off): where one was the best guess, the next best
    /// code shows if the model is sure enough of it.
    pub fn spot_hiding(&self, text: &str, threshold: f32, hidden: &[String]) -> Result<Vec<Finding>, String> {
        let mut out: Vec<Finding> = Vec::new();
        let mut best_p = 0.0f32;
        for (r, p, code) in self.scores(text, hidden)? {
            if p < threshold {
                continue;
            }
            // Leading spaces are part of SentencePiece pieces; start at the word.
            let lead = text[r.clone()].len() - text[r.clone()].trim_start().len();
            let r = r.start + lead..r.end;
            match out.last_mut() {
                Some(last) if r.start <= last.range.end + 1 => {
                    last.range.end = r.end;
                    if p > best_p {
                        last.code = self.labels[code].clone();
                        best_p = p;
                    }
                    last.confidence = last.confidence.max(p);
                }
                _ => {
                    best_p = p;
                    out.push(Finding { range: r, code: self.labels[code].clone(), confidence: p });
                }
            }
        }
        Ok(out)
    }

    /// How wrong `text` looks overall: the highest mistake probability.
    pub fn worst(&self, text: &str) -> Result<f32, String> {
        Ok(self.scores(text, &[])?.into_iter().map(|(_, p, _)| p).fold(0.0, f32::max))
    }

    /// Tries each replacement for `range` in `text` and returns those that
    /// make the passage look right, best first: (replacement, how wrong it
    /// still looks).
    pub fn best_of(
        &self,
        text: &str,
        range: Range<usize>,
        candidates: &[String],
    ) -> Result<Vec<(String, f32)>, String> {
        let mut ranked = Vec::new();
        for c in candidates {
            let mut t = text.to_string();
            t.replace_range(range.clone(), c);
            ranked.push((c.clone(), self.worst(&t)?));
        }
        ranked.sort_by(|a, b| a.1.total_cmp(&b.1));
        Ok(ranked)
    }
}

/// A suggested fix: replace `range` with `text`.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Fix {
    pub range: Range<usize>,
    pub text: String,
}

const PRONOUNS: &[(&str, &str)] =
    &[("I", "me"), ("he", "him"), ("she", "her"), ("we", "us"), ("they", "them"), ("who", "whom")];

/// Words in `range` of `text` with their byte ranges.
fn words_in(text: &str, range: &Range<usize>) -> Vec<(Range<usize>, String)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, c) in text[range.clone()].char_indices().chain([(range.len(), ' ')]) {
        let word_char = c.is_alphanumeric() || c == '\'' || c == '\u{2019}' || c == '-';
        match (start, word_char) {
            (None, true) => start = Some(i),
            (Some(s), false) => {
                out.push((range.start + s..range.start + i, text[range.start + s..range.start + i].to_string()));
                start = None;
            }
            _ => {}
        }
    }
    out
}

fn match_case(word: &str, like: &str) -> String {
    if like.chars().next().is_some_and(char::is_uppercase) {
        let mut c = word.chars();
        c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
    } else {
        word.to_string()
    }
}

/// Candidate fixes for a finding, from its code's strategy (see
/// `xenify_grammar::code_candidates`). Not yet tested.
pub fn candidates(text: &str, f: &Finding) -> Vec<Fix> {
    candidates_for(text, f, xenify_grammar::code_candidates(&f.code))
}

fn candidates_for(text: &str, f: &Finding, strategy: &str) -> Vec<Fix> {
    let words = words_in(text, &f.range);
    let mut out = Vec::new();
    let list = |s: &str| s.split(',').map(str::to_string).collect::<Vec<_>>();
    if let Some(items) = strategy.strip_prefix("insert:") {
        // Before each word of the span, and after the last.
        let mut gaps: Vec<usize> = words.iter().map(|(r, _)| r.start).collect();
        if let Some((r, _)) = words.last()
            && text[r.end..].starts_with(' ')
        {
            gaps.push(r.end + 1);
        }
        // Gaps between the spot's words first: the spot is usually marked around
        // the gap ("went ⟦⟧ store"), and only the first candidates get tested.
        if gaps.len() > 2 {
            let first = gaps.remove(0);
            gaps.push(first);
        }
        for g in gaps {
            for w in list(items) {
                let next_is_cap = text[g..].chars().next().is_some_and(char::is_uppercase)
                    && (g == 0 || text[..g].trim_end().ends_with('.'));
                let ins = if next_is_cap { format!("{} ", match_case(&w, "X")) } else { format!("{w} ") };
                out.push(Fix { range: g..g, text: ins });
            }
        }
    } else if let Some(items) = strategy.strip_prefix("replace:") {
        let opts = list(items);
        for (r, w) in &words {
            if opts.iter().any(|o| o.eq_ignore_ascii_case(w)) || words.len() == 1 {
                for o in &opts {
                    if o.eq_ignore_ascii_case(w) {
                        continue;
                    }
                    if o.is_empty() {
                        let end = if text[r.end..].starts_with(' ') { r.end + 1 } else { r.end };
                        out.push(Fix { range: r.start..end, text: String::new() });
                    } else {
                        out.push(Fix { range: r.clone(), text: match_case(o, w) });
                    }
                }
            }
        }
    } else {
        match strategy {
            "soundalikes" => {
                for (r, w) in &words {
                    let lw = w.to_lowercase();
                    if let Some(set) = xenify_grammar::sound_alike::set_of(&lw) {
                        for m in &xenify_grammar::sound_alike::sets()[set] {
                            // Rare words ("pi") are never suggested for another.
                            if m.word != lw && !m.never_suggest {
                                out.push(Fix { range: r.clone(), text: match_case(&m.word, w) });
                            }
                        }
                    }
                }
            }
            "verbforms" => {
                for (r, w) in &words {
                    for v in xenify_grammar::verb_forms(w) {
                        out.push(Fix { range: r.clone(), text: match_case(&v, w) });
                    }
                }
            }
            "pronouns" => {
                for (r, w) in &words {
                    let lw = w.to_lowercase();
                    for (a, b) in PRONOUNS {
                        let other = if lw == a.to_lowercase() {
                            Some(*b)
                        } else if lw == *b {
                            Some(*a)
                        } else {
                            None
                        };
                        if let Some(o) = other {
                            let o = if o == "I" { "I".to_string() } else { match_case(o, w) };
                            out.push(Fix { range: r.clone(), text: o });
                        }
                    }
                }
            }
            "commas" => {
                for (r, _) in &words {
                    if !text[r.end..].starts_with(',') {
                        out.push(Fix { range: r.end..r.end, text: ",".into() });
                    }
                }
            }
            "splice" => {
                for (i, _) in text[f.range.clone()].match_indices(", ") {
                    let at = f.range.start + i;
                    let next: String = text[at + 2..].chars().take(1).flat_map(char::to_uppercase).collect();
                    let next_len = text[at + 2..].chars().next().map(char::len_utf8).unwrap_or(0);
                    out.push(Fix { range: at..at + 2 + next_len, text: format!(". {next}") });
                    out.push(Fix { range: at..at + 2, text: "; ".into() });
                    out.push(Fix { range: at..at + 2, text: ", and ".into() });
                    out.push(Fix { range: at..at + 2, text: ", so ".into() });
                }
            }
            "hyphen" => {
                // Missing: join the words. Not needed: take it out ("highly-skilled").
                if words.len() >= 2 {
                    let r = words[0].0.start..words[words.len() - 1].0.end;
                    out.push(Fix { range: r.clone(), text: text[r].replace(' ', "-") });
                }
                for (r, w) in &words {
                    if w.contains('-') {
                        out.push(Fix { range: r.clone(), text: w.replace('-', " ") });
                    }
                }
            }
            "apostrophes" => {
                for (r, w) in &words {
                    let alt: Vec<String> = if let Some(b) = w.strip_suffix("'s") {
                        vec![format!("{b}s"), format!("{b}s'")]
                    } else if let Some(b) = w.strip_suffix("s'") {
                        vec![format!("{b}s"), format!("{b}'s")]
                    } else if let Some(b) = w.strip_suffix('s') {
                        vec![format!("{b}'s"), format!("{w}'")]
                    } else {
                        vec![format!("{w}'s")]
                    };
                    out.extend(alt.into_iter().map(|a| Fix { range: r.clone(), text: a }));
                }
            }
            "inflections" => {
                for (r, w) in &words {
                    for v in inflections(w) {
                        out.push(Fix { range: r.clone(), text: match_case(&v, w) });
                    }
                }
            }
            "swap" => {
                for pair in words.windows(2) {
                    let ((a, wa), (b, wb)) = (&pair[0], &pair[1]);
                    if text[a.end..b.start].chars().all(char::is_whitespace) {
                        let gap = &text[a.end..b.start];
                        let (first, second) = if a.start == 0 || text[..a.start].trim_end().ends_with(['.', '!', '?']) {
                            (match_case(&wb.to_lowercase(), wa), if wa == "I" { wa.clone() } else { wa.to_lowercase() })
                        } else {
                            (wb.clone(), wa.clone())
                        };
                        out.push(Fix { range: a.start..b.end, text: format!("{first}{gap}{second}") });
                    }
                }
            }
            "delete" => {
                for (r, _) in &words {
                    let end = if text[r.end..].starts_with(' ') { r.end + 1 } else { r.end };
                    out.push(Fix { range: r.start..end, text: String::new() });
                }
            }
            _ => {}
        }
    }
    out.truncate(40);
    out
}

/// The word with its ending changed: -s/-es/-ies added or removed, and its
/// other verb forms ("feels" → feel, felt…; "informations" → information).
fn inflections(w: &str) -> Vec<String> {
    let lw = w.to_lowercase();
    let mut v = Vec::new();
    if let Some(b) = lw.strip_suffix("ies") {
        v.push(format!("{b}y"));
    }
    if let Some(b) = lw.strip_suffix("es") {
        v.push(b.to_string());
    }
    if let Some(b) = lw.strip_suffix('s').filter(|b| !b.ends_with('s')) {
        v.push(b.to_string());
    }
    if !lw.ends_with('s') {
        match lw.strip_suffix('y') {
            Some(b) if !b.ends_with(['a', 'e', 'o', 'u']) => v.push(format!("{b}ies")),
            _ if lw.ends_with(['x', 'z']) || lw.ends_with("ch") || lw.ends_with("sh") => v.push(format!("{lw}es")),
            _ => v.push(format!("{lw}s")),
        }
    }
    v.extend(xenify_grammar::verb_forms(w).into_iter().map(|f| f.to_lowercase()));
    v.retain(|x| !x.is_empty() && *x != lw);
    let mut seen = Vec::new();
    v.retain(|x| {
        let new = !seen.contains(x);
        seen.push(x.clone());
        new
    });
    v
}

/// Candidates when the guessed kind of mistake gave none that work: the model
/// may have named the wrong kind ("feels" called a plural). Word endings, verb
/// forms, sound-alikes and two neighbouring words swapped.
fn fallback_candidates(text: &str, f: &Finding) -> Vec<Fix> {
    let mut out = Vec::new();
    for strategy in ["inflections", "soundalikes", "swap"] {
        let g = Finding { code: String::new(), ..f.clone() };
        out.extend(candidates_for(text, &g, strategy));
    }
    out.truncate(40);
    out
}

impl Spotter {
    /// Up to `max` fixes for a finding, each tested: applied to the text,
    /// the spot must then look right to the model. Best first.
    pub fn suggest(&self, text: &str, f: &Finding, max: usize) -> Result<Vec<Fix>, String> {
        let before_scores = self.scores(text, &[])?;
        let before =
            before_scores.iter().filter(|(r, _, _)| overlaps(r, &f.range)).map(|(_, p, _)| *p).fold(0.0, f32::max);
        let mut tested = self.test_fixes(text, f, &before_scores, before, candidates(text, f))?;
        // A sentence that doesn't hold together needs rewriting, not a one-word change.
        if tested.is_empty() && f.code != "garbled" {
            tested = self.test_fixes(text, f, &before_scores, before, fallback_candidates(text, f))?;
        }
        tested.sort_by(|a, b| a.1.total_cmp(&b.1));
        Ok(tested.into_iter().take(max).map(|(f, _)| f).collect())
    }

    /// The fixes that make the spot look right, with how wrong the area still looks.
    fn test_fixes(
        &self,
        text: &str,
        f: &Finding,
        before_scores: &[(Range<usize>, f32, usize)],
        before: f32,
        fixes: Vec<Fix>,
    ) -> Result<Vec<(Fix, f32)>, String> {
        let mut tested = Vec::new();
        for fix in fixes {
            let mut t = text.to_string();
            t.replace_range(fix.range.clone(), &fix.text);
            let new_end = fix.range.start + fix.text.len();
            // Back to the original text's offsets (None inside the fix).
            let original = |r: &Range<usize>| -> Option<Range<usize>> {
                if r.end <= fix.range.start {
                    Some(r.clone())
                } else if r.start >= new_end {
                    Some(r.start + fix.range.end - new_end..r.end + fix.range.end - new_end)
                } else {
                    None
                }
            };
            // Near the spot: the spot itself must look right now, and the
            // fix mustn't make other words look worse than they did
            // (mistakes already elsewhere don't count against it).
            // (A fix that breaks something shows it right beside it:
            // "an pet", "pet a dog".)
            let lo = fix.range.start.saturating_sub(12);
            let hi = (new_end + 12).min(t.len());
            let near = self
                .scores(&t, &[])?
                .into_iter()
                .filter(|(r, _, _)| r.end >= lo && r.start <= hi)
                .map(|(r, p, _)| match original(&r) {
                    Some(o) if !overlaps(&o, &f.range) => {
                        let old = before_scores.iter().find(|(b, _, _)| *b == o).map_or(0.0, |(_, q, _)| *q);
                        if p > old + 0.3 { p } else { 0.0 }
                    }
                    _ => p,
                })
                .fold(0.0f32, f32::max);
            if near < 0.5 && near < before {
                tested.push((fix, near));
            }
        }
        Ok(tested)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Set XENIFY_SPOTTER to a model folder to run these.
    fn model() -> Option<Spotter> {
        let dir = std::env::var_os("XENIFY_SPOTTER")?;
        Some(Spotter::load(Path::new(&dir)).expect("model loads"))
    }

    #[test]
    fn inflections_cover_plurals_and_verb_forms() {
        let has = |w: &str, x: &str| inflections(w).iter().any(|v| v == x);
        assert!(has("feels", "feel"), "{:?}", inflections("feels"));
        assert!(has("informations", "information"));
        assert!(has("child", "childs") && has("story", "stories") && has("stories", "story"));
        assert!(has("box", "boxes") && has("boxes", "box"));
        assert!(!inflections("glass").contains(&"glas".to_string()));
    }

    #[test]
    fn swap_tries_neighbouring_words() {
        let t = "I know not why.";
        let f = Finding { range: 2..10, code: "word_order".into(), confidence: 1.0 };
        let fixes: Vec<String> = candidates_for(t, &f, "swap").into_iter().map(|x| x.text).collect();
        assert!(fixes.contains(&"not know".to_string()), "{fixes:?}");
    }

    #[test]
    fn falls_back_when_the_kind_of_mistake_is_wrong() {
        let Some(s) = model() else { return };
        let t = "Wot does it feels like to seat in the shadow?";
        let at = t.find("feels").unwrap();
        // As if the model had called it a plural (B7 did).
        let f = Finding { range: at..at + 5, code: "real_word_typo".into(), confidence: 0.9 };
        let fixes = s.suggest(t, &f, 3).unwrap();
        assert!(fixes.iter().any(|x| x.text == "feel"), "{fixes:?}");
    }

    #[test]
    fn suggests_a_fix_next_to_another_mistake() {
        let Some(s) = model() else { return };
        let t = "I have pet dog at home and we needs to finish the report.";
        let f = s.spot(t, 0.5).unwrap().into_iter().find(|f| f.code == "article_missing").expect("spotted");
        let fixes = s.suggest(t, &f, 3).unwrap();
        assert!(fixes.iter().any(|x| x.text.trim() == "a" || x.text.contains("a pet")), "{fixes:?}");
    }

    #[test]
    fn spots_mistakes_and_leaves_correct_text() {
        let Some(s) = model() else { return };
        let t = "We need finish the migration before Friday.";
        let f = s.spot(t, 0.9).unwrap();
        assert!(
            f.iter().any(|f| t[f.range.clone()].contains("need") || t[f.range.clone()].contains("finish")),
            "{f:?}"
        );
        assert!(s.spot("The team reviewed the proposal and approved it on Monday.", 0.9).unwrap().is_empty());
        let ranked = s.best_of(t, 8..8, &["to ".into(), "the ".into(), "of ".into()]).unwrap();
        assert_eq!(ranked[0].0, "to ", "{ranked:?}");
    }

    #[test]
    fn suggests_tested_fixes() {
        let Some(s) = model() else { return };
        for (text, wants) in [
            ("We need finish the migration before Friday.", &["We need to finish the migration before Friday."][..]),
            ("I have pet dog at home.", &["I have a pet dog at home."][..]),
            (
                "The meeting ran late, we missed the train.",
                &[
                    "The meeting ran late. We missed the train.",
                    "The meeting ran late; we missed the train.",
                    "The meeting ran late, so we missed the train.",
                    "The meeting ran late, and we missed the train.",
                ][..],
            ),
        ] {
            let f = s.spot(text, 0.9).unwrap();
            assert!(!f.is_empty(), "{text}: nothing spotted");
            let fixes = s.suggest(text, &f[0], 3).unwrap();
            let applied: Vec<String> = fixes
                .iter()
                .map(|x| {
                    let mut t = text.to_string();
                    t.replace_range(x.range.clone(), &x.text);
                    t
                })
                .collect();
            assert!(applied.iter().any(|a| wants.contains(&a.as_str())), "{text}: {applied:?} ({:?})", f[0]);
        }
    }
}
