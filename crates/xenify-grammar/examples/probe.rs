//! `cargo run -p xenify-grammar --example probe -- "text"`: every issue the
//! checker reports for each argument, with its rule and suggestions.
use xenify_grammar::{Checker, Settings};

fn main() {
    let mut c = Checker::new(Settings::default(), &[]);
    for text in std::env::args().skip(1) {
        println!("{text}");
        for i in c.check(&text) {
            println!(
                "  {:?} {:?} [{}] {} → {:?}",
                &text[i.range.clone()],
                i.category,
                i.rule,
                i.message,
                i.suggestions
            );
        }
    }
}
