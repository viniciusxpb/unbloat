//! Layout and drawing of the three screens. Everything is in DIPs and every
//! string arrives already formatted; this module only places it.
use super::paint::{
    palette::{self, *},
    *,
};

pub const WIDTH: f32 = 640.0;
const MARGIN: f32 = 28.0;
const CONTENT: f32 = WIDTH - 2.0 * MARGIN;
const FOOTER_HEIGHT: f32 = 76.0;
pub const BUTTON_HEIGHT: f32 = 36.0;
const CARD_HEIGHT: f32 = 66.0;

pub const WORKING_HEIGHT: f32 = 440.0;
pub const DONE_HEIGHT: f32 = 340.0;
pub const FAILED_HEIGHT: f32 = 300.0;

const STATUS: Style = Style::new(Family::Text, 11.0, SEMIBOLD).tracked(1.2);
const TITLE: Style = Style::new(Family::Display, 22.0, BOLD);
const HEADING: Style = Style::new(Family::Display, 18.0, BOLD);
const PATH: Style = Style::new(Family::Mono, 12.5, REGULAR);
const PERCENT: Style = Style::new(Family::Display, 44.0, BOLD);
const PERCENT_SIGN: Style = Style::new(Family::Display, 20.0, SEMIBOLD);
const BODY: Style = Style::new(Family::Text, 13.0, REGULAR);
const BODY_STRONG: Style = Style::new(Family::Text, 13.0, SEMIBOLD);
const LABEL: Style = Style::new(Family::Text, 12.0, REGULAR);
const VALUE: Style = Style::new(Family::Mono, 15.5, REGULAR);
const BUTTON_TEXT: Style = Style::new(Family::Text, 13.0, SEMIBOLD);

pub struct Card {
    pub glyph: char,
    pub label: &'static str,
    pub value: String,
}

pub struct Working {
    pub status: &'static str,
    pub status_color: Color,
    pub name: String,
    /// (folder, suffix): the folder is shortened in the middle when space runs out.
    pub path: (String, String),
    /// The integer percentage, once known.
    pub percent: Option<String>,
    /// `None` draws an indeterminate bar at `pulse` (0..1).
    pub fraction: Option<f32>,
    pub pulse: f32,
    /// "1,41 GB de 3,01 GB", split for emphasis on the first value.
    pub amount: Option<[String; 3]>,
    pub remaining: Option<[String; 3]>,
    pub cards: [Card; 6],
    pub current: String,
}

pub struct Done {
    pub title: &'static str,
    /// (folder, suffix): the folder is shortened in the middle when space runs out.
    pub path: (String, String),
    pub original_label: &'static str,
    pub original: String,
    pub archive: String,
    pub fraction: f32,
    pub saved: [String; 3],
    pub cards: [Card; 3],
}

pub struct Failed {
    pub title: &'static str,
    pub message: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ButtonKind {
    Primary,
    Neutral,
    Danger,
}

#[derive(Clone, Copy, Default, Debug)]
pub struct ButtonState {
    pub hot: bool,
    pub pressed: bool,
    pub focused: bool,
    pub disabled: bool,
}

/// Button rectangles for a screen of the given height: (x, width) from the right.
pub fn button_row(height: f32, widths: &[f32]) -> Vec<Rect> {
    let y = height - FOOTER_HEIGHT + (FOOTER_HEIGHT - BUTTON_HEIGHT) / 2.0;
    let mut right = WIDTH - MARGIN;
    let mut rects: Vec<Rect> = widths
        .iter()
        .rev()
        .map(|width| {
            let rect = Rect::new(right - width, y, *width, BUTTON_HEIGHT);
            right -= width + 10.0;
            rect
        })
        .collect();
    rects.reverse();
    rects
}

fn footer(canvas: &Canvas, height: f32) {
    let top = height - FOOTER_HEIGHT;
    canvas.fill(
        Rect::new(0.0, top, WIDTH, FOOTER_HEIGHT),
        0.0,
        palette::FOOTER,
    );
    canvas.line((0.0, top + 0.5), (WIDTH, top + 0.5), BORDER);
}

fn bar(canvas: &Canvas, rect: Rect, fraction: Option<f32>, pulse: f32) {
    let radius = rect.h / 2.0;
    canvas.fill(rect, radius, TRACK);
    match fraction {
        Some(fraction) if fraction > 0.0 => {
            let width = (rect.w * fraction.clamp(0.0, 1.0)).max(rect.h);
            canvas.fill(Rect { w: width, ..rect }, radius, GREEN);
        }
        Some(_) => {}
        None => {
            // A segment sweeping across the track while the size is unknown.
            let segment = rect.w * 0.28;
            let x = rect.x - segment + (rect.w + segment) * pulse;
            let start = x.max(rect.x);
            let end = (x + segment).min(rect.right());
            if end > start {
                canvas.fill(Rect::new(start, rect.y, end - start, rect.h), radius, GREEN);
            }
        }
    }
}

fn cards(canvas: &Canvas, top: f32, columns: usize, cards: &[Card]) {
    let rows = cards.len().div_ceil(columns);
    let frame = Rect::new(MARGIN, top, CONTENT, CARD_HEIGHT * rows as f32);
    canvas.fill(frame, 8.0, CARD);
    let width = CONTENT / columns as f32;
    for row in 1..rows {
        let y = (top + CARD_HEIGHT * row as f32).round() + 0.5;
        canvas.line((MARGIN, y), (MARGIN + CONTENT, y), BORDER);
    }
    for column in 1..columns {
        let x = (MARGIN + width * column as f32).round() + 0.5;
        canvas.line((x, top), (x, frame.bottom()), BORDER);
    }
    canvas.stroke(frame, 8.0, BORDER, 1.0);
    for (index, card) in cards.iter().enumerate() {
        let x = MARGIN + width * (index % columns) as f32;
        let y = top + CARD_HEIGHT * (index / columns) as f32;
        canvas.icon(card.glyph, 16.0, (x + 26.0, y + CARD_HEIGHT / 2.0), MUTED);
        let text_x = x + 50.0;
        let text_w = width - 58.0;
        canvas.text(
            card.label,
            LABEL,
            Rect::new(text_x, y + 12.0, text_w, 18.0),
            MUTED,
            Align::Start,
        );
        canvas.text(
            &card.value,
            VALUE,
            Rect::new(text_x, y + 31.0, text_w, 22.0),
            TEXT,
            Align::Start,
        );
    }
}

/// "prefix **value** suffix" with the value emphasized.
fn emphasized(
    canvas: &Canvas,
    parts: &[String; 3],
    rect: Rect,
    align: Align,
    accent: Color,
) -> f32 {
    canvas.spans(
        &[
            (&parts[0], BODY, MUTED),
            (&parts[1], BODY_STRONG, accent),
            (&parts[2], BODY, MUTED),
        ],
        rect,
        align,
    )
}

pub fn working(canvas: &Canvas, view: &Working) {
    canvas.clear(WINDOW);
    let x = MARGIN;
    // Status line: "● COMPACTANDO"
    canvas.circle(x + 4.0, 33.0, 4.0, view.status_color);
    canvas.text(
        view.status,
        STATUS,
        Rect::new(x + 16.0, 24.0, 360.0, 18.0),
        MUTED,
        Align::Start,
    );
    // Name and paths, leaving room on the right for the percentage.
    let text_width = CONTENT - 150.0;
    canvas.icon(glyph::FOLDER, 19.0, (x + 10.0, 64.0), MUTED);
    canvas.text(
        &view.name,
        TITLE,
        Rect::new(x + 30.0, 48.0, text_width - 30.0, 32.0),
        TEXT,
        Align::Start,
    );
    canvas.text(
        &fit_path(canvas.graphics, &view.path, text_width),
        PATH,
        Rect::new(x, 82.0, text_width, 20.0),
        FAINT,
        Align::Start,
    );
    if let Some(percent) = &view.percent {
        canvas.spans(
            &[(percent, PERCENT, TEXT), ("%", PERCENT_SIGN, MUTED)],
            Rect::new(WIDTH - MARGIN - 160.0, 44.0, 160.0, 60.0),
            Align::End,
        );
    }
    bar(
        canvas,
        Rect::new(x, 116.0, CONTENT, 6.0),
        view.fraction,
        view.pulse,
    );
    // Amount on the left, estimate on the right.
    let line = Rect::new(x + 22.0, 132.0, CONTENT / 2.0, 20.0);
    if let Some(amount) = &view.amount {
        canvas.icon(glyph::DATA, 13.0, (x + 7.0, 142.0), MUTED);
        emphasized(canvas, amount, line, Align::Start, TEXT);
    }
    if let Some(remaining) = &view.remaining {
        let right = Rect::new(WIDTH / 2.0, 132.0, WIDTH / 2.0 - MARGIN, 20.0);
        let width = emphasized(canvas, remaining, right, Align::End, TEXT);
        canvas.icon(
            glyph::STOPWATCH,
            13.0,
            (right.right() - width - 12.0, 142.0),
            MUTED,
        );
    }
    cards(canvas, 168.0, 3, &view.cards);
    // The file being read right now.
    canvas.icon(glyph::PAGE, 13.0, (x + 7.0, 324.0), FAINT);
    canvas.text(
        &view.current,
        PATH,
        Rect::new(x + 22.0, 314.0, CONTENT - 22.0, 20.0),
        MUTED,
        Align::Start,
    );
    footer(canvas, WORKING_HEIGHT);
}

pub fn done(canvas: &Canvas, view: &Done) {
    canvas.clear(WINDOW);
    let x = MARGIN;
    canvas.circle(x + 22.0, 50.0, 22.0, GREEN_TINT);
    canvas.icon(glyph::CHECK, 18.0, (x + 22.0, 50.0), GREEN);
    canvas.text(
        view.title,
        HEADING,
        Rect::new(x + 58.0, 26.0, CONTENT - 58.0, 28.0),
        TEXT,
        Align::Start,
    );
    canvas.text(
        &fit_path(canvas.graphics, &view.path, CONTENT - 58.0),
        PATH,
        Rect::new(x + 58.0, 54.0, CONTENT - 58.0, 20.0),
        FAINT,
        Align::Start,
    );
    let line = Rect::new(x, 96.0, CONTENT, 20.0);
    canvas.spans(
        &[
            (view.original_label, BODY, MUTED),
            (" ", BODY, MUTED),
            (&view.original, BODY_STRONG, TEXT),
        ],
        line,
        Align::Start,
    );
    canvas.spans(
        &[
            (".7z", BODY, MUTED),
            (" ", BODY, MUTED),
            (&view.archive, BODY_STRONG, GREEN),
        ],
        line,
        Align::End,
    );
    bar(
        canvas,
        Rect::new(x, 122.0, CONTENT, 6.0),
        Some(view.fraction),
        0.0,
    );
    emphasized(
        canvas,
        &view.saved,
        Rect::new(x, 136.0, CONTENT, 20.0),
        Align::Start,
        GREEN,
    );
    cards(canvas, 176.0, 3, &view.cards);
    footer(canvas, DONE_HEIGHT);
}

pub fn failed(canvas: &Canvas, view: &Failed) {
    canvas.clear(WINDOW);
    let x = MARGIN;
    canvas.circle(x + 22.0, 50.0, 22.0, RED_TINT);
    canvas.icon(glyph::CLOSE, 16.0, (x + 22.0, 50.0), RED);
    canvas.text(
        view.title,
        HEADING,
        Rect::new(x + 58.0, 26.0, CONTENT - 58.0, 28.0),
        TEXT,
        Align::Start,
    );
    canvas.text(
        &view.message,
        BODY.wrapped(),
        Rect::new(
            x + 58.0,
            60.0,
            CONTENT - 58.0,
            FAILED_HEIGHT - FOOTER_HEIGHT - 76.0,
        ),
        MUTED,
        Align::Start,
    );
    footer(canvas, FAILED_HEIGHT);
}

pub fn button_width(graphics: &Graphics, label: &str, glyph: Option<char>) -> f32 {
    let icon = if glyph.is_some() { 22.0 } else { 0.0 };
    (graphics.measure(label, &BUTTON_TEXT) + icon + 36.0)
        .max(96.0)
        .ceil()
}

/// A button filling the whole target (`size` in DIPs), drawn over the footer color.
pub fn button(
    canvas: &Canvas,
    size: (f32, f32),
    label: &str,
    glyph: Option<char>,
    kind: ButtonKind,
    state: ButtonState,
) {
    canvas.clear(palette::FOOTER);
    let rect = Rect::new(0.0, 0.0, size.0, size.1);
    let (fill, border, text) = match (kind, state.disabled, state.pressed, state.hot) {
        (_, true, ..) => (BUTTON, BUTTON_BORDER, FAINT),
        (ButtonKind::Primary, _, true, _) => (GREEN_PRESSED, GREEN_PRESSED, ON_GREEN),
        (ButtonKind::Primary, _, _, true) => (GREEN_HOVER, GREEN_HOVER, ON_GREEN),
        (ButtonKind::Primary, ..) => (GREEN, GREEN, ON_GREEN),
        (ButtonKind::Danger, _, true, _) => (RED_TINT, RED_BORDER, RED),
        (ButtonKind::Danger, _, _, true) => (RED_TINT_HOVER, RED_BORDER, RED),
        (ButtonKind::Danger, ..) => (RED_TINT, RED_BORDER, RED),
        (ButtonKind::Neutral, _, true, _) => (BUTTON_PRESSED, BUTTON_BORDER, TEXT),
        (ButtonKind::Neutral, _, _, true) => (BUTTON_HOVER, BUTTON_BORDER, TEXT),
        (ButtonKind::Neutral, ..) => (BUTTON, BUTTON_BORDER, TEXT),
    };
    canvas.fill(rect, 7.0, fill);
    canvas.stroke(rect, 7.0, border, 1.0);
    if state.focused {
        canvas.stroke(rect, 7.0, FOCUS, 2.0);
    }
    let label_width = canvas.measure(label, &BUTTON_TEXT);
    let icon_width = if glyph.is_some() { 22.0 } else { 0.0 };
    let start = (size.0 - label_width - icon_width) / 2.0;
    if let Some(glyph) = glyph {
        canvas.icon(glyph, 12.0, (start + 7.0, size.1 / 2.0), text);
    }
    canvas.text(
        label,
        BUTTON_TEXT,
        Rect::new(start + icon_width, 0.0, label_width + 2.0, size.1),
        text,
        Align::Start,
    );
}

/// Drops folders from the middle of a path ("C:\Users\…\ui → x.7z") until it
/// fits, keeping the drive and the nearest folder; the suffix always stays.
fn fit_path(graphics: &Graphics, (folder, suffix): &(String, String), width: f32) -> String {
    let fits = |text: &str| graphics.measure(text, &PATH) <= width;
    let full = format!("{folder}{suffix}");
    if fits(&full) {
        return full;
    }
    let separator = std::path::MAIN_SEPARATOR;
    let parts: Vec<&str> = folder.split(separator).collect();
    for kept in (1..parts.len()).rev() {
        let tail = (kept / 2).max(1);
        let head = kept - tail;
        let joined = |range: &[&str]| range.join(&separator.to_string());
        let tail = joined(&parts[parts.len() - tail..]);
        let candidate = match head {
            0 => format!("…{separator}{tail}{suffix}"),
            _ => format!(
                "{}{separator}…{separator}{tail}{suffix}",
                joined(&parts[..head])
            ),
        };
        if fits(&candidate) {
            return candidate;
        }
    }
    full // Still too long: the layout trims the end with an ellipsis.
}
