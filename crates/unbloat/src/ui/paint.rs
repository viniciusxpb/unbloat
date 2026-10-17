//! Direct2D/DirectWrite drawing in device-independent pixels, with the dark palette.
use std::{cell::RefCell, collections::HashMap};
use windows::{
    Win32::Graphics::{
        Direct2D::{Common::*, *},
        DirectWrite::*,
    },
    core::{BOOL, HSTRING, Interface, Result, w},
};
use windows_numerics::Vector2;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Color(pub u32);

impl Color {
    fn d2d(self) -> D2D1_COLOR_F {
        let channel = |shift: u32| ((self.0 >> shift) & 0xff) as f32 / 255.0;
        D2D1_COLOR_F {
            r: channel(16),
            g: channel(8),
            b: channel(0),
            a: 1.0,
        }
    }
    /// The COLORREF (0x00BBGGRR) that DWM expects.
    pub fn colorref(self) -> u32 {
        (self.0 & 0xff) << 16 | (self.0 & 0xff00) | (self.0 >> 16 & 0xff)
    }
}

pub mod palette {
    use super::Color;
    pub const WINDOW: Color = Color(0x15181c);
    pub const CARD: Color = Color(0x1a1e23);
    pub const BORDER: Color = Color(0x2a3038);
    pub const FOOTER: Color = Color(0x111316);
    pub const TEXT: Color = Color(0xe8eaed);
    pub const MUTED: Color = Color(0x9aa3ad);
    pub const FAINT: Color = Color(0x6b7480);
    pub const TRACK: Color = Color(0x2a3038);
    pub const GREEN: Color = Color(0x3fb68b);
    pub const GREEN_HOVER: Color = Color(0x4cc79a);
    pub const GREEN_PRESSED: Color = Color(0x35a07a);
    pub const GREEN_TINT: Color = Color(0x1d3a30);
    pub const ON_GREEN: Color = Color(0x0b1712);
    pub const RED: Color = Color(0xf08c8c);
    pub const RED_TINT: Color = Color(0x2e1719);
    pub const RED_TINT_HOVER: Color = Color(0x3b1c1f);
    pub const RED_BORDER: Color = Color(0x6e2a2e);
    pub const AMBER: Color = Color(0xe0b252);
    pub const BUTTON: Color = Color(0x23282e);
    pub const BUTTON_HOVER: Color = Color(0x2c3239);
    pub const BUTTON_PRESSED: Color = Color(0x1e2227);
    pub const BUTTON_BORDER: Color = Color(0x353c45);
    pub const FOCUS: Color = Color(0x8ab4f8);
}

/// Segoe Fluent Icons (Windows 11) and Segoe MDL2 Assets (Windows 10) share code points.
pub mod glyph {
    pub const FOLDER: char = '\u{E8B7}';
    pub const FOLDER_OPEN: char = '\u{E838}';
    pub const DATA: char = '\u{F156}';
    pub const STOPWATCH: char = '\u{E916}';
    pub const CLOCK: char = '\u{E823}';
    pub const SPEED: char = '\u{EC4A}';
    pub const SHRINK: char = '\u{E73F}';
    pub const FILES: char = '\u{E8C8}';
    pub const PACKAGE: char = '\u{E7B8}';
    pub const FLAG: char = '\u{E7C1}';
    pub const PAGE: char = '\u{E7C3}';
    pub const CLOSE: char = '\u{E711}';
    pub const CHECK: char = '\u{E73E}';
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Family {
    Text,
    Display,
    Mono,
    Icons,
}

#[derive(Clone, Copy, Debug)]
pub struct Style {
    pub family: Family,
    pub size: f32,
    pub weight: i32,
    pub wrap: bool,
    /// Extra space after each character, for small capitals labels.
    pub tracking: f32,
}

impl Style {
    pub const fn new(family: Family, size: f32, weight: i32) -> Self {
        Self {
            family,
            size,
            weight,
            wrap: false,
            tracking: 0.0,
        }
    }
    pub const fn wrapped(self) -> Self {
        Self { wrap: true, ..self }
    }
    pub const fn tracked(self, tracking: f32) -> Self {
        Self { tracking, ..self }
    }
}

pub const REGULAR: i32 = 400;
pub const SEMIBOLD: i32 = 600;
pub const BOLD: i32 = 700;

#[derive(Clone, Copy, Debug, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }
    pub fn right(&self) -> f32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
    pub fn inset(&self, by: f32) -> Self {
        Self::new(
            self.x + by,
            self.y + by,
            self.w - 2.0 * by,
            self.h - 2.0 * by,
        )
    }
    fn d2d(&self) -> D2D_RECT_F {
        D2D_RECT_F {
            left: self.x,
            top: self.y,
            right: self.right(),
            bottom: self.bottom(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Align {
    Start,
    Center,
    End,
}

#[derive(Clone, Copy, Debug, Default)]
struct Line {
    width: f32,
    baseline: f32,
    height: f32,
}

/// Factories and installed font families, shared by every render target.
pub struct Graphics {
    pub d2d: ID2D1Factory,
    dwrite: IDWriteFactory,
    families: HashMap<Family, HSTRING>,
    formats: RefCell<HashMap<(Family, u32, i32), IDWriteTextFormat>>,
}

impl Graphics {
    pub fn new() -> Result<Self> {
        unsafe {
            let d2d = D2D1CreateFactory::<ID2D1Factory>(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let dwrite = DWriteCreateFactory::<IDWriteFactory>(DWRITE_FACTORY_TYPE_SHARED)?;
            let mut collection = None;
            dwrite.GetSystemFontCollection(&mut collection, false)?;
            let installed = |name: &str| {
                let (mut index, mut exists) = (0, BOOL(0));
                collection.as_ref().is_some_and(|c| {
                    c.FindFamilyName(&HSTRING::from(name), &mut index, &mut exists)
                        .is_ok()
                        && exists.as_bool()
                })
            };
            // Prefer the Windows 11 faces, falling back to what Windows 10 ships.
            let first = |names: &[&str]| {
                HSTRING::from(
                    *names
                        .iter()
                        .find(|n| installed(n))
                        .unwrap_or(names.last().unwrap()),
                )
            };
            let families = HashMap::from([
                (Family::Text, first(&["Segoe UI Variable Text", "Segoe UI"])),
                (
                    Family::Display,
                    first(&["Segoe UI Variable Display", "Segoe UI"]),
                ),
                (Family::Mono, first(&["Cascadia Mono", "Consolas"])),
                (
                    Family::Icons,
                    first(&["Segoe Fluent Icons", "Segoe MDL2 Assets"]),
                ),
            ]);
            Ok(Self {
                d2d,
                dwrite,
                families,
                formats: RefCell::default(),
            })
        }
    }

    fn format(&self, style: &Style) -> Result<IDWriteTextFormat> {
        let key = (style.family, style.size.to_bits(), style.weight);
        if let Some(format) = self.formats.borrow().get(&key) {
            return Ok(format.clone());
        }
        let format = unsafe {
            self.dwrite.CreateTextFormat(
                &self.families[&style.family],
                None,
                DWRITE_FONT_WEIGHT(style.weight),
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                style.size,
                w!(""),
            )?
        };
        self.formats.borrow_mut().insert(key, format.clone());
        Ok(format)
    }

    fn layout(
        &self,
        text: &str,
        style: &Style,
        width: f32,
        height: f32,
    ) -> Result<IDWriteTextLayout> {
        let wide: Vec<u16> = text.encode_utf16().collect();
        let format = self.format(style)?;
        unsafe {
            let layout =
                self.dwrite
                    .CreateTextLayout(&wide, &format, width.max(0.0), height.max(0.0))?;
            let range = DWRITE_TEXT_RANGE {
                startPosition: 0,
                length: wide.len() as u32,
            };
            if style.wrap {
                layout.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP)?;
            } else {
                layout.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            }
            let trimming = DWRITE_TRIMMING {
                granularity: if style.wrap {
                    DWRITE_TRIMMING_GRANULARITY_WORD
                } else {
                    DWRITE_TRIMMING_GRANULARITY_CHARACTER
                },
                delimiter: 0,
                delimiterCount: 0,
            };
            let ellipsis = self.dwrite.CreateEllipsisTrimmingSign(&format)?;
            layout.SetTrimming(&trimming, &ellipsis)?;
            if style.tracking != 0.0
                && let Ok(spaced) = layout.cast::<IDWriteTextLayout1>()
            {
                spaced.SetCharacterSpacing(0.0, style.tracking, 0.0, range)?;
            }
            Ok(layout)
        }
    }

    /// Width of a single line of text.
    pub fn measure(&self, text: &str, style: &Style) -> f32 {
        self.metrics(text, style).map_or(0.0, |line| line.width)
    }

    /// Measures a single unconstrained line.
    fn metrics(&self, text: &str, style: &Style) -> Result<Line> {
        let layout = self.layout(
            text,
            &Style {
                wrap: false,
                ..*style
            },
            10_000.0,
            1_000.0,
        )?;
        let mut metrics = DWRITE_TEXT_METRICS::default();
        let mut line = [DWRITE_LINE_METRICS::default()];
        let mut lines = 0;
        unsafe {
            layout.GetMetrics(&mut metrics)?;
            let _ = layout.GetLineMetrics(Some(&mut line), &mut lines);
        }
        Ok(Line {
            width: metrics.widthIncludingTrailingWhitespace,
            baseline: line[0].baseline,
            height: line[0].height,
        })
    }
}

/// One frame of drawing on a render target, in DIPs.
pub struct Canvas<'a> {
    pub target: &'a ID2D1RenderTarget,
    pub graphics: &'a Graphics,
}

impl Canvas<'_> {
    fn brush(&self, color: Color) -> Option<ID2D1SolidColorBrush> {
        unsafe { self.target.CreateSolidColorBrush(&color.d2d(), None).ok() }
    }

    pub fn clear(&self, color: Color) {
        unsafe { self.target.Clear(Some(&color.d2d())) }
    }

    pub fn fill(&self, rect: Rect, radius: f32, color: Color) {
        let Some(brush) = self.brush(color) else {
            return;
        };
        unsafe {
            if radius > 0.0 {
                let rounded = D2D1_ROUNDED_RECT {
                    rect: rect.d2d(),
                    radiusX: radius,
                    radiusY: radius,
                };
                self.target.FillRoundedRectangle(&rounded, &brush);
            } else {
                self.target.FillRectangle(&rect.d2d(), &brush);
            }
        }
    }

    /// A one-DIP outline drawn inside `rect`.
    pub fn stroke(&self, rect: Rect, radius: f32, color: Color, width: f32) {
        let Some(brush) = self.brush(color) else {
            return;
        };
        let rounded = D2D1_ROUNDED_RECT {
            rect: rect.inset(width / 2.0).d2d(),
            radiusX: radius,
            radiusY: radius,
        };
        unsafe {
            self.target
                .DrawRoundedRectangle(&rounded, &brush, width, None)
        }
    }

    pub fn circle(&self, x: f32, y: f32, radius: f32, color: Color) {
        let Some(brush) = self.brush(color) else {
            return;
        };
        let ellipse = D2D1_ELLIPSE {
            point: Vector2 { X: x, Y: y },
            radiusX: radius,
            radiusY: radius,
        };
        unsafe { self.target.FillEllipse(&ellipse, &brush) }
    }

    pub fn line(&self, from: (f32, f32), to: (f32, f32), color: Color) {
        let Some(brush) = self.brush(color) else {
            return;
        };
        unsafe {
            self.target.DrawLine(
                Vector2 {
                    X: from.0,
                    Y: from.1,
                },
                Vector2 { X: to.0, Y: to.1 },
                &brush,
                1.0,
                None,
            )
        }
    }

    /// Text in `rect`, vertically centered, trimmed with an ellipsis when too long.
    pub fn text(&self, text: &str, style: Style, rect: Rect, color: Color, align: Align) {
        self.draw(text, style, rect, color, align, !style.wrap);
    }

    /// Text whose layout box starts at the top of `rect`, or is centered in it.
    fn draw(
        &self,
        text: &str,
        style: Style,
        rect: Rect,
        color: Color,
        align: Align,
        centered: bool,
    ) {
        let (Some(brush), Ok(layout)) = (
            self.brush(color),
            self.graphics.layout(text, &style, rect.w, rect.h),
        ) else {
            return;
        };
        unsafe {
            let _ = layout.SetTextAlignment(match align {
                Align::Start => DWRITE_TEXT_ALIGNMENT_LEADING,
                Align::Center => DWRITE_TEXT_ALIGNMENT_CENTER,
                Align::End => DWRITE_TEXT_ALIGNMENT_TRAILING,
            });
            let _ = layout.SetParagraphAlignment(if centered {
                DWRITE_PARAGRAPH_ALIGNMENT_CENTER
            } else {
                DWRITE_PARAGRAPH_ALIGNMENT_NEAR
            });
            self.target.DrawTextLayout(
                Vector2 {
                    X: rect.x,
                    Y: rect.y,
                },
                &layout,
                &brush,
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
            );
        }
    }

    /// Several differently styled pieces on one shared baseline, like "47" + "%".
    /// Returns the width used.
    pub fn spans(&self, spans: &[(&str, Style, Color)], rect: Rect, align: Align) -> f32 {
        let measured: Vec<Line> = spans
            .iter()
            .map(|(text, style, _)| self.graphics.metrics(text, style).unwrap_or_default())
            .collect();
        let width: f32 = measured.iter().map(|line| line.width).sum();
        let ascent = measured
            .iter()
            .map(|line| line.baseline)
            .fold(0.0, f32::max);
        let descent = measured
            .iter()
            .map(|line| line.height - line.baseline)
            .fold(0.0, f32::max);
        // The combined line box is centered; every piece sits on its baseline.
        let top = rect.y + (rect.h - ascent - descent) / 2.0;
        let mut x = match align {
            Align::Start => rect.x,
            Align::Center => rect.x + (rect.w - width) / 2.0,
            Align::End => rect.right() - width,
        };
        for ((text, style, color), line) in spans.iter().zip(&measured) {
            let y = top + ascent - line.baseline;
            let box_ = Rect::new(x, y, line.width + 1.0, line.height);
            self.draw(text, *style, box_, *color, Align::Start, false);
            x += line.width;
        }
        width
    }

    /// An icon glyph centered on (x, y).
    pub fn icon(&self, glyph: char, size: f32, center: (f32, f32), color: Color) {
        let style = Style::new(Family::Icons, size, REGULAR);
        let side = size * 2.0;
        self.text(
            glyph.encode_utf8(&mut [0; 4]),
            style,
            Rect::new(center.0 - side / 2.0, center.1 - side / 2.0, side, side),
            color,
            Align::Center,
        );
    }

    pub fn measure(&self, text: &str, style: &Style) -> f32 {
        self.graphics.measure(text, style)
    }
}
