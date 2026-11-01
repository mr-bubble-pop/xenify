//! All integration tests in one test program: each file used to be its own
//! program linking everything again. Files stay where they are; run one file
//! with `cargo test -p xenify-wp-io --test wp_io <file>::`.

#[path = "datetime.rs"]
mod datetime;
#[path = "docx.rs"]
mod docx;
#[path = "equations.rs"]
mod equations;
#[path = "forms.rs"]
mod forms;
#[path = "frames.rs"]
mod frames;
#[path = "hostile.rs"]
mod hostile;
#[path = "html.rs"]
mod html;
#[path = "igdoc.rs"]
mod igdoc;
#[path = "markdown.rs"]
mod markdown;
#[path = "pdf.rs"]
mod pdf;
#[path = "references.rs"]
mod references;
#[path = "review.rs"]
mod review;
#[path = "table_look.rs"]
mod table_look;
#[path = "toc.rs"]
mod toc;
#[path = "word_fixtures.rs"]
mod word_fixtures;
#[path = "writing.rs"]
mod writing;
