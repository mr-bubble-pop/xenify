//! `cargo run --release --example spot -- MODEL_DIR < text.txt`: prints the
//! spots in each line, plus load time and speed. Optional 2nd argument: threshold (default 0.9).
use std::io::BufRead;
use std::time::Instant;

fn main() {
    let dir = std::env::args().nth(1).expect("model folder");
    let threshold: f32 = std::env::args().nth(2).map_or(0.9, |t| t.parse().expect("threshold"));
    let t = Instant::now();
    let s = xenify_grammar_spotter::Spotter::load(std::path::Path::new(&dir)).expect("load");
    eprintln!("loaded in {:.2}s", t.elapsed().as_secs_f64());
    let (mut n, t) = (0, Instant::now());
    for line in std::io::stdin().lock().lines().map_while(Result::ok) {
        for f in s.spot(&line, threshold).unwrap() {
            println!("{} [{} {:.2}]", &line[f.range.clone()], f.code, f.confidence);
        }
        n += 1;
    }
    eprintln!("{n} lines, {:.0} ms each", t.elapsed().as_secs_f64() * 1000.0 / n.max(1) as f64);
    if let Ok(st) = std::fs::read_to_string("/proc/self/status") {
        for l in st.lines() {
            if let Some(v) = l.strip_prefix("VmHWM:") {
                eprintln!("peak memory: {}", v.trim());
            } else if let Some(v) = l.strip_prefix("VmRSS:") {
                eprintln!("memory now: {}", v.trim());
            }
        }
    }
}
