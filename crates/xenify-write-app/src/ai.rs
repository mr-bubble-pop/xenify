//! The AI assistant, with the AI of the Xenify server Write is signed in to
//! (its admin chooses the models): review, rewrite, continue writing and
//! questions about the document. The
//! documents aren't held while the model works, so typing carries on.

use serde_json::{Value, json};
use std::sync::Mutex;
use xenify_openrouter::OpenRouter;
use xenify_wp_ai::{Message, Voice};
use xenify_wp_doc::{NodeId, ops};
use xenify_write_host::docs::Docs;

fn arg<'a>(args: &'a Value, key: &str) -> &'a str {
    args.get(key).and_then(Value::as_str).unwrap_or("")
}

/// The paragraph being written: the caret's, or when that's empty (a new
/// line just started) the nearest one above it with text, else below.
fn writing_para(ed: &xenify_wp_doc::Editor) -> Option<NodeId> {
    let has_text = |p: &xenify_wp_doc::Paragraph| !xenify_wp_ai::readable(&ed.doc, p).trim().is_empty();
    let all: Vec<&xenify_wp_doc::Paragraph> = ed.doc.paragraphs().collect();
    let at = all.iter().position(|p| p.id == ed.sel.focus.para)?;
    if has_text(all[at]) {
        return Some(all[at].id);
    }
    all[..at].iter().rev().find(|p| has_text(p)).or_else(|| all[at..].iter().find(|p| has_text(p))).map(|p| p.id)
}

/// The table the caret is in, if any: asking about "this paragraph" there
/// means the whole table (one cell alone makes no sense to the AI).
fn caret_table(ed: &xenify_wp_doc::Editor) -> Option<NodeId> {
    xenify_wp_doc::tables::cell_at(ed, ed.sel.focus.para).map(|(t, _, _)| t)
}

/// What to work on: "selection", "paragraph" (the caret's) or "whole" (`whole: true` too).
fn scope(args: &Value) -> &str {
    if args.get("whole").and_then(Value::as_bool).unwrap_or(false) {
        return "whole";
    }
    args.get("scope").and_then(Value::as_str).unwrap_or("selection")
}

/// The Xenify server whose AI is used, while signed in to one that has AI.
struct ServerAi {
    url: String,
    token: String,
}
static SERVER: Mutex<Option<ServerAi>> = Mutex::new(None);

fn send(body: &Value) -> Result<Value, String> {
    let (url, token) = SERVER
        .lock()
        .map_err(|e| e.to_string())?
        .as_ref()
        .map(|s| (s.url.clone(), s.token.clone()))
        .ok_or("AI comes from a Xenify server: sign in to one that has AI")?;
    // An empty model: the server's admin chooses it.
    let mut body = body.clone();
    body["model"] = json!("");
    let client = OpenRouter::with_key(&format!("{url}/v1/ai"), &token);
    match client.chat(&body) {
        // Some models can't fill in tools: ask for the same JSON in the reply.
        Err(xenify_openrouter::AiError::Api { status: 400 | 404, .. }) if body.get("tools").is_some() => {
            client.chat(&xenify_wp_ai::without_tools(&body)).map_err(|e| e.to_string())
        }
        r => r.map_err(|e| e.to_string()),
    }
}

pub fn call(docs: &Mutex<Docs>, name: &str, args: &Value) -> Result<Value, String> {
    let lock = || docs.lock().map_err(|_| "Xenify Write hit a problem; please restart it".to_string());
    match name {
        "status" => Ok(json!({
            "server": SERVER.lock().ok().and_then(|s| s.as_ref().map(|s| json!({ "url": s.url }))),
        })),
        // Use a Xenify server's AI (`url`, `token`), or stop (no `url`).
        "use_server" => {
            let url = arg(args, "url").trim_end_matches('/');
            *SERVER.lock().map_err(|e| e.to_string())? =
                (!url.is_empty()).then(|| ServerAi { url: url.into(), token: arg(args, "token").into() });
            Ok(Value::Null)
        }
        "review" => {
            let (doc, paras, model) = {
                let d = lock()?;
                let ed = d.front_editor();
                let paras: Vec<NodeId> = match scope(args) {
                    "paragraph" => vec![writing_para(ed).ok_or("there's no text to look at yet")?],
                    "selection" if !ed.sel.is_collapsed() => ops::selected_paragraphs(ed),
                    _ => ed.doc.paragraphs().map(|p| p.id).collect(),
                };
                (ed.doc.clone(), paras, String::new())
            };
            let focus = xenify_wp_ai::Focus::ALL.to_vec();
            let voice = Voice::of(&doc);
            // In parts of about 250 words, all sent at once: one long answer
            // took most of a minute even on fast services.
            let parts = xenify_wp_ai::review_parts(&doc, &paras, 250);
            let replies: Vec<Result<Value, String>> = std::thread::scope(|sc| {
                let asks: Vec<_> = parts
                    .iter()
                    .map(|part| {
                        let body = xenify_wp_ai::review_request(&model, &doc, part, &focus, &voice);
                        sc.spawn(move || send(&body))
                    })
                    .collect();
                asks.into_iter().map(|a| a.join().unwrap_or_else(|_| Err("the AI check stopped".into()))).collect()
            });
            let (mut found, mut dropped) = (Vec::new(), 0);
            let mut failed = None;
            for r in replies {
                match r {
                    Ok(resp) => {
                        let (f, d) = xenify_wp_ai::parse_review(&resp, &doc);
                        found.extend(f);
                        dropped += d;
                    }
                    Err(e) => failed = Some(e),
                }
            }
            // Every part failing is an error; some failing still shows the rest.
            if let Some(e) = failed.filter(|_| found.is_empty() && dropped == 0) {
                return Err(e);
            }
            Ok(json!({
                "dropped": dropped,
                "suggestions": found.iter().map(|s| {
                    let text = doc.para(s.para).map(|p| p.text.as_str()).unwrap_or("");
                    json!({
                        "para": s.para.0, "start": s.range.start, "end": s.range.end,
                        "quote": text.get(s.range.clone()).unwrap_or(""),
                        "category": s.category, "severity": s.severity,
                        "explanation": s.explanation, "replacement": s.replacement,
                    })
                }).collect::<Vec<_>>(),
            }))
        }
        "preview" => {
            // What a scope would send, shown in the dialog before anything is sent.
            let d = lock()?;
            let ed = d.front_editor();
            let table = caret_table(ed).and_then(|t| ed.doc.table(t));
            let text = match scope(args) {
                "whole" => xenify_wp_ai::readable_document(&ed.doc),
                "paragraph" | "selection"
                    if table.is_some() && (scope(args) == "paragraph" || ed.sel.is_collapsed()) =>
                {
                    table.map(|t| xenify_wp_ai::readable_table(&ed.doc, t)).unwrap_or_default()
                }
                "paragraph" => writing_para(ed)
                    .and_then(|id| ed.doc.para(id))
                    .map(|p| xenify_wp_ai::readable(&ed.doc, p))
                    .unwrap_or_default(),
                _ => ops::copy_spans(ed)
                    .iter()
                    .map(|s| s.text.replace(xenify_wp_doc::Field::CHAR, ""))
                    .collect::<Vec<_>>()
                    .join(" "),
            };
            // (A table's words: what's in its cells, not the row labels.)
            let words = match table {
                Some(t) if scope(args) != "whole" && text.starts_with("A table") => {
                    let mut ids = Vec::new();
                    for row in &t.rows {
                        for c in &row.cells {
                            ids.extend(c.blocks.iter().filter_map(|b| match b {
                                xenify_wp_doc::Block::Paragraph(p) => Some(p.text.split_whitespace().count()),
                                _ => None,
                            }));
                        }
                    }
                    ids.iter().sum()
                }
                _ => text.split_whitespace().count(),
            };
            let start: String = text.split_whitespace().take(14).collect::<Vec<_>>().join(" ");
            Ok(json!({ "words": words, "start": start, "more": words > 14, "table": table.map(|t| t.rows.len()) }))
        }
        "rewrite" => {
            // The selection inside one paragraph, or the caret's paragraph.
            let (para, range, text, voice, model) = {
                let d = lock()?;
                let ed = d.front_editor();
                let (a, b) = ed.doc.ordered(ed.sel);
                if a.para != b.para {
                    return Err("select text inside one paragraph to rewrite it".into());
                }
                // No selection: the paragraph being written.
                let para = if a == b { writing_para(ed).ok_or("there's no text to rewrite yet")? } else { a.para };
                let p = ed.doc.para(para).ok_or("put the cursor in some text")?;
                let range = if a == b { 0..p.text.len() } else { a.offset..b.offset };
                if p.text[range.clone()].trim().is_empty() {
                    return Err(
                        "there's no text here to rewrite: put the cursor in a paragraph, or select some words".into()
                    );
                }
                // A date, badge or picture in the middle would be lost in the new text.
                if p.text[range.clone()].contains(xenify_wp_doc::Field::CHAR) {
                    return Err(
                        "this part has a date, badge or other item in it, which a rewrite would lose: select just the words around it"
                            .into(),
                    );
                }
                (
                    para,
                    range.clone(),
                    p.text[range].to_string(),
                    Voice::of(&ed.doc),
                    String::new(),
                )
            };
            let body = xenify_wp_ai::rewrite_request(&model, &text, arg(args, "how"), &voice);
            let new = xenify_wp_ai::reply_text(&send(&body)?).trim().to_string();
            if new.is_empty() {
                return Err("the AI didn't send anything back; try again".into());
            }
            let diff: Vec<Value> = xenify_wp_ai::word_diff(&text, &new)
                .into_iter()
                .map(|p| match p {
                    xenify_wp_ai::Piece::Same(t) => json!({"same": t}),
                    xenify_wp_ai::Piece::Removed(t) => json!({"removed": t}),
                    xenify_wp_ai::Piece::Added(t) => json!({"added": t}),
                })
                .collect();
            Ok(json!({ "para": para.0, "start": range.start, "end": range.end, "old": text, "new": new, "diff": diff }))
        }
        "continue" => {
            let (before, voice, model) = {
                let d = lock()?;
                let ed = d.front_editor();
                let mut before = String::new();
                for p in ed.doc.paragraphs() {
                    if p.id == ed.sel.focus.para {
                        before.push_str(&p.text[..ed.sel.focus.offset.min(p.text.len())]);
                        break;
                    }
                    before.push_str(&p.text);
                    before.push_str("\n\n");
                }
                // The last few thousand characters are plenty to go on.
                let cut = before.char_indices().rev().nth(4000).map_or(0, |(i, _)| i);
                (before[cut..].to_string(), Voice::of(&ed.doc), String::new())
            };
            let body = xenify_wp_ai::continue_request(&model, &before, arg(args, "hint"), &voice);
            let text = xenify_wp_ai::reply_text(&send(&body)?);
            Ok(json!({ "text": text.trim_end() }))
        }
        "ask" | "summarise" => {
            let history: Vec<Message> = if name == "summarise" {
                let what = match scope(args) {
                    "whole" => "the whole document",
                    "paragraph" => "the paragraph the user is in (given as the selection)",
                    _ => "the selected text",
                };
                vec![Message {
                    role: "user".into(),
                    text: format!(
                        "Summarise {what} in a few sentences, for the readers it's written for. Don't propose any changes."
                    ),
                }]
            } else {
                serde_json::from_value(args.get("history").cloned().unwrap_or_default()).unwrap_or_default()
            };
            let (doc, selection, model, table) = {
                let d = lock()?;
                let ed = d.front_editor();
                let table = caret_table(ed).filter(|_| scope(args) != "whole");
                // In a table: the whole table (with the selected words too, if any).
                let in_table = table.and_then(|t| ed.doc.table(t)).map(|t| xenify_wp_ai::readable_table(&ed.doc, t));
                let sel = match scope(args) {
                    "whole" => String::new(),
                    _ if in_table.is_some() => {
                        let words = ops::copy_spans(ed)
                            .iter()
                            .map(|s| s.text.replace(xenify_wp_doc::Field::CHAR, ""))
                            .collect::<Vec<_>>()
                            .join(" ");
                        let t = in_table.unwrap_or_default();
                        if scope(args) == "selection" && !words.trim().is_empty() {
                            format!("{t}\n\nThe words selected in it: {words}")
                        } else {
                            t
                        }
                    }
                    "paragraph" => writing_para(ed)
                        .and_then(|id| ed.doc.para(id))
                        .map(|p| xenify_wp_ai::readable(&ed.doc, p))
                        .unwrap_or_default(),
                    _ => ops::copy_spans(ed)
                        .iter()
                        .map(|s| s.text.replace(xenify_wp_doc::Field::CHAR, ""))
                        .collect::<Vec<_>>()
                        .join("\n"),
                };
                (ed.doc.clone(), sel, String::new(), table)
            };
            if scope(args) != "whole" && selection.trim().is_empty() {
                return Err(
                    "there's no text there yet: write something, select some words, or choose The whole document"
                        .into(),
                );
            }
            let body = xenify_wp_ai::chat_request(
                &model,
                &doc,
                &selection,
                scope(args) == "whole",
                &history,
                &Voice::of(&doc),
            );
            let (reply, proposals) = xenify_wp_ai::parse_chat(&send(&body)?);
            // Table changes are for the table that was sent.
            let table = table.or_else(|| {
                // (The whole document: the caret's table, if it's in one.)
                let d = docs.lock().ok()?;
                caret_table(d.front_editor())
            });
            Ok(json!({ "reply": reply, "proposals": proposals, "table": table.map(|t| t.0) }))
        }
        _ => Err(format!("unknown AI call “{name}”")),
    }
}
