//! The plan's own fields: checking a value against its field's rules (as a
//! schema would: kind, required, lowest/highest, length, pattern, picks),
//! and what an issue still needs filling in.

use crate::model::{Field, Plan, Task};
use crate::schedule::day_name;
use serde_json::{Value as Json, json};
use xenify_holidays::Date;

/// What kinds of field there are, as the screens name them.
pub const KINDS: &[(&str, &str)] = &[
    ("text", "Text"),
    ("long_text", "Long text"),
    ("number", "Number"),
    ("date", "Date"),
    ("choice", "One choice"),
    ("choices", "Several choices"),
    ("person", "Person"),
    ("check", "Yes or no"),
    ("link", "Web link"),
    ("email", "Email address"),
];

fn empty(v: &Json) -> bool {
    match v {
        Json::Null => true,
        Json::String(s) => s.trim().is_empty(),
        Json::Array(a) => a.is_empty(),
        _ => false,
    }
}

fn num(n: f64, f: &Field) -> String {
    let s = match f.decimals {
        Some(d) => format!("{n:.*}", d as usize),
        None => {
            let s = format!("{n}");
            s.strip_suffix(".0").map(String::from).unwrap_or(s)
        }
    };
    format!("{}{s}{}", f.prefix, if f.suffix.is_empty() { String::new() } else { format!(" {}", f.suffix) })
}

/// Checks `value` for field `f`: the value to keep (tidied: trimmed text,
/// rounded numbers), Null for nothing, or what's wrong in a sentence.
pub fn check(f: &Field, value: &Json, today: Date, people: &[&str]) -> Result<Json, String> {
    if empty(value) {
        return Ok(Json::Null);
    }
    let name = &f.name;
    match f.kind.as_str() {
        "number" => {
            let n = match value {
                Json::Number(n) => n.as_f64(),
                Json::String(s) => s
                    .trim()
                    .trim_start_matches(f.prefix.as_str())
                    .trim_end_matches(f.suffix.as_str())
                    .trim()
                    .replace(',', "")
                    .parse::<f64>()
                    .ok(),
                _ => None,
            }
            .filter(|n| n.is_finite())
            .ok_or_else(|| format!("{name} is a number"))?;
            if f.decimals == Some(0) && n.fract() != 0.0 {
                return Err(format!("{name} is a whole number"));
            }
            let n = match f.decimals {
                Some(d) => {
                    let k = 10f64.powi(d as i32);
                    (n * k).round() / k
                }
                None => n,
            };
            if let Some(lo) = f.min
                && n < lo
            {
                return Err(format!("{name} is at least {}", num(lo, f)));
            }
            if let Some(hi) = f.max
                && n > hi
            {
                return Err(format!("{name} is at most {}", num(hi, f)));
            }
            Ok(json!(n))
        }
        "date" => {
            let d = value.as_str().and_then(|s| Date::parse(s.trim())).ok_or_else(|| format!("{name} is a date"))?;
            if let Some(lo) = f.min_date
                && d < lo
            {
                return Err(format!("{name} is no earlier than {}", day_name(lo)));
            }
            if let Some(hi) = f.max_date
                && d > hi
            {
                return Err(format!("{name} is no later than {}", day_name(hi)));
            }
            if f.not_past && d < today {
                return Err(format!("{name} can’t be in the past"));
            }
            Ok(json!(d))
        }
        "check" => match value {
            Json::Bool(b) => Ok(json!(b)),
            Json::String(s) if matches!(s.to_lowercase().as_str(), "yes" | "true") => Ok(json!(true)),
            Json::String(s) if matches!(s.to_lowercase().as_str(), "no" | "false") => Ok(json!(false)),
            _ => Err(format!("{name} is yes or no")),
        },
        "choice" => {
            let s = value.as_str().map(str::trim).unwrap_or_default();
            let c = f
                .choices
                .iter()
                .find(|c| c.eq_ignore_ascii_case(s))
                .ok_or_else(|| format!("{name} is one of: {}", f.choices.join(", ")))?;
            Ok(json!(c))
        }
        "choices" => {
            let picked: Vec<&str> = match value {
                Json::Array(a) => a.iter().filter_map(Json::as_str).collect(),
                Json::String(s) => s.split(',').map(str::trim).filter(|x| !x.is_empty()).collect(),
                _ => Vec::new(),
            };
            let mut out: Vec<String> = Vec::new();
            for p in picked {
                let c = f
                    .choices
                    .iter()
                    .find(|c| c.eq_ignore_ascii_case(p))
                    .ok_or_else(|| format!("“{p}” isn’t one of {name}’s choices"))?;
                if !out.contains(c) {
                    out.push(c.clone());
                }
            }
            if let Some(lo) = f.min_picks
                && (out.len() as u32) < lo
            {
                return Err(format!("Pick at least {lo} for {name}"));
            }
            if let Some(hi) = f.max_picks
                && (out.len() as u32) > hi
            {
                return Err(format!("Pick at most {hi} for {name}"));
            }
            Ok(json!(out))
        }
        "person" => {
            let s = value.as_str().unwrap_or_default();
            if people.contains(&s) { Ok(json!(s)) } else { Err(format!("{name} is someone in this plan")) }
        }
        // text, long_text, link, email
        _ => {
            let s = value.as_str().ok_or_else(|| format!("{name} is text"))?.trim().to_string();
            let n = s.chars().count() as u32;
            if let Some(lo) = f.min_len
                && n < lo
            {
                return Err(format!("{name} is at least {lo} characters"));
            }
            if let Some(hi) = f.max_len
                && n > hi
            {
                return Err(format!("{name} is at most {hi} characters"));
            }
            if f.kind == "email" && !is_email(&s) {
                return Err(format!("{name} is an email address, like sam@example.com"));
            }
            let s = if f.kind == "link" {
                let s = if s.contains("://") || s.starts_with("mailto:") { s } else { format!("https://{s}") };
                if !is_link(&s) {
                    return Err(format!("{name} is a web address, like https://example.com"));
                }
                s
            } else {
                s
            };
            if !f.pattern.is_empty() {
                let re = regex_lite::Regex::new(&format!("^(?:{})$", f.pattern))
                    .map_err(|_| format!("{name}’s pattern can’t be read; change it in Fields"))?;
                if !re.is_match(&s) {
                    return Err(if f.pattern_says.is_empty() {
                        format!("{name} isn’t in the right form")
                    } else {
                        f.pattern_says.clone()
                    });
                }
            }
            Ok(json!(s))
        }
    }
}

fn is_email(s: &str) -> bool {
    let Some((a, b)) = s.split_once('@') else { return false };
    !a.is_empty()
        && !s.contains(char::is_whitespace)
        && b.contains('.')
        && !b.starts_with('.')
        && !b.ends_with('.')
        && !b.contains('@')
}

fn is_link(s: &str) -> bool {
    let rest = s.strip_prefix("https://").or_else(|| s.strip_prefix("http://"));
    match rest {
        Some(r) => {
            r.split('/').next().is_some_and(|host| host.contains('.') || host.starts_with("localhost"))
                && !s.contains(char::is_whitespace)
        }
        None => s.starts_with("mailto:"),
    }
}

/// Whether field `f` is for issues of this type.
pub fn applies(f: &Field, t: &Task) -> bool {
    f.types.is_empty() || f.types.contains(&t.kind)
}

/// A rule-set that can't be kept (lowest above highest …), in a sentence.
pub fn check_field(f: &Field) -> Result<(), String> {
    if f.name.trim().is_empty() {
        return Err("A field needs a name".into());
    }
    if !KINDS.iter().any(|(k, _)| *k == f.kind) {
        return Err(format!("There’s no kind of field called “{}”", f.kind));
    }
    if let (Some(a), Some(b)) = (f.min, f.max)
        && a > b
    {
        return Err("The lowest number is above the highest".into());
    }
    if let (Some(a), Some(b)) = (f.min_len, f.max_len)
        && a > b
    {
        return Err("The fewest characters is more than the most".into());
    }
    if let (Some(a), Some(b)) = (f.min_date, f.max_date)
        && a > b
    {
        return Err("The earliest date is after the latest".into());
    }
    if let (Some(a), Some(b)) = (f.min_picks, f.max_picks)
        && a > b
    {
        return Err("The fewest picks is more than the most".into());
    }
    if matches!(f.kind.as_str(), "choice" | "choices") && f.choices.iter().all(|c| c.trim().is_empty()) {
        return Err("Give it at least one choice".into());
    }
    if !f.pattern.is_empty() && regex_lite::Regex::new(&f.pattern).is_err() {
        return Err("The pattern isn’t a regular expression that can be read".into());
    }
    Ok(())
}

/// What an issue is still missing: no description, no estimate, required
/// fields left empty. Epics and milestones need no estimate; done issues need nothing.
pub fn missing(plan: &Plan, t: &Task) -> Vec<String> {
    let mut out = Vec::new();
    // Done, or a mirror (nothing of its own to fill in).
    if plan.is_done(t) || t.mirror.is_some() {
        return out;
    }
    if t.work.is_none() && !plan.is_epic(t) && !plan.is_milestone(t) {
        out.push("estimate".to_string());
    }
    for f in &plan.fields {
        if f.required && applies(f, t) && t.fields.get(&f.id).is_none_or(empty) {
            out.push(f.name.clone());
        }
    }
    out
}
