//! Mail-merge data: CSV (with a header row) or JSON (an array of objects,
//! or an object holding one), as a list of records.

use serde_json::{Map, Value};

/// Splits CSV text into rows of fields (quotes, doubled quotes, newlines
/// inside quotes; the delimiter is guessed from the header line).
pub fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let first = text.lines().next().unwrap_or("");
    let delim = [',', ';', '\t', '|'].into_iter().max_by_key(|d| first.matches(*d).count()).unwrap_or(',');
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    field.push('"');
                    chars.next();
                }
                '"' => quoted = false,
                _ => field.push(c),
            }
            continue;
        }
        match c {
            '"' if field.is_empty() => quoted = true,
            c if c == delim => row.push(std::mem::take(&mut field)),
            '\r' => {}
            '\n' => {
                row.push(std::mem::take(&mut field));
                if !(row.len() == 1 && row[0].is_empty()) {
                    rows.push(std::mem::take(&mut row));
                }
                row.clear();
            }
            _ => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

/// Records from CSV text: one object per row, keyed by the header.
pub fn csv_records(text: &str) -> Vec<Value> {
    let mut rows = parse_csv(text).into_iter();
    let Some(header) = rows.next() else { return Vec::new() };
    let header: Vec<String> = header.into_iter().map(|h| h.trim().to_string()).collect();
    rows.map(|r| {
        let mut m = Map::new();
        for (i, h) in header.iter().enumerate() {
            m.insert(h.clone(), Value::String(r.get(i).cloned().unwrap_or_default()));
        }
        Value::Object(m)
    })
    .collect()
}

/// Records from a data file's bytes (`name` picks CSV or JSON).
pub fn records(bytes: &[u8], name: &str) -> Result<Vec<Value>, String> {
    let text = String::from_utf8_lossy(bytes);
    if name.to_ascii_lowercase().ends_with(".json") {
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("{name}: {e}"))?;
        return match v {
            Value::Array(a) => Ok(a),
            Value::Object(m) => {
                // An object with one array in it (e.g. {"customers": [...]}), or a single record.
                match m.values().find(|v| v.is_array()) {
                    Some(Value::Array(a)) => Ok(a.clone()),
                    _ => Ok(vec![Value::Object(m)]),
                }
            }
            _ => Err(format!("{name} doesn't hold a list of records")),
        };
    }
    Ok(csv_records(&text))
}

/// Rows with the same `key` become one record: the first row's fields,
/// plus `rows`, the list of all of them (for repeated table rows).
pub fn group(records: Vec<Value>, key: &str) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    let mut index: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for r in records {
        let k = r.get(key).map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_string)).unwrap_or_default();
        match index.get(&k) {
            Some(&i) => {
                if let Some(Value::Array(rows)) = out[i].get_mut("rows") {
                    rows.push(r);
                }
            }
            None => {
                let mut first = r.clone();
                if let Value::Object(m) = &mut first {
                    m.insert("rows".into(), Value::Array(vec![r]));
                }
                index.insert(k, out.len());
                out.push(first);
            }
        }
    }
    out
}

/// `1-5,8` style ranges (1-based, inclusive) → 0-based indexes below `n`.
pub fn select(range: &str, n: usize) -> Vec<usize> {
    if range.trim().is_empty() {
        return (0..n).collect();
    }
    let mut out = Vec::new();
    for part in range.split(',') {
        let part = part.trim();
        let (a, b) = match part.split_once('-') {
            Some((a, b)) => (a.trim().parse::<usize>().unwrap_or(1), b.trim().parse::<usize>().unwrap_or(n)),
            None => match part.parse::<usize>() {
                Ok(v) => (v, v),
                Err(_) => continue,
            },
        };
        for i in a.max(1)..=b.min(n) {
            if !out.contains(&(i - 1)) {
                out.push(i - 1);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_with_quotes_and_delimiters() {
        let rows = parse_csv("name;note\n\"Smith, J\";\"said \"\"hi\"\"\nthen left\"\nDoe;x\n");
        assert_eq!(rows, vec![vec!["name", "note"], vec!["Smith, J", "said \"hi\"\nthen left"], vec!["Doe", "x"]]);
        let recs = csv_records("a,b\n1,2\n3\n");
        assert_eq!(recs[1]["b"], "");
    }

    #[test]
    fn json_and_grouping() {
        let r = records(br#"{"customers": [{"n": 1}, {"n": 2}]}"#, "x.json").unwrap();
        assert_eq!(r.len(), 2);
        let g = group(csv_records("inv,item\nA,tea\nA,cake\nB,jam\n"), "inv");
        assert_eq!(g.len(), 2);
        assert_eq!(g[0]["rows"].as_array().unwrap().len(), 2);
        assert_eq!(select("2-3, 5", 4), vec![1, 2]);
        assert_eq!(select("", 3), vec![0, 1, 2]);
    }
}

/// What a merge produces.
#[derive(Clone, Debug)]
pub struct MergeJob {
    pub format: crate::Format,
    /// One file with every record, instead of one file per record.
    pub combined: bool,
    /// File names (without extension), with fields; empty: the config's or `{{#}}`.
    pub name_template: String,
    /// Records to use, like `1-10,15` (empty: all).
    pub range: String,
}

/// The merged files: (file name with extension, bytes). `progress` is called
/// after each record with (done, total).
pub fn run(
    doc: &xenify_wp_doc::Document,
    records: &[Value],
    job: &MergeJob,
    engine: &mut xenify_wp_layout::Engine,
    image: &dyn Fn(&str) -> Option<std::sync::Arc<Vec<u8>>>,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<Vec<(String, Vec<u8>)>, crate::IoError> {
    use xenify_wp_doc::merge;
    let chosen = select(&job.range, records.len());
    let ext = job.format.extension();
    let template = if job.name_template.trim().is_empty() {
        doc.meta.merge.as_ref().map(|m| m.file_name.clone()).unwrap_or_default()
    } else {
        job.name_template.clone()
    };
    // Merged documents have no merge settings of their own.
    let mut base = doc.clone();
    base.meta.merge = None;
    let total = chosen.len();
    if job.combined {
        let docs: Vec<_> = chosen.iter().map(|&i| merge::instantiate(&base, &records[i], i + 1)).collect();
        progress(total, total);
        let all = merge::combine(docs);
        let stem = doc.meta.title.trim();
        let stem = if stem.is_empty() || stem.contains("{{") { "merged" } else { stem };
        let name = format!("{}.{ext}", merge::file_name(stem, &Value::Null, 0));
        return Ok(vec![(name, crate::save_with(&all, job.format, engine, image)?)]);
    }
    let mut out = Vec::new();
    let mut used = std::collections::HashSet::new();
    for (k, &i) in chosen.iter().enumerate() {
        let one = merge::instantiate(&base, &records[i], i + 1);
        let mut name = merge::file_name(&template, &records[i], i + 1);
        let mut n = 2;
        let stem = name.clone();
        while !used.insert(name.clone()) {
            name = format!("{stem}-{n}");
            n += 1;
        }
        out.push((format!("{name}.{ext}"), crate::save_with(&one, job.format, engine, image)?));
        progress(k + 1, total);
    }
    Ok(out)
}
