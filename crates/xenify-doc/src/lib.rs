//! The Xenify Draw document model.
//!
//! A [`Project`] holds [`Composition`]s; a composition holds [`Layer`]s in paint
//! order (index 0 is drawn first, i.e. it is the bottom-most layer). Every value
//! that can change over time is a [`Property`], which is either static or a list
//! of [`Keyframe`]s.
//!
//! This crate is pure data + serde. It must not touch the filesystem, processes
//! or threads so that it keeps compiling for `wasm32-unknown-unknown`.

mod asset;
pub mod blobs;
pub mod file;
pub mod bezier;
mod camera;
mod color;
mod connector;
mod distort;
mod easing;
mod effects;
pub mod generators;
mod id;
mod layer;
mod masks;
pub mod media;
mod mesh;
mod modifiers;
mod path;
mod pen;
pub mod primitives;
mod project;
mod property;
mod sample;
mod slides;
mod solid;
pub mod templates;
pub mod text_anim;
mod units;

pub use asset::{ImageAsset, ImagePlayback, ModelSource, hash_bytes};
pub use camera::{CameraLayer, CameraShake, active_camera, camera_layer};
pub use color::Color;
pub use connector::{
    Attach, Connector, ConnectorEnd, DEFAULT_PORTS, DiagramSettings, Flow, FlowKind, JumpStyle, Marker, MarkerKind,
    NodeLabel, Port, PortRef, PortSide, RouteStyle,
};
pub use distort::{Distort, Homography, Quad};
pub use easing::{Easing, LinearStops, StepJump};
pub use effects::{BlendMode, Effect, EffectField, EffectKind, Fx, adjust_color};
pub use generators::{
    EmitMode, Emitter, EmitterField, EmitterShape, EndLook, FIRE_PRESETS, LiquidField, LiquidFill, PARTICLE_PRESETS,
    ParticleShape,
};
pub use id::Id;
pub use layer::{
    Expression, Fill, FillRule, Geometry, Gradient, GradientKind, GradientPaint, GradientStop, Layer, LayerKind,
    LayerMeta, LineCap, LineJoin, Matte, MatteMode, Pattern, Rig, ShapeItem, Stroke, TextAlign, TextFrame, TextLayer,
    TextPath, TextVAlign, Transform, Trim, Wiggle,
};
pub use masks::{Mask, MaskMode, as_rect, rect_path};
pub use media::{AudioLayer, MediaAsset, MediaKind, MediaSource, Soundtrack, SoundtrackClip, VideoLayer};
pub use mesh::{Faces, HOLE, Mesh, MeshFace, MeshPart};
pub use modifiers::{
    ArrayStep, CopyPlan, Deform, Evaluated, MAX_COPIES, Modifier, ModifierField, ModifierKind, copies, plan, plan_with,
    reflection,
};
pub use path::{AnyProperty, CameraField, MaskField, PartField, PropPath, ShapeField, SolidField, Value, ValueKind};
pub use pen::{PenKind, PenParams, PenStyle};
pub use primitives::Primitive;
pub use project::{CompKind, CompSettings, Composition, MotionBlur, Project, ProjectError, Slice};
pub use property::{Animatable, Keyframe, Property, TIME_EPSILON};
pub use sample::demo_project;
pub use slides::{Direction, Slide, Transition, TransitionKind};
pub use solid::{PartMotion, Solid, SolidShape};
pub use templates::{Content, Layout, Role, THEMES, Theme};
pub use text_anim::{GlyphState, TextAnimator, TextOrder, TextUnit};
pub use units::{GuideLine, Unit, Units};

pub use kurbo::{self, BezPath, Point, Vec2};
