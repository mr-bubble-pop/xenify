//! Stand-in for `colored` (see `patches/README.md`): the same method names,
//! returning the text as it is.

use std::fmt;

/// Text that would have been coloured.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ColoredString(String);

impl fmt::Display for ColoredString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::ops::Deref for ColoredString {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}

macro_rules! plain {
    ($($name:ident),*) => {
        /// Colour and style methods; all of them leave the text unchanged.
        pub trait Colorize {
            fn plain(self) -> ColoredString;
            $(fn $name(self) -> ColoredString where Self: Sized { self.plain() })*
        }
    };
}

plain!(
    normal, clear, bold, dimmed, italic, underline, blink, reversed, hidden, strikethrough, black, red, green,
    yellow, blue, magenta, purple, cyan, white, bright_black, bright_red, bright_green, bright_yellow, bright_blue,
    bright_magenta, bright_purple, bright_cyan, bright_white, on_black, on_red, on_green, on_yellow, on_blue,
    on_magenta, on_purple, on_cyan, on_white
);

impl Colorize for &str {
    fn plain(self) -> ColoredString {
        ColoredString(self.to_string())
    }
}

impl Colorize for String {
    fn plain(self) -> ColoredString {
        ColoredString(self)
    }
}

impl Colorize for ColoredString {
    fn plain(self) -> ColoredString {
        self
    }
}
