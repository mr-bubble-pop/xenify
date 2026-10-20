//! Headless AI runs: the full agent loop with real models, no UI.

use xenify_ai::{Agent, AiSettings, ChatItem, Next, OpenRouter, Purpose, ToolHost};
use xenify_anim::{SceneText, TextMeasure};
use xenify_doc::{Id, Project};
use xenify_ops::{Command, Editor};
use xenify_render::Offscreen;
use std::time::Instant;

struct CliHost {
    editor: Editor,
    comp: Id,
    gpu: Offscreen,
}

impl ToolHost for CliHost {
    fn project(&self) -> &Project {
        self.editor.project()
    }
    fn comp(&self) -> Id {
        self.comp
    }
    fn set_comp(&mut self, comp: Id) -> Result<(), String> {
        self.editor.project().comp(comp).map_err(|e| e.to_string())?;
        self.comp = comp;
        Ok(())
    }
    fn time(&self) -> f64 {
        0.0
    }
    fn execute(&mut self, label: &str, cmd: Command) -> Result<(), String> {
        self.editor.execute(label, cmd).map_err(|e| e.to_string())
    }
    fn text_bounds(&mut self, text: &SceneText) -> kurbo::Rect {
        self.gpu.text_engine().text_bounds(text)
    }
    fn trace_image(&mut self, image: &[u8], colors: u8) -> Result<Vec<xenify_doc::Layer>, String> {
        let s = &self.editor.project().comp(self.comp).map_err(|e| e.to_string())?.settings;
        let opts = xenify_export::trace::TraceOptions { colors, ..Default::default() };
        xenify_export::trace::trace_to_layers(image, &opts, self.editor.project().next_id, s.width, s.height)
    }
    fn import_svg(&mut self, svg: &[u8]) -> Result<Vec<xenify_doc::Layer>, String> {
        let s = &self.editor.project().comp(self.comp).map_err(|e| e.to_string())?.settings;
        xenify_export::svg::import_svg(svg, self.editor.project().next_id, s.width, s.height)
    }
    fn render_png(&mut self, time: f64) -> Result<Vec<u8>, String> {
        render_preview(&mut self.gpu, self.editor.project(), self.comp, time)
    }
}

fn render_preview(gpu: &mut Offscreen, project: &Project, comp: Id, time: f64) -> Result<Vec<u8>, String> {
    gpu.preview_png(project, comp, time, 768).map_err(|e| e.to_string())
}

pub fn run(
    project: Project,
    prompt: &str,
    settings: AiSettings,
    answer: &str,
    steps: Option<usize>,
) -> Result<Project, Box<dyn std::error::Error>> {
    let transport = OpenRouter::new(&settings)?;
    eprintln!(
        "brain: {}   reviewer: {}",
        settings.brain_model,
        if settings.reviewer_model.is_empty() { "(none)" } else { &settings.reviewer_model }
    );
    let comp = project.compositions[0].id;
    let mut host = CliHost { editor: Editor::new(project), comp, gpu: Offscreen::new()? };
    let mut agent = Agent::new(settings);
    if let Some(n) = steps {
        agent.allow_steps_next_turn(n);
    }
    let start = Instant::now();
    let (mut requests, mut waiting, mut sent_bytes) = (0usize, 0.0f64, 0usize);
    let mut shown = 0;
    let mut streamed = String::new();
    let mut next = agent.send(prompt, &mut host);
    loop {
        for item in &agent.transcript[shown..] {
            match item {
                ChatItem::User(t) => eprintln!("\n> {t}"),
                // Already printed while it streamed in.
                ChatItem::Assistant(t) if t.trim() == streamed.trim() => {}
                ChatItem::Assistant(t) => eprintln!("\nassistant: {t}"),
                ChatItem::Activity { text, ok } => eprintln!("  {} {text}", if *ok { "✓" } else { "✗" }),
                ChatItem::Question { text, options, .. } => eprintln!("  ? {text} {options:?}"),
                ChatItem::Review { time, text, .. } => eprintln!("  👁 reviewer @ {time:.2}s: {text}"),
                ChatItem::Generated { text, .. } => eprintln!("  🖼 {text}"),
                ChatItem::Error(e) => eprintln!("  ! {e}"),
            }
        }
        shown = agent.transcript.len();
        next = match next {
            Next::Request { body, purpose } => {
                streamed.clear();
                requests += 1;
                let t0 = Instant::now();
                let sent = body.to_string().len();
                let result = if matches!(purpose, Purpose::Brain) {
                    transport.chat_stream(&body, &mut |t| {
                        // Models often start with blank lines.
                        let t = if streamed.is_empty() { t.trim_start() } else { t };
                        if t.is_empty() {
                            return;
                        }
                        if streamed.is_empty() {
                            eprint!("\nassistant: ");
                        }
                        streamed.push_str(t);
                        eprint!("{t}");
                    })
                } else {
                    transport.chat(&body)
                };
                if !streamed.is_empty() {
                    eprintln!();
                }
                waiting += t0.elapsed().as_secs_f64();
                sent_bytes += sent;
                agent.handle_response(purpose, result, &mut host)
            }
            Next::WaitForUser => agent.send(answer, &mut host),
            Next::Idle => break,
        };
    }
    let u = agent.usage;
    eprintln!(
        "\n{:.1}s · {} prompt + {} completion tokens · ${:.5}",
        start.elapsed().as_secs_f64(),
        u.prompt_tokens,
        u.completion_tokens,
        u.cost
    );
    eprintln!(
        "{requests} model requests · {:.1}s waiting on the model · {:.0} KB sent per request on average",
        waiting,
        sent_bytes as f64 / requests.max(1) as f64 / 1024.0
    );
    Ok(host.editor.project().clone())
}
