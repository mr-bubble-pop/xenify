use xenify_doc::{
    AnyProperty, Color, CompSettings, Easing, Id, Layer, LayerKind, LayerMeta, Project, PropPath, ShapeField,
    ShapeItem, TextLayer, Value, Vec2,
};
use xenify_ops::{Command, Editor, OpError, apply};

struct Fixture {
    project: Project,
    comp: Id,
    a: Id,
    b: Id,
    text: Id,
}

fn fixture() -> Fixture {
    let mut project = Project::new();
    let comp = project.compositions[0].id;
    let a = project.alloc_id();
    let b = project.alloc_id();
    let text = project.alloc_id();
    let layers = vec![
        Layer::shape(a, "A", vec![ShapeItem::rect(Vec2::new(100.0, 50.0), Color::WHITE)]),
        Layer::shape(b, "B", vec![ShapeItem::ellipse(Vec2::new(40.0, 40.0), Color::BLACK)]),
        Layer::new(text, "Title", LayerKind::Text(TextLayer::new("Hello", "Inter", 48.0, Color::WHITE))),
    ];
    project.comp_mut(comp).unwrap().layers = layers;
    Fixture { project, comp, a, b, text }
}

/// Applies `cmd`, checks the inverse restores the original exactly, and that
/// re-applying the inverse's inverse gets back to the edited state.
///
/// `next_id` is deliberately not rolled back by undo (ids are never reused), so
/// it is excluded from the comparison.
fn round_trip(project: &Project, cmd: Command) -> Project {
    let mut project = project.clone();
    project.next_id = 1000;
    let project = &project;
    let mut p = project.clone();
    let inverse = apply(&mut p, cmd.clone()).unwrap_or_else(|e| panic!("{cmd:?} failed: {e}"));
    let edited = p.clone();
    assert_ne!(&edited, project, "{cmd:?} changed nothing");
    let redo = apply(&mut p, inverse).unwrap();
    assert_eq!(&p, project, "undo of {cmd:?} did not restore the project");
    apply(&mut p, redo).unwrap();
    assert_eq!(p, edited, "redo of {cmd:?} did not reproduce the edit");
    edited
}

#[test]
fn every_command_round_trips() {
    let f = fixture();
    let size = PropPath::shape(0, ShapeField::Size);
    let mut new_id = f.project.clone();
    let c = new_id.alloc_id();

    let animated = round_trip(
        &f.project,
        Command::SetKeyframe {
            layer: f.a,
            path: PropPath::Position,
            time: 0.0,
            value: Value::Vec2(Vec2::ZERO),
            easing: None,
        },
    );
    let two_keys = round_trip(
        &animated,
        Command::SetKeyframe {
            layer: f.a,
            path: PropPath::Position,
            time: 1.0,
            value: Value::Vec2(Vec2::new(10.0, 0.0)),
            easing: Some(Easing::EASE_OUT),
        },
    );

    let cmds = vec![
        Command::AddLayer { comp: f.comp, index: Some(1), layer: Layer::new(c, "Null", LayerKind::Null) },
        Command::AddLayer { comp: f.comp, index: None, layer: Layer::new(c, "Null", LayerKind::Null) },
        Command::RemoveLayer { layer: f.b },
        Command::MoveLayer { layer: f.a, index: 2 },
        Command::SetLayerMeta { layer: f.a, meta: LayerMeta { parent: Some(f.b), ..LayerMeta::named("A2") } },
        Command::SetProperty { layer: f.a, path: PropPath::Rotation, property: AnyProperty::Scalar(90.0.into()) },
        Command::SetValue { layer: f.a, path: size, time: 0.0, value: Value::Vec2(Vec2::new(1.0, 2.0)) },
        Command::SetValue { layer: f.text, path: PropPath::TextColor, time: 0.0, value: Value::Color(Color::BLACK) },
        Command::AddShape { layer: f.b, index: Some(0), shape: ShapeItem::rect(Vec2::new(5.0, 5.0), Color::WHITE) },
        Command::RemoveShape { layer: f.a, index: 0 },
        Command::SetText { layer: f.text, text: "World".into() },
        Command::SetShape { layer: f.a, index: 0, shape: ShapeItem::ellipse(Vec2::new(3.0, 3.0), Color::BLACK) },
        Command::SetFontFamily { layer: f.text, family: "DejaVu Sans".into() },
        Command::SetTextAlign {
            layer: f.text,
            align: xenify_doc::TextAlign::Center,
            valign: xenify_doc::TextVAlign::Middle,
        },
        Command::SetCompSettings { comp: f.comp, settings: CompSettings { fps: 60.0, ..Default::default() } },
        Command::batch(vec![
            Command::SetText { layer: f.text, text: "1".into() },
            Command::SetText { layer: f.text, text: "2".into() },
            Command::RemoveLayer { layer: f.a },
        ]),
    ];
    for cmd in cmds {
        round_trip(&f.project, cmd);
    }

    let keyed = vec![
        Command::RemoveKeyframe { layer: f.a, path: PropPath::Position, time: 1.0 },
        Command::MoveKeyframe { layer: f.a, path: PropPath::Position, from: 1.0, to: 2.5 },
        Command::SetEasing { layer: f.a, path: PropPath::Position, time: 0.0, easing: Easing::Hold },
        Command::SetValue { layer: f.a, path: PropPath::Position, time: 0.5, value: Value::Vec2(Vec2::new(3.0, 3.0)) },
    ];
    for cmd in keyed {
        round_trip(&two_keys, cmd);
    }
}

#[test]
fn removing_a_parent_unparents_children_and_undo_restores() {
    let f = fixture();
    let mut p = f.project.clone();
    apply(&mut p, Command::SetLayerMeta { layer: f.a, meta: LayerMeta { parent: Some(f.b), ..LayerMeta::named("A") } })
        .unwrap();
    let before = p.clone();
    let inv = apply(&mut p, Command::RemoveLayer { layer: f.b }).unwrap();
    assert_eq!(p.layer(f.a).unwrap().meta.parent, None);
    apply(&mut p, inv).unwrap();
    assert_eq!(p, before);
}

#[test]
fn invalid_commands_leave_project_untouched() {
    let f = fixture();
    let bad = vec![
        Command::AddLayer { comp: f.comp, index: None, layer: Layer::new(f.a, "dup", LayerKind::Null) },
        Command::AddLayer { comp: Id(999), index: None, layer: Layer::new(Id(500), "x", LayerKind::Null) },
        Command::RemoveLayer { layer: Id(999) },
        Command::MoveLayer { layer: f.a, index: 99 },
        Command::SetLayerMeta { layer: f.a, meta: LayerMeta { parent: Some(f.a), ..LayerMeta::named("A") } },
        Command::SetLayerMeta {
            layer: f.a,
            meta: LayerMeta { in_point: 2.0, out_point: Some(1.0), ..LayerMeta::named("A") },
        },
        Command::SetValue { layer: f.a, path: PropPath::Opacity, time: 0.0, value: Value::Color(Color::BLACK) },
        Command::SetKeyframe {
            layer: f.a,
            path: PropPath::Opacity,
            time: -1.0,
            value: Value::Scalar(1.0),
            easing: None,
        },
        Command::SetValue { layer: f.a, path: PropPath::FontSize, time: 0.0, value: Value::Scalar(1.0) },
        Command::RemoveKeyframe { layer: f.a, path: PropPath::Opacity, time: 0.0 },
        Command::MoveKeyframe { layer: f.a, path: PropPath::Opacity, from: 0.0, to: 1.0 },
        Command::RemoveShape { layer: f.text, index: 0 },
        Command::SetText { layer: f.a, text: "no".into() },
        Command::SetFontFamily { layer: f.a, family: "no".into() },
        Command::SetCompSettings { comp: f.comp, settings: CompSettings { width: 0, ..Default::default() } },
        // Fails on the second command; the first must be rolled back.
        Command::batch(vec![
            Command::SetText { layer: f.text, text: "changed".into() },
            Command::RemoveLayer { layer: Id(999) },
        ]),
    ];
    for cmd in bad {
        let mut p = f.project.clone();
        assert!(apply(&mut p, cmd.clone()).is_err(), "{cmd:?} should fail");
        assert_eq!(p, f.project, "{cmd:?} modified the project despite failing");
    }
}

#[test]
fn parent_cycles_are_rejected() {
    let f = fixture();
    let mut p = f.project.clone();
    apply(&mut p, Command::SetLayerMeta { layer: f.a, meta: LayerMeta { parent: Some(f.b), ..LayerMeta::named("A") } })
        .unwrap();
    let err = apply(
        &mut p,
        Command::SetLayerMeta { layer: f.b, meta: LayerMeta { parent: Some(f.a), ..LayerMeta::named("B") } },
    )
    .unwrap_err();
    assert!(matches!(err, OpError::Invalid(m) if m.contains("cycle")));
}

#[test]
fn editor_undo_redo_groups_and_merging() {
    let f = fixture();
    let mut ed = Editor::new(f.project.clone());
    let set_pos = |x: f64| Command::SetValue {
        layer: f.a,
        path: PropPath::Position,
        time: 0.0,
        value: Value::Vec2(Vec2::new(x, 0.0)),
    };

    // A drag gesture: many commands, one undo step.
    for x in 1..=10 {
        ed.execute_merged("Move A", "drag-a", set_pos(x as f64)).unwrap();
    }
    ed.execute("Rename", Command::SetText { layer: f.text, text: "Hi".into() }).unwrap();
    assert!(ed.is_dirty());

    assert_eq!(ed.undo().as_deref(), Some("Rename"));
    assert_eq!(ed.undo().as_deref(), Some("Move A"));
    assert_eq!(ed.project(), &f.project);
    assert_eq!(ed.undo(), None);

    assert_eq!(ed.redo().as_deref(), Some("Move A"));
    assert_eq!(ed.project().layer(f.a).unwrap().transform.position.value_at(0.0), Vec2::new(10.0, 0.0));

    // Grouped (e.g. an AI change): one undo step for everything.
    ed.begin_group("AI: intro");
    ed.execute("x", set_pos(50.0)).unwrap();
    ed.begin_group("nested");
    ed.execute("y", Command::RemoveLayer { layer: f.b }).unwrap();
    ed.end_group();
    ed.end_group();
    assert_eq!(ed.undo_label(), Some("AI: intro"));
    ed.undo();
    assert_eq!(ed.project().layer(f.a).unwrap().transform.position.value_at(0.0), Vec2::new(10.0, 0.0));
    assert!(ed.project().layer(f.b).is_ok());

    // Cancelled group reverts and leaves no history.
    let before = ed.project().clone();
    ed.begin_group("rejected");
    ed.execute("z", Command::RemoveLayer { layer: f.a }).unwrap();
    ed.cancel_group();
    assert_eq!(ed.project(), &before);
    assert_eq!(ed.undo_label(), Some("Move A"));

    // Failed command: no history entry, redo stack intact.
    assert!(ed.execute("bad", Command::RemoveLayer { layer: Id(999) }).is_err());
    assert_eq!(ed.undo_label(), Some("Move A"));

    ed.mark_saved();
    assert!(!ed.is_dirty());
}

#[test]
fn commands_have_readable_json() {
    let json = r#"{"op":"set_keyframe","layer":2,"path":"shapes.0.fill_color","time":1.5,
                   "value":{"r":1,"g":0.5,"b":0},"easing":{"type":"bezier","x1":0.42,"y1":0,"x2":0.58,"y2":1}}"#;
    let cmd: Command = serde_json::from_str(json).unwrap();
    assert_eq!(
        cmd,
        Command::SetKeyframe {
            layer: Id(2),
            path: PropPath::shape(0, ShapeField::FillColor),
            time: 1.5,
            value: Value::Color(Color::rgb(1.0, 0.5, 0.0)),
            easing: Some(Easing::EASE_IN_OUT),
        }
    );
    let back: Command = serde_json::from_str(&serde_json::to_string(&cmd).unwrap()).unwrap();
    assert_eq!(back, cmd);
}

#[test]
fn edit_value_auto_key() {
    let f = fixture();
    let v = |x: f64| Value::Vec2(Vec2::new(x, 0.0));

    // Auto-key off: plain value change, nothing animated.
    let mut p = f.project.clone();
    let cmd = Command::edit_value(&p, f.a, PropPath::Position, 1.0, v(50.0), false).unwrap();
    apply(&mut p, cmd).unwrap();
    assert!(!p.layer(f.a).unwrap().transform.position.is_animated());

    // Auto-key on at 1s: old value keyed at 0, new value at 1s.
    let mut p = f.project.clone();
    let cmd = Command::edit_value(&p, f.a, PropPath::Position, 1.0, v(50.0), true).unwrap();
    apply(&mut p, cmd).unwrap();
    let pos = &p.layer(f.a).unwrap().transform.position;
    assert_eq!(pos.keyframes().len(), 2);
    assert_eq!(pos.value_at(0.0), Vec2::ZERO);
    assert_eq!(pos.value_at(1.0), Vec2::new(50.0, 0.0));

    // Already animated: just keys the current time.
    let cmd = Command::edit_value(&p, f.a, PropPath::Position, 2.0, v(80.0), true).unwrap();
    assert!(matches!(cmd, Command::SetValue { .. }));
    apply(&mut p, cmd).unwrap();
    assert_eq!(p.layer(f.a).unwrap().transform.position.keyframes().len(), 3);

    // Auto-key at time 0: a single keyframe, no duplicate.
    let mut p = f.project.clone();
    let cmd = Command::edit_value(&p, f.a, PropPath::Position, 0.0, v(5.0), true).unwrap();
    apply(&mut p, cmd).unwrap();
    assert_eq!(p.layer(f.a).unwrap().transform.position.keyframes().len(), 1);
}

#[test]
fn key_all_properties() {
    let f = fixture();
    let mut p = f.project.clone();
    let pos = |x: f64| Value::Vec2(Vec2::new(x, 0.0));
    let cmd = Command::edit_value(&p, f.a, PropPath::Position, 2.0, pos(100.0), true).unwrap();
    apply(&mut p, cmd).unwrap();
    apply(&mut p, Command::SetEasing { layer: f.a, path: PropPath::Position, time: 0.0, easing: Easing::EASE_OUT })
        .unwrap();

    // Animated only: position gets a key at 1 s with its in-between value; nothing else animates.
    let cmd = Command::key_all(&p, &[f.a, f.b], 1.0, false).unwrap().unwrap();
    let q = round_trip(&p, cmd);
    let a = q.layer(f.a).unwrap();
    assert_eq!(a.keyframe_times(PropPath::Position).unwrap(), vec![0.0, 1.0, 2.0]);
    assert_eq!(
        a.value_at(PropPath::Position, 1.0).unwrap(),
        p.layer(f.a).unwrap().value_at(PropPath::Position, 1.0).unwrap()
    );
    assert!(!a.transform.rotation.is_animated());
    assert!(Command::key_all(&p, &[f.b], 1.0, false).unwrap().is_none());

    // Everything: every property of both layers is keyed; existing easing is kept.
    let cmd = Command::key_all(&p, &[f.a, f.b], 0.0, true).unwrap().unwrap();
    let q = round_trip(&p, cmd);
    for id in [f.a, f.b] {
        let l = q.layer(id).unwrap();
        for path in l.property_paths() {
            assert!(l.property(path).unwrap().is_animated(), "{path} on {id:?}");
        }
    }
    let a = q.layer(f.a).unwrap();
    assert_eq!(a.property(PropPath::Position).unwrap().easing_at(0.0), Some(Easing::EASE_OUT));
}

#[test]
fn moving_keyframe_groups_and_shifting_layers() {
    let f = fixture();
    let mut p = f.project.clone();
    let key = |t: f64, x: f64| Command::SetKeyframe {
        layer: f.a,
        path: PropPath::Position,
        time: t,
        value: Value::Vec2(Vec2::new(x, 0.0)),
        easing: None,
    };
    apply(&mut p, Command::batch(vec![key(0.0, 0.0), key(1.0, 10.0), key(2.0, 20.0)])).unwrap();
    let times = |p: &Project| p.layer(f.a).unwrap().keyframe_times(PropPath::Position).unwrap();

    // Moving adjacent keys right by exactly their spacing must not merge them.
    let sel = [(f.a, PropPath::Position, 1.0), (f.a, PropPath::Position, 2.0)];
    let before = p.clone();
    let inv = apply(&mut p, Command::move_keyframes(&sel, 1.0).unwrap()).unwrap();
    assert_eq!(times(&p), vec![0.0, 2.0, 3.0]);
    assert_eq!(p.layer(f.a).unwrap().transform.position.value_at(3.0), Vec2::new(20.0, 0.0));
    apply(&mut p, inv).unwrap();
    assert_eq!(p, before);

    // Same leftwards.
    let sel = [(f.a, PropPath::Position, 0.0), (f.a, PropPath::Position, 1.0)];
    let mut q = p.clone();
    // Clamped: can't go before 0, so nothing to do.
    assert!(Command::move_keyframes(&sel, -1.0).is_none());
    apply(&mut q, Command::move_keyframes(&sel, 0.5).unwrap()).unwrap();
    assert_eq!(times(&q), vec![0.5, 1.5, 2.0]);

    // Shift the whole layer: in point and all keys move; clamped at 0.
    let mut r = p.clone();
    let shift = Command::shift_layer(&r, f.a, 0.5).unwrap().unwrap();
    apply(&mut r, shift).unwrap();
    assert_eq!(times(&r), vec![0.5, 1.5, 2.5]);
    assert_eq!(r.layer(f.a).unwrap().meta.in_point, 0.5);
    let back = Command::shift_layer(&r, f.a, -5.0).unwrap().unwrap();
    apply(&mut r, back).unwrap();
    assert_eq!(times(&r), vec![0.0, 1.0, 2.0]);
    assert!(Command::shift_layer(&r, f.a, -1.0).unwrap().is_none());
}
