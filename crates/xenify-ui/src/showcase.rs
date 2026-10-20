//! The showcase presentation (built by `xenify-cli showcase`, shipped as
//! assets/showcase.xenify): opened at first launch behind the New dialog, from
//! the launch column's Examples, and from Help › Showcase.

use crate::app::XenifyApp;
use xenify_doc::Project;

const DATA: &str = include_str!("../../../assets/showcase.xenify");

/// The showcase, read from the copy built into the program.
pub fn project() -> Option<Project> {
    Project::from_json(DATA).ok()
}

/// Opens the showcase in a tab (replacing an untouched document).
pub fn open(app: &mut XenifyApp) {
    let Some(p) = project() else {
        app.fail("The showcase couldn't be read.");
        return;
    };
    let first = p.slides.first().map(|s| s.comp);
    app.open_document(p, None);
    if let Some(c) = first {
        app.open_comp(c);
    }
    crate::dock::reveal(app, crate::dock::Panel::Slides);
    let key = app.keymap.text(crate::keymap::Action::Present);
    app.status =
        format!("The showcase: {key} presents it (click or → to go on at each pause). Everything in it is editable.");
}
