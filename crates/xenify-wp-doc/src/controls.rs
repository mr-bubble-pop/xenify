//! Form controls from Word documents (content controls and the older form
//! fields): text boxes, dates, drop-down lists and combo boxes to fill in,
//! and checkboxes to tick. A control's text carries [`CharFmt::control`]
//! (an id in [`Meta::controls`]); a checkbox is a [`Field::Check`].

use crate::edit::{EditError, Editor, Step};
use crate::{CharFmt, Field, NodeId, Pos, Selection, Span};
use serde::{Deserialize, Serialize};

/// What a control holds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlKind {
    /// Plain text (one style), or `rich` text (any formatting).
    Text {
        #[serde(default)]
        rich: bool,
    },
    /// A date, shown in `format` (Word's date format, e.g. "d/MM/yyyy").
    Date {
        #[serde(default)]
        format: String,
    },
    /// Pick one of `items`.
    Dropdown { items: Vec<String> },
    /// Pick one of `items`, or type something else.
    ComboBox { items: Vec<String> },
}

/// One control in the document.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Control {
    pub id: NodeId,
    pub kind: ControlKind,
    /// Its title (Word's "alias"), shown on hover.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    /// Word's tag (for whoever made the form).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tag: String,
    /// The text is a prompt ("Click to enter a date"), shown grey; typing replaces it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub placeholder: bool,
}

impl crate::Document {
    pub fn control(&self, id: NodeId) -> Option<&Control> {
        self.meta.controls.iter().find(|c| c.id == id)
    }
}

/// The control at `pos` and the byte range of its text in that paragraph.
pub fn control_at(ed: &Editor, pos: Pos) -> Option<(Control, std::ops::Range<usize>)> {
    let p = ed.doc.para(pos.para)?;
    let ranges: Vec<(std::ops::Range<usize>, NodeId)> =
        p.run_ranges().filter_map(|(r, f)| f.control.map(|c| (r, c))).collect();
    let (_, id) = ranges.iter().find(|(r, _)| r.start <= pos.offset && pos.offset <= r.end)?.clone();
    // The control's whole text: the touching runs that carry it.
    let start = ranges.iter().filter(|(_, c)| *c == id).map(|(r, _)| r.start).min()?;
    let end = ranges.iter().filter(|(_, c)| *c == id).map(|(r, _)| r.end).max()?;
    Some((ed.doc.control(id)?.clone(), start..end))
}

/// Ticks or unticks the checkbox at `pos` (one undo step). Returns false when there's none.
pub fn toggle_check(ed: &mut Editor, pos: Pos) -> Result<bool, EditError> {
    let Some(p) = ed.doc.para(pos.para) else { return Ok(false) };
    let at = pos.offset;
    let fmt = p.fmt_at(at + Field::CHAR.len_utf8());
    let Some(Field::Check { checked }) = fmt.field.clone() else { return Ok(false) };
    if p.text.get(at..at + Field::CHAR.len_utf8()) != Some(Field::STR) {
        return Ok(false);
    }
    let new = CharFmt { field: Some(Field::Check { checked: !checked }), ..fmt };
    let para = pos.para;
    let sel = ed.sel;
    ed.seal();
    ed.transact(if checked { "Untick" } else { "Tick" }, None, |tx| {
        tx.step(Step::Replace {
            para,
            offset: at,
            remove: Field::CHAR.len_utf8(),
            insert: Span::plain(Field::STR, new),
        })?;
        Ok(sel)
    })?;
    ed.seal();
    Ok(true)
}

/// Sets the text of the control at `pos` to `value` (picking from a list,
/// or replacing a prompt), keeping its formatting. One undo step.
pub fn set_value(ed: &mut Editor, pos: Pos, value: &str) -> Result<bool, EditError> {
    let Some((control, range)) = control_at(ed, pos) else { return Ok(false) };
    let Some(p) = ed.doc.para(pos.para) else { return Ok(false) };
    let fmt = p.fmt_at(range.start + usize::from(!range.is_empty()));
    let para = pos.para;
    let id = control.id;
    ed.seal();
    ed.transact("Fill in", None, |tx| {
        tx.step(Step::Replace { para, offset: range.start, remove: range.len(), insert: Span::plain(value, fmt) })?;
        if control.placeholder {
            let mut meta = tx.doc.meta.clone();
            if let Some(c) = meta.controls.iter_mut().find(|c| c.id == id) {
                c.placeholder = false;
            }
            tx.step(Step::SetMeta { meta: Box::new(meta) })?;
        }
        Ok(Selection::caret(Pos::new(para, range.start + value.len())))
    })?;
    ed.seal();
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Document, Paragraph, Run};

    fn form() -> (Editor, NodeId) {
        let mut doc = Document::new();
        let para = doc.paragraphs().next().unwrap().id;
        let ctl = NodeId(900);
        doc.meta.controls.push(Control {
            id: ctl,
            kind: ControlKind::Dropdown { items: vec!["Red".into(), "Blue".into()] },
            title: "Colour".into(),
            tag: String::new(),
            placeholder: true,
        });
        let p: &mut Paragraph = doc.para_mut(para).unwrap();
        p.text = format!("Pick: Choose an item. {}", Field::STR);
        p.runs = vec![
            Run { len: 6, fmt: CharFmt::default() },
            Run { len: 15, fmt: CharFmt { control: Some(ctl), ..Default::default() } },
            Run { len: 1, fmt: CharFmt::default() },
            Run {
                len: Field::CHAR.len_utf8(),
                fmt: CharFmt { field: Some(Field::Check { checked: false }), ..Default::default() },
            },
        ];
        (Editor::new(doc), para)
    }

    #[test]
    fn pick_a_value_and_tick_a_box() {
        let (mut ed, para) = form();
        let (c, r) = control_at(&ed, Pos::new(para, 8)).unwrap();
        assert_eq!(c.title, "Colour");
        assert_eq!(&ed.doc.para(para).unwrap().text[r], "Choose an item.");
        assert!(set_value(&mut ed, Pos::new(para, 8), "Blue").unwrap());
        assert_eq!(ed.doc.para(para).unwrap().text, format!("Pick: Blue {}", Field::STR));
        assert!(!ed.doc.meta.controls[0].placeholder, "a real value now");
        let at = ed.doc.para(para).unwrap().text.find(Field::STR).unwrap();
        assert!(toggle_check(&mut ed, Pos::new(para, at)).unwrap());
        let f = ed.doc.para(para).unwrap().fmt_at(at + 3).field;
        assert_eq!(f, Some(Field::Check { checked: true }));
        ed.undo();
        ed.undo();
        assert_eq!(ed.doc.para(para).unwrap().text, format!("Pick: Choose an item. {}", Field::STR));
        assert!(ed.doc.meta.controls[0].placeholder);
    }
}
