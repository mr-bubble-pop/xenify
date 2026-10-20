//! `cargo run --example sounds < text.txt`: for every sound-alike word in each
//! line, what the context model prefers and how strongly (margin, support).
//! The checker flags a word when margin ≥ 10 and support ≥ 3.
use xenify_grammar::sound_alike::{Model, set_of, tagged};
use std::io::BufRead;

fn main() {
    let model = Model::bundled();
    for line in std::io::stdin().lock().lines().map_while(Result::ok) {
        let toks = tagged(&line);
        for (t, (_, w, _)) in toks.iter().enumerate() {
            let Some(set) = set_of(w) else { continue };
            match model.better(set, &toks, t) {
                Some((best, margin, support)) => println!("{w:>10} → {best:<10} margin {margin:5.1} support {support:2}  | {line}"),
                None => println!("{w:>10}   (context agrees)              | {line}"),
            }
        }
    }
}
