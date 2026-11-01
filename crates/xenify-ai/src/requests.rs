//! Ready-made requests for the app's AI menu (and the CLI): a new animation
//! or slide deck from a description, a new slide, a change to a slide.

/// Model round trips allowed for building a whole deck (the usual limit
/// is sized for one scene).
pub const DECK_STEPS: usize = 150;

/// Text sizes that read well on a slide `height` pixels tall.
fn type_scale(height: u32) -> String {
    let h = height as f64;
    format!(
        "Use the whole slide and big, readable text: titles font_size ≈ {:.0} px, body text ≈ {:.0} px, small \
         labels at least {:.0} px; keep margins of about {:.0} px",
        h * 0.075,
        h * 0.04,
        h * 0.028,
        h * 0.07
    )
}

/// Asks for a new animation built in the open (empty) composition.
pub fn new_animation(description: &str, width: u32, height: u32) -> String {
    format!(
        "Create this in the new, empty {width}×{height} composition. Build it straight away unless something \
         essential is missing.\n\n{}",
        description.trim()
    )
}

/// Asks for a whole slide deck, starting in the open (empty) composition.
pub fn new_deck(description: &str, width: u32, height: u32) -> String {
    format!(
        "Make a slide deck (a presentation) in this new, empty {width}×{height} project from the description \
         below. Build slide 1 (usually a title slide) in the open composition, then add_slide for each further \
         slide. Unless the description says how many, make 5–8 slides.\n\
         - Give every slide the same visual style, a short build-in animation, a transition and speaker notes.\n\
         - {}.\n\
         - Work efficiently: build each slide in one or two responses (all its layers at once, then all its \
         animation at once). Prefer simple layouts that can't collide (a title plus a bullet list, two columns, \
         a few big numbers) over fiddly diagrams, and don't spend more than one extra step fixing small layout \
         warnings on a slide: every slide must get built.\n\
         - Keep decorative shapes (bars, blocks, lines) clear of text: in the margins or behind nothing.\n\
         Build it straight away unless something essential is missing.\n\n{}",
        type_scale(height),
        description.trim()
    )
}

/// Asks for content on a slide that was just added and opened.
/// `slide` is its index (0-based) when the project is a presentation.
pub fn new_slide(description: &str, slide: Option<usize>, height: u32) -> String {
    format!(
        "I've just added {} and it's empty. Build its content from the description below, matching the style of \
         the other slides (background, palette, fonts, title position, margins, how things animate in); look at \
         another slide with open_slide if you need to, but finish on this new slide. {}. Give it a transition \
         like the others and short speaker notes.\n\n{}",
        which(slide),
        type_scale(height),
        description.trim()
    )
}

/// Asks for a change to the open slide.
pub fn modify_slide(description: &str, slide: Option<usize>) -> String {
    format!(
        "Change {} as described below. Only change what's asked; keep the rest of the slide as it is.\n\n{}",
        which(slide),
        description.trim()
    )
}

fn which(slide: Option<usize>) -> String {
    slide.map_or("the open composition".to_string(), |i| format!("slide {} (the open composition)", i + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_say_what_to_build() {
        let deck = new_deck(" Our Q3 results ", 1920, 1080);
        assert!(deck.contains("add_slide") && deck.ends_with("Our Q3 results"));
        assert!(deck.contains("titles font_size ≈ 81 px"), "{deck}");
        assert!(new_animation("a logo reveal", 800, 600).contains("800×600"));
        assert!(new_slide("Pricing", Some(2), 1080).contains("slide 3"));
        assert!(modify_slide("bigger", None).contains("the open composition"));
    }
}
