//! Every export format, and working it out from a file name.

use xenify_platform::VideoFormat;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportTarget {
    Video(VideoFormat),
    /// The current frame as a PNG image.
    PngFrame,
    /// The current frame as an SVG image.
    SvgFrame,
    /// The whole composition as an SVG that plays in browsers.
    SvgAnimated,
    LottieJson,
    /// A web page playing the Lottie animation.
    LottieHtml,
    /// A PDF with one page per slide (or this frame).
    Pdf,
}

impl ExportTarget {
    pub const ALL: [ExportTarget; 10] = [
        ExportTarget::Video(VideoFormat::Mp4),
        ExportTarget::Video(VideoFormat::WebM),
        ExportTarget::Video(VideoFormat::Gif),
        ExportTarget::Video(VideoFormat::PngSequence),
        ExportTarget::PngFrame,
        ExportTarget::SvgAnimated,
        ExportTarget::SvgFrame,
        ExportTarget::LottieJson,
        ExportTarget::LottieHtml,
        ExportTarget::Pdf,
    ];

    /// The format a file name asks for (`None` if the extension is unknown).
    /// `.png` means one frame and `.svg` the animation; a name without an
    /// extension is a folder of PNG frames.
    pub fn from_path(path: &Path) -> Option<Self> {
        let Some(ext) = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase) else {
            return Some(ExportTarget::Video(VideoFormat::PngSequence));
        };
        Some(match ext.as_str() {
            "mp4" | "m4v" => ExportTarget::Video(VideoFormat::Mp4),
            "webm" => ExportTarget::Video(VideoFormat::WebM),
            "gif" => ExportTarget::Video(VideoFormat::Gif),
            "png" => ExportTarget::PngFrame,
            "svg" => ExportTarget::SvgAnimated,
            "json" | "lottie" => ExportTarget::LottieJson,
            "html" | "htm" => ExportTarget::LottieHtml,
            "pdf" => ExportTarget::Pdf,
            _ => return None,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            ExportTarget::Video(VideoFormat::Mp4) => "MP4 video (H.264)",
            ExportTarget::Video(VideoFormat::WebM) => "WebM video (VP9, transparency)",
            ExportTarget::Video(VideoFormat::Gif) => "GIF animation",
            ExportTarget::Video(VideoFormat::PngSequence) => "PNG sequence (folder of frames)",
            ExportTarget::PngFrame => "PNG image (this frame)",
            ExportTarget::SvgAnimated => "Animated SVG (plays in browsers)",
            ExportTarget::SvgFrame => "SVG image (this frame)",
            ExportTarget::LottieJson => "Lottie animation (.json)",
            ExportTarget::LottieHtml => "Web page (Lottie player, .html)",
            ExportTarget::Pdf => "PDF (a page per slide)",
        }
    }

    /// The file extension written (`""` for a folder).
    pub fn extension(self) -> &'static str {
        match self {
            ExportTarget::Video(VideoFormat::PngSequence) => "",
            ExportTarget::Video(f) => f.extension(),
            ExportTarget::PngFrame => "png",
            ExportTarget::SvgAnimated | ExportTarget::SvgFrame => "svg",
            ExportTarget::LottieJson => "json",
            ExportTarget::LottieHtml => "html",
            ExportTarget::Pdf => "pdf",
        }
    }

    /// Rendered to pixels (so size and transparency apply).
    pub fn is_raster(self) -> bool {
        matches!(self, ExportTarget::Video(_) | ExportTarget::PngFrame | ExportTarget::Pdf)
    }

    pub fn supports_alpha(self) -> bool {
        match self {
            ExportTarget::Video(f) => f.supports_alpha(),
            ExportTarget::Pdf => false,
            _ => true,
        }
    }

    /// Every extension any target writes, for file pickers.
    pub fn all_extensions() -> Vec<&'static str> {
        let mut v: Vec<&'static str> = Self::ALL.iter().map(|t| t.extension()).filter(|e| !e.is_empty()).collect();
        v.sort_unstable();
        v.dedup();
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_from_file_names() {
        let t = |p: &str| ExportTarget::from_path(Path::new(p));
        assert_eq!(t("clip.MP4"), Some(ExportTarget::Video(VideoFormat::Mp4)));
        assert_eq!(t("a/b.webm"), Some(ExportTarget::Video(VideoFormat::WebM)));
        assert_eq!(t("loop.gif"), Some(ExportTarget::Video(VideoFormat::Gif)));
        assert_eq!(t("frames"), Some(ExportTarget::Video(VideoFormat::PngSequence)));
        assert_eq!(t("still.png"), Some(ExportTarget::PngFrame));
        assert_eq!(t("anim.svg"), Some(ExportTarget::SvgAnimated));
        assert_eq!(t("anim.json"), Some(ExportTarget::LottieJson));
        assert_eq!(t("page.html"), Some(ExportTarget::LottieHtml));
        assert_eq!(t("deck.PDF"), Some(ExportTarget::Pdf));
        assert_eq!(t("doc.xyz"), None);
        for target in ExportTarget::ALL {
            let name = format!("x.{}", target.extension());
            let back = ExportTarget::from_path(Path::new(if target.extension().is_empty() { "x" } else { &name }));
            // .svg and .png have two meanings each; the others map back exactly.
            if !matches!(target, ExportTarget::SvgFrame) {
                assert_eq!(back, Some(target), "{target:?}");
            }
        }
        assert!(ExportTarget::all_extensions().contains(&"html"));
    }
}
