//! All integration tests in one test program: each file used to be its own
//! program linking everything again. Files stay where they are; run one file
//! with `cargo test -p xenify-export --test export_tests <file>::`.

#[path = "animated_image.rs"]
mod animated_image;
#[path = "diagram.rs"]
mod diagram;
#[path = "export.rs"]
mod export;
#[path = "lottie_import.rs"]
mod lottie_import;
#[path = "motion.rs"]
mod motion;
#[path = "provenance.rs"]
mod provenance;
#[path = "templates.rs"]
mod templates;
#[path = "video.rs"]
mod video;
