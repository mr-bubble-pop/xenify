//! The showcase: a presentation that shows what the program does, built
//! entirely with the program's own editing operations (every step is an
//! ordinary command, so it replays), with sounds, music and clips the
//! program makes itself. The product name is a parameter; nothing else
//! names it.
//!
//! ```text
//! xenify-cli showcase --name "Spark" showcase.xenify
//! ```

pub mod kit;
mod s01_open;
mod s02_time;
mod s03_vector;
mod s04_motion;
mod s05_slides;
mod s06_movie;
mod s07_effects;
mod s08_three;
mod s09_diagrams;
mod s10_export;
mod s11_finale;

use xenify_doc::Project;
use xenify_ops::Command;

/// What to build.
#[derive(Clone, Debug)]
pub struct Options {
    /// The product name, shown in the title and the finale.
    pub name: String,
    /// Render the movie-editing slide's clips to video (needs a GPU and
    /// ffmpeg). Without them, still pictures stand in.
    pub clips: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self { name: "Spark".into(), clips: true }
    }
}

/// The showcase for product `name`.
pub fn build(name: &str) -> Project {
    build_with(&Options { name: name.into(), ..Default::default() }).0
}

/// The showcase and the commands that made it (from `Project::new()`).
pub fn build_with(o: &Options) -> (Project, Vec<Command>) {
    let mut d = kit::Deck::new(&o.name);
    s01_open::build(&mut d);
    s02_time::build(&mut d);
    s03_vector::build(&mut d);
    s04_motion::build(&mut d);
    s05_slides::build(&mut d);
    s06_movie::build(&mut d, o.clips);
    s07_effects::build(&mut d);
    s08_three::build(&mut d);
    s09_diagrams::build(&mut d);
    s10_export::build(&mut d);
    s11_finale::build(&mut d);
    d.finish_slide();
    d.p.theme = None;
    (d.p, d.log)
}
