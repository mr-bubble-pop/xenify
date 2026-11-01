//! Lenient parsing of values from model-written JSON: colours as `"#hex"`,
//! points as `[x, y]`, paths as SVG path data, easings by name.

use kurbo::{BezPath, Vec2};
use serde_json::Value as Json;
use xenify_doc::{Color, Easing, Value, ValueKind};

/// A number, also accepting numeric strings like `"0.5"` (models often quote numbers).
pub fn number(v: &Json) -> Option<f64> {
    match v {
        Json::Number(n) => n.as_f64(),
        Json::String(s) => s.trim().trim_end_matches("px").trim().parse().ok(),
        _ => None,
    }
}

pub fn color(v: &Json) -> Result<Color, String> {
    match v {
        Json::String(s) => Color::from_hex(s).ok_or_else(|| format!("bad colour {s:?} (use \"#rrggbb\")")),
        Json::Array(a) if a.len() >= 3 => {
            let n = |i: usize| a.get(i).and_then(number);
            let (r, g, b) = (n(0).unwrap_or(0.0), n(1).unwrap_or(0.0), n(2).unwrap_or(0.0));
            let a_ = n(3).unwrap_or(1.0);
            // Accept 0..255 as well as 0..1.
            let k = if r > 1.0 || g > 1.0 || b > 1.0 { 255.0 } else { 1.0 };
            Ok(Color::rgba(r / k, g / k, b / k, a_))
        }
        Json::Object(_) => serde_json::from_value(v.clone()).map_err(|e| e.to_string()),
        _ => Err(format!("bad colour {v}")),
    }
}

pub fn vec2(v: &Json) -> Result<Vec2, String> {
    match v {
        Json::Array(a) if a.len() == 2 => {
            let x = number(&a[0]).ok_or("x must be a number")?;
            let y = number(&a[1]).ok_or("y must be a number")?;
            Ok(Vec2::new(x, y))
        }
        Json::Object(o) => {
            let x = o.get("x").and_then(number).ok_or("missing x")?;
            let y = o.get("y").and_then(number).ok_or("missing y")?;
            Ok(Vec2::new(x, y))
        }
        // "[100, 200]" as a string.
        Json::String(s) if s.trim_start().starts_with('[') => {
            vec2(&serde_json::from_str(s).map_err(|_| format!("expected [x, y], got {s:?}"))?)
        }
        // A single number means the same on both axes (handy for scale).
        other if number(other).is_some() => {
            let f = number(other).unwrap();
            Ok(Vec2::new(f, f))
        }
        _ => Err(format!("expected [x, y], got {v}")),
    }
}

pub fn path(v: &Json) -> Result<BezPath, String> {
    let s = v.as_str().ok_or("path must be an SVG path string like \"M0 0 L10 0 Z\"")?;
    BezPath::from_svg(s).map_err(|e| format!("bad SVG path: {e:?}"))
}

/// Four corner points `[[x,y],[x,y],[x,y],[x,y]]` (top-left, top-right, bottom-right, bottom-left).
pub fn quad(v: &Json) -> Result<xenify_doc::Quad, String> {
    let pts = v.as_array().filter(|a| a.len() == 4).ok_or("expected 4 corner points [[x,y],…]")?;
    let mut out = [xenify_doc::Vec2::ZERO; 4];
    for (o, p) in out.iter_mut().zip(pts) {
        *o = vec2(p)?;
    }
    Ok(xenify_doc::Quad(out))
}

pub fn scalar(v: &Json) -> Result<f64, String> {
    number(v).ok_or_else(|| format!("expected a number, got {v}"))
}

/// Parses `v` as a value of `kind`.
pub fn value(kind: ValueKind, v: &Json) -> Result<Value, String> {
    Ok(match kind {
        ValueKind::Scalar => Value::Scalar(scalar(v)?),
        ValueKind::Vec2 => Value::Vec2(vec2(v)?),
        ValueKind::Color => Value::Color(color(v)?),
        ValueKind::Path => Value::Path(path(v)?),
        // Needs the shape's bounds; see `tools::parse_value`.
        ValueKind::Stops => match v.as_str().and_then(css_gradient) {
            Some(g) => Value::Stops(g?.2),
            None => return Err("expected a CSS gradient string".into()),
        },
        ValueKind::Quad => Value::Quad(quad(v)?),
    })
}

/// A parsed CSS-style gradient: kind, direction (our convention: degrees,
/// 0 = left→right, 90 = top→bottom) and stops.
pub type ParsedGradient = (xenify_doc::GradientKind, f64, Vec<xenify_doc::GradientStop>);

/// Parses `linear-gradient(...)` / `radial-gradient(...)`. `None` if `s` isn't a
/// gradient at all; `Some(Err)` if it is one but malformed.
pub fn css_gradient(s: &str) -> Option<Result<ParsedGradient, String>> {
    use xenify_doc::{GradientKind, GradientStop};
    let s = s.trim();
    let (kind, inner) = if let Some(r) = s.strip_prefix("linear-gradient(") {
        (GradientKind::Linear, r)
    } else {
        (GradientKind::Radial, s.strip_prefix("radial-gradient(")?)
    };
    Some((|| {
        let inner = inner.strip_suffix(')').ok_or("gradient is missing its closing ')'")?;
        let mut parts: Vec<&str> = inner.split(',').map(str::trim).filter(|p| !p.is_empty()).collect();
        // CSS: 0deg = to top, 90deg = to right. Ours: 0 = to right, 90 = to bottom.
        let mut angle = 90.0;
        if let Some(first) = parts.first() {
            let dir = match *first {
                "to right" => Some(90.0),
                "to left" => Some(270.0),
                "to bottom" => Some(180.0),
                "to top" => Some(0.0),
                "to bottom right" | "to right bottom" => Some(135.0),
                "to top right" | "to right top" => Some(45.0),
                "to bottom left" | "to left bottom" => Some(225.0),
                "to top left" | "to left top" => Some(315.0),
                d if d.ends_with("deg") => d.trim_end_matches("deg").trim().parse::<f64>().ok(),
                _ => None,
            };
            if let Some(d) = dir {
                angle = d;
                parts.remove(0);
            } else if kind == GradientKind::Radial && !first.starts_with('#') && !first.starts_with("rgb") {
                parts.remove(0); // e.g. "circle" / "circle at center"
            }
        }
        if parts.len() < 2 {
            return Err("a gradient needs at least two colours".to_string());
        }
        let n = parts.len();
        let stops = parts
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let mut it = p.split_whitespace();
                let c = it.next().unwrap_or("");
                let color = color(&Json::String(c.to_string()))?;
                let offset = it
                    .next()
                    .and_then(|o| o.trim_end_matches('%').parse::<f64>().ok())
                    .map_or(i as f64 / (n - 1) as f64, |pct| pct / 100.0);
                Ok(GradientStop { offset: offset.clamp(0.0, 1.0), color })
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok((kind, angle - 90.0, stops))
    })())
}

/// Gradient colour stops as CSS (direction left out).
pub fn stops_css(stops: &[xenify_doc::GradientStop]) -> String {
    let g = xenify_doc::Gradient {
        kind: xenify_doc::GradientKind::Linear,
        start: Default::default(),
        end: Default::default(),
        stops: stops.to_vec(),
    };
    let stops: Vec<String> =
        g.sorted_stops().iter().map(|s| format!("{} {}%", s.color.to_hex(), (s.offset * 100.0).round())).collect();
    format!("linear-gradient({})", stops.join(", "))
}

/// `"ease_in_out"`, `"linear"`, `"step_end"`, … a CSS easing (`"steps(4, jump-end)"`,
/// `"cubic-bezier(…)"`) or `[x1, y1, x2, y2]`.
pub fn easing(v: &Json) -> Result<Easing, String> {
    match v {
        Json::String(s) => Easing::preset(&s.to_lowercase().replace(['-', ' '], "_"))
            .or_else(|| Easing::from_css(s))
            .ok_or_else(|| format!("unknown easing {s:?}; use one of {} or steps(n, jump-end)", easing_names())),
        Json::Array(a) if a.len() == 4 => {
            let n: Vec<f64> = a.iter().filter_map(number).collect();
            if n.len() != 4 {
                return Err("bezier easing needs 4 numbers".into());
            }
            Ok(Easing::Bezier { x1: n[0], y1: n[1], x2: n[2], y2: n[3] })
        }
        Json::Null => Ok(Easing::Linear),
        _ => Err(format!("bad easing {v}")),
    }
}

pub fn easing_names() -> String {
    Easing::PRESETS.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", ")
}

/// A value rendered compactly for the model.
pub fn to_json(v: &Value) -> Json {
    let r = |f: f64| (f * 1000.0).round() / 1000.0;
    match v {
        Value::Scalar(s) => Json::from(r(*s)),
        Value::Vec2(p) => serde_json::json!([r(p.x), r(p.y)]),
        Value::Color(c) => Json::from(c.to_hex()),
        Value::Path(p) => Json::from(p.to_svg()),
        Value::Stops(s) => Json::from(stops_css(s)),
        Value::Quad(q) => Json::from(q.0.iter().map(|p| serde_json::json!([r(p.x), r(p.y)])).collect::<Vec<_>>()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn lenient_parsing() {
        assert_eq!(color(&json!("#ff0000")).unwrap(), Color::rgb(1.0, 0.0, 0.0));
        assert_eq!(color(&json!([255, 0, 0])).unwrap(), Color::rgb(1.0, 0.0, 0.0));
        assert_eq!(color(&json!([1, 0, 0, 0.5])).unwrap().a, 0.5);
        assert_eq!(vec2(&json!([3, 4])).unwrap(), Vec2::new(3.0, 4.0));
        assert_eq!(vec2(&json!({"x": 1, "y": 2})).unwrap(), Vec2::new(1.0, 2.0));
        assert_eq!(vec2(&json!(1.5)).unwrap(), Vec2::new(1.5, 1.5));
        assert!(path(&json!("M0 0 L10 0 L10 10 Z")).is_ok());
        assert!(path(&json!("nonsense")).is_err());
        assert_eq!(easing(&json!("ease-in-out")).unwrap(), Easing::EASE_IN_OUT);
        assert!(matches!(easing(&json!([0.1, 0.2, 0.3, 0.4])).unwrap(), Easing::Bezier { .. }));
        assert!(easing(&json!("wobbly")).is_err());
        // Models often quote numbers.
        assert_eq!(scalar(&json!("0")).unwrap(), 0.0);
        assert_eq!(scalar(&json!(" 12.5px")).unwrap(), 12.5);
        assert_eq!(vec2(&json!(["1", "2"])).unwrap(), Vec2::new(1.0, 2.0));
        assert_eq!(vec2(&json!("[3, 4]")).unwrap(), Vec2::new(3.0, 4.0));
        assert_eq!(vec2(&json!("2")).unwrap(), Vec2::new(2.0, 2.0));
        // CSS gradients.
        let (k, a, stops) = css_gradient("linear-gradient(135deg, #ff0000, #0000ff 80%)").unwrap().unwrap();
        assert_eq!((k, a), (xenify_doc::GradientKind::Linear, 45.0));
        assert_eq!(stops[1].offset, 0.8);
        let (_, a, _) = css_gradient("linear-gradient(to right, #000, #fff)").unwrap().unwrap();
        assert_eq!(a, 0.0);
        let (k, _, s) = css_gradient("radial-gradient(circle, #fff, #000)").unwrap().unwrap();
        assert_eq!((k, s.len()), (xenify_doc::GradientKind::Radial, 2));
        assert!(css_gradient("#ff0000").is_none());
        assert!(css_gradient("linear-gradient(90deg, #fff)").unwrap().is_err());
    }
}

/// Text as the model wrote it, with HTML entities it sometimes adds
/// (`&amp;`, `&lt;`, `&#39;` …) turned back into characters.
pub fn plain_text(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let end = rest[..rest.len().min(10)].find(';');
        let decoded = end.and_then(|e| {
            let name = &rest[1..e];
            let c = match name {
                "amp" => '&',
                "lt" => '<',
                "gt" => '>',
                "quot" => '"',
                "apos" => '\'',
                "nbsp" => '\u{a0}',
                n if n.starts_with("#x") || n.starts_with("#X") => {
                    char::from_u32(u32::from_str_radix(&n[2..], 16).ok()?)?
                }
                n if n.starts_with('#') => char::from_u32(n[1..].parse().ok()?)?,
                _ => return None,
            };
            Some((c, e + 1))
        });
        match decoded {
            Some((c, len)) => {
                out.push(c);
                rest = &rest[len..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod entity_tests {
    #[test]
    fn html_entities_become_characters() {
        use super::plain_text;
        assert_eq!(plain_text("Beds &amp; paths"), "Beds & paths");
        assert_eq!(plain_text("it&#39;s &lt;3 &#x263A;"), "it's <3 ☺");
        assert_eq!(plain_text("R&D & more &unknown;"), "R&D & more &unknown;");
    }
}
