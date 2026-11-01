//! `cargo run --release --example suggest -- MODEL_DIR "Some text."`: shows
//! each spot, the candidate fixes tried (with what the model then spots) and
//! the fixes [`Spotter::suggest`] keeps.
use xenify_grammar_spotter::{Spotter, candidates};

fn main() {
    let dir = std::env::args().nth(1).unwrap();
    let text = std::env::args().nth(2).unwrap();
    let s = Spotter::load(std::path::Path::new(&dir)).unwrap();
    for f in s.spot(&text, 0.5).unwrap() {
        println!("SPOT {:?} {:?} {} {:.2}", f.range, &text[f.range.clone()], f.code, f.confidence);
        for c in candidates(&text, &f)
            .iter()
            .filter(|c| std::env::var("SHOW").map_or(true, |s| c.text.contains(s.as_str())))
            .take(80)
        {
            let mut t = text.clone();
            t.replace_range(c.range.clone(), &c.text);
            let sp: Vec<String> = s
                .spot(&t, 0.3)
                .unwrap()
                .iter()
                .map(|g| format!("{}:{}:{:.2}", &t[g.range.clone()], g.code, g.confidence))
                .collect();
            println!("  try {:?}@{:?} → {}  | {:?}", c.text, c.range, t, sp);
        }
        println!("  suggest: {:?}", s.suggest(&text, &f, 3).unwrap());
    }
}
