//! Pictures people put on a sheet (Insert › Picture): floating over the
//! cells like charts, moved and resized with the mouse, and moving with
//! their cell as rows and columns are added or taken away. A .xcalc keeps
//! each picture's bytes as a file of its own; an Excel file gets a real
//! Excel picture (and Calc's extras, so it opens again as one of Calc's).
//! Pictures that came in an Excel file are kept as they were (drawings.rs).

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Picture {
    pub id: u32,
    /// The sheet it floats on, and its top left (a cell, plus pixels into it).
    pub sheet: u32,
    pub row: i32,
    pub col: i32,
    pub dx: f64,
    pub dy: f64,
    pub w: f64,
    pub h: f64,
    /// image/png, image/jpeg, image/gif, image/webp or image/svg+xml.
    pub mime: String,
    /// The picture's bytes, base64 (left out where a file holds them apart).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub data: String,
    /// The file it came from ("logo.png"), and what it shows, for screen readers.
    pub name: String,
    pub alt: String,
    /// In a saved file: the part holding its bytes.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub part: String,
}

impl Picture {
    /// After `count` rows (or columns) were inserted (or, when negative,
    /// deleted) at `at` on `sheet`: the picture moves with its cell.
    pub fn shift(&mut self, sheet: u32, at: i32, count: i32, rows: bool) {
        if self.sheet != sheet {
            return;
        }
        let mv = |v: i32| if v >= at { (v + count).max(at) } else { v };
        if rows {
            self.row = mv(self.row).max(1);
        } else {
            self.col = mv(self.col).max(1);
        }
    }

    /// The usual ending for its kind of file.
    pub fn extension(&self) -> &'static str {
        match self.mime.as_str() {
            "image/jpeg" => "jpeg",
            "image/gif" => "gif",
            "image/webp" => "webp",
            "image/svg+xml" => "svg",
            _ => "png",
        }
    }

    /// The kind of file from its ending.
    pub fn mime_of(name: &str) -> &'static str {
        match name.rsplit('.').next().unwrap_or("").to_ascii_lowercase().as_str() {
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "webp" => "image/webp",
            "svg" => "image/svg+xml",
            _ => "image/png",
        }
    }
}
