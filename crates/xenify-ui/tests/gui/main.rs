//! Headless GUI tests: the whole app runs without a window or GPU, with fake
//! OS services (clipboard, file manager, printers). One test binary (each
//! integration test file would link its own copy of the app).

mod a11y;
mod ai;
mod camera;
mod canvas;
mod colour;
mod common;
mod diagram;
mod effects;
mod emoji;
mod fixes;
mod giphy;
mod import;
mod layers;
mod masks;
mod media;
mod menus;
mod modifiers;
mod particles;
mod precision;
mod settings;
mod shapes;
mod slides;
mod smoke;
mod tabs;
mod templates;
mod text_anim;
mod text_in_place;
mod timeline;
mod tools;
mod ux;
mod water;
