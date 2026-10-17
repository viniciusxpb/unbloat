//! Native progress window. The engine runs on a worker thread; this thread
//! draws its live counters with Direct2D. Buttons stay real Win32 buttons
//! (owner-drawn) so keyboard navigation and screen readers keep working.
mod format;
mod paint;
mod view;

use paint::{Canvas, Graphics, glyph, palette};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
use unbloat::{
    Result, archiver,
    config::Config,
    display_path, error,
    i18n::{self, Language},
    output, scanner,
    tracker::{Phase, Tracker},
};
use view::{ButtonKind, ButtonState};
use windows::{
    Win32::{
        Foundation::*,
        Graphics::{
            Direct2D::{Common::*, *},
            Dwm::*,
            Dxgi::Common::*,
            Gdi::*,
        },
        System::{Com::*, LibraryLoader::GetModuleHandleW, SystemInformation::GetLocalTime},
        UI::{Controls::*, HiDpi::*, Input::KeyboardAndMouse::*, Shell::*, WindowsAndMessaging::*},
    },
    core::{BOOL, HSTRING, PCWSTR, w},
};

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

pub fn show_error(message: &str) {
    let message = wide(message);
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(message.as_ptr()),
            PCWSTR(wide(i18n::text("app.name")).as_ptr()),
            MB_OK | MB_ICONERROR,
        );
    }
}

/// A failure the progress window already showed; `main` must not show it again.
#[derive(Debug)]
pub struct Reported(pub String);
impl std::fmt::Display for Reported {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Reported {}

const CLOSE: i32 = 1; // IDOK: Enter
const CANCEL: i32 = 2; // IDCANCEL: Esc
const OPEN_FOLDER: i32 = 100;
const TIMER: usize = 1;
/// Speed and time-remaining estimates use this much recent history.
const WINDOW: Duration = Duration::from_secs(4);

struct Summary {
    path: PathBuf,
    original: u64,
    archive: u64,
    files: u64,
    seconds: f64,
}

enum Outcome {
    Working,
    Done(Summary),
    Failed(String),
    Cancelled,
}

struct Button {
    hwnd: HWND,
    id: i32,
    label: &'static str,
    glyph: Option<char>,
}

struct Sample {
    at: Instant,
    fraction: f64,
    bytes: u64,
}

struct Window {
    cancel: Arc<AtomicBool>,
    tracker: Arc<Tracker>,
    current: Arc<Mutex<String>>,
    preview: Arc<Mutex<Option<String>>>,
    result: mpsc::Receiver<Result<PathBuf>>,
    outcome: Outcome,
    cancelling: bool,
    started: Instant,
    samples: VecDeque<Sample>,
    sampled: Phase,
    name: String,
    folder: String,
    language: Language,
    graphics: Graphics,
    target: Option<ID2D1HwndRenderTarget>,
    buttons_target: Option<ID2D1DCRenderTarget>,
    hwnd: HWND,
    dpi: u32,
    buttons: Vec<Button>,
    taskbar: Option<ITaskbarList3>,
    title: String,
}

impl Window {
    fn height(&self) -> f32 {
        match self.outcome {
            Outcome::Working | Outcome::Cancelled => view::WORKING_HEIGHT,
            Outcome::Done(_) => view::DONE_HEIGHT,
            Outcome::Failed(_) => view::FAILED_HEIGHT,
        }
    }

    fn px(&self, dips: f32) -> i32 {
        (dips * self.dpi as f32 / 96.0).round() as i32
    }

    /// Places the buttons of the current screen and sizes the window to it.
    fn layout(&mut self) {
        let visible: &[i32] = match self.outcome {
            Outcome::Working | Outcome::Cancelled => &[CANCEL],
            Outcome::Done(_) => &[OPEN_FOLDER, CLOSE],
            Outcome::Failed(_) => &[CLOSE],
        };
        let widths: Vec<f32> = visible
            .iter()
            .filter_map(|id| self.buttons.iter().find(|b| b.id == *id))
            .map(|b| view::button_width(&self.graphics, b.label, b.glyph))
            .collect();
        let rects = view::button_row(self.height(), &widths);
        unsafe {
            for button in &self.buttons {
                match visible.iter().position(|id| *id == button.id) {
                    Some(index) => {
                        let r = rects[index];
                        let _ = SetWindowPos(
                            button.hwnd,
                            None,
                            self.px(r.x),
                            self.px(r.y),
                            self.px(r.right()) - self.px(r.x),
                            self.px(r.bottom()) - self.px(r.y),
                            SWP_NOZORDER | SWP_NOACTIVATE,
                        );
                        let _ = ShowWindow(button.hwnd, SW_SHOW);
                    }
                    None => {
                        let _ = ShowWindow(button.hwnd, SW_HIDE);
                    }
                }
            }
            let mut frame = RECT {
                left: 0,
                top: 0,
                right: self.px(view::WIDTH),
                bottom: self.px(self.height()),
            };
            let _ = AdjustWindowRectExForDpi(&mut frame, style(), false, WS_EX_APPWINDOW, self.dpi);
            let _ = SetWindowPos(
                self.hwnd,
                None,
                0,
                0,
                frame.right - frame.left,
                frame.bottom - frame.top,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    fn set_title(&mut self, title: String) {
        if title != self.title {
            unsafe {
                let _ = SetWindowTextW(self.hwnd, PCWSTR(wide(&title).as_ptr()));
            }
            self.title = title;
        }
    }

    fn taskbar(&self, state: TBPFLAG, fraction: Option<f64>) {
        if let Some(taskbar) = &self.taskbar {
            unsafe {
                let _ = taskbar.SetProgressState(self.hwnd, state);
                if let Some(fraction) = fraction {
                    let _ = taskbar.SetProgressValue(self.hwnd, (fraction * 1000.0) as u64, 1000);
                }
            }
        }
    }

    fn tick(&mut self) {
        if !matches!(self.outcome, Outcome::Working) {
            return;
        }
        match self.result.try_recv() {
            Ok(result) => return self.finish(result),
            Err(mpsc::TryRecvError::Disconnected) => {
                return self.finish(Err(error(i18n::text("progress.interrupted"))));
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }
        let snapshot = self.tracker.snapshot();
        let now = Instant::now();
        if snapshot.phase != self.sampled {
            self.samples.clear();
            self.sampled = snapshot.phase;
        }
        self.samples.push_back(Sample {
            at: now,
            fraction: snapshot.fraction(),
            bytes: match snapshot.phase {
                Phase::Verifying => snapshot.bytes_verified,
                _ => snapshot.bytes_read,
            },
        });
        while self.samples.len() > 2 && now - self.samples[0].at > WINDOW {
            self.samples.pop_front();
        }
        let percent = (snapshot.fraction() * 100.0).floor() as u32;
        let (title, state) = match (self.cancelling, snapshot.phase) {
            (true, _) => (
                i18n::text("progress.window_cancelling").to_owned(),
                TBPF_PAUSED,
            ),
            (_, Phase::Scanning) => (
                i18n::text("progress.window_scanning").to_owned(),
                TBPF_INDETERMINATE,
            ),
            _ => (
                i18n::message("progress.window_working", &[&percent]),
                TBPF_NORMAL,
            ),
        };
        self.set_title(title);
        self.taskbar(
            state,
            (snapshot.phase != Phase::Scanning).then(|| snapshot.fraction()),
        );
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    /// (bytes per second, seconds remaining) over the recent samples.
    fn rates(&self) -> (Option<f64>, Option<f64>) {
        let (Some(first), Some(last)) = (self.samples.front(), self.samples.back()) else {
            return (None, None);
        };
        let seconds = (last.at - first.at).as_secs_f64();
        if seconds < 0.75 {
            return (None, None);
        }
        let speed = (last.bytes - first.bytes) as f64 / seconds;
        let progress = (last.fraction - first.fraction) / seconds;
        let remaining = (progress > 0.0 && self.started.elapsed() > Duration::from_secs(2))
            .then(|| (1.0 - last.fraction) / progress);
        (Some(speed), remaining)
    }

    fn request_cancel(&mut self) {
        if self.cancelling || !matches!(self.outcome, Outcome::Working) {
            return;
        }
        self.cancelling = true;
        self.cancel.store(true, Ordering::Relaxed);
        if let Some(button) = self.buttons.iter().find(|b| b.id == CANCEL) {
            unsafe {
                let _ = EnableWindow(button.hwnd, false);
            }
        }
        self.tick();
    }

    fn finish(&mut self, result: Result<PathBuf>) {
        self.outcome = match result {
            // A cancellation that lost the race still produced an archive: say so.
            Ok(path) => {
                let snapshot = self.tracker.snapshot();
                Outcome::Done(Summary {
                    archive: std::fs::metadata(&path).map_or(snapshot.bytes_written, |m| m.len()),
                    path,
                    original: snapshot.total_bytes,
                    files: snapshot.total_files,
                    seconds: self.started.elapsed().as_secs_f64(),
                })
            }
            Err(_) if self.cancelling => Outcome::Cancelled,
            Err(e) => Outcome::Failed(e.to_string()),
        };
        unsafe {
            match &self.outcome {
                Outcome::Cancelled => {
                    let _ = DestroyWindow(self.hwnd);
                    return;
                }
                Outcome::Done(_) => {
                    self.set_title(i18n::text("progress.window_done").to_owned());
                    self.taskbar(TBPF_NOPROGRESS, None);
                }
                Outcome::Failed(_) => {
                    self.set_title(i18n::text("progress.window_failed").to_owned());
                    self.taskbar(TBPF_ERROR, Some(1.0));
                }
                Outcome::Working => {}
            }
            let _ = KillTimer(Some(self.hwnd), TIMER);
            self.layout();
            if let Some(close) = self.buttons.iter().find(|b| b.id == CLOSE) {
                let _ = SetFocus(Some(close.hwnd));
            }
            // Draw attention when the user switched to another window meanwhile.
            let flash = FLASHWINFO {
                cbSize: size_of::<FLASHWINFO>() as u32,
                hwnd: self.hwnd,
                dwFlags: FLASHW_TRAY | FLASHW_TIMERNOFG,
                uCount: 0,
                dwTimeout: 0,
            };
            let _ = FlashWindowEx(&flash);
        }
    }

    fn working_view(&self) -> view::Working {
        let language = self.language;
        let snapshot = self.tracker.snapshot();
        let scanning = snapshot.phase == Phase::Scanning;
        let elapsed = self.started.elapsed();
        let (speed, remaining) = self.rates();
        let (status, status_color) = match (self.cancelling, snapshot.phase) {
            (true, _) => ("progress.status_cancelling", palette::AMBER),
            (_, Phase::Scanning) => ("progress.status_scanning", palette::GREEN),
            (_, Phase::Compressing) => ("progress.status_compressing", palette::GREEN),
            _ => ("progress.status_verifying", palette::GREEN),
        };
        let processed = match snapshot.phase {
            Phase::Verifying | Phase::Done => snapshot.bytes_verified,
            _ => snapshot.bytes_read,
        };
        let dash = || "—".to_owned();
        let finishes = remaining.map(|seconds| {
            let now = unsafe { GetLocalTime() };
            format::time_of_day(now.wHour, now.wMinute, now.wSecond, seconds)
        });
        let preview = self
            .preview
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let current = if self.cancelling {
            i18n::text("progress.cancelling").to_owned()
        } else if scanning {
            i18n::text("progress.scanning").to_owned()
        } else {
            self.current
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone()
        };
        view::Working {
            status: i18n::text(status),
            status_color,
            name: self.name.clone(),
            path: (
                self.folder.clone(),
                preview.map(|name| format!(" → {name}")).unwrap_or_default(),
            ),
            percent: (!scanning)
                .then(|| ((snapshot.fraction() * 100.0).floor() as u32).to_string()),
            fraction: (!scanning).then(|| snapshot.fraction() as f32),
            pulse: (elapsed.as_millis() % 1600) as f32 / 1600.0,
            amount: (!scanning).then(|| {
                format::emphasis(
                    i18n::text("progress.amount"),
                    &format::bytes(processed, language),
                    &[&format::bytes(snapshot.total_bytes, language)],
                )
            }),
            remaining: remaining.filter(|_| !self.cancelling).map(|seconds| {
                format::emphasis(
                    i18n::text("progress.remaining"),
                    &format::remaining(seconds),
                    &[],
                )
            }),
            cards: [
                view::Card {
                    glyph: glyph::CLOCK,
                    label: i18n::text("progress.elapsed"),
                    value: format::clock(elapsed.as_secs()),
                },
                view::Card {
                    glyph: glyph::SPEED,
                    label: i18n::text("progress.speed"),
                    value: speed.map_or_else(dash, |s| format::speed(s, language)),
                },
                view::Card {
                    glyph: glyph::SHRINK,
                    label: i18n::text("progress.ratio"),
                    value: match snapshot.bytes_read {
                        0 => dash(),
                        read => format!("{}%", snapshot.bytes_written * 100 / read),
                    },
                },
                view::Card {
                    glyph: glyph::FILES,
                    label: i18n::text("progress.files"),
                    value: if scanning {
                        dash()
                    } else {
                        format!(
                            "{} / {}",
                            format::count(snapshot.files_done, language),
                            format::count(snapshot.total_files, language)
                        )
                    },
                },
                view::Card {
                    glyph: glyph::PACKAGE,
                    label: i18n::text("progress.archive_size"),
                    value: if scanning {
                        dash()
                    } else {
                        format::bytes(snapshot.bytes_written, language)
                    },
                },
                view::Card {
                    glyph: glyph::FLAG,
                    label: i18n::text("progress.finishes"),
                    value: finishes.filter(|_| !self.cancelling).unwrap_or_else(dash),
                },
            ],
            current,
        }
    }

    fn done_view(&self, summary: &Summary) -> view::Done {
        let language = self.language;
        let saved = if summary.archive < summary.original {
            let smaller = 100 - summary.archive * 100 / summary.original;
            format::emphasis(
                i18n::text("progress.saved"),
                &format::bytes(summary.original - summary.archive, language),
                &[&smaller],
            )
        } else {
            [
                i18n::text("progress.no_savings").to_owned(),
                String::new(),
                String::new(),
            ]
        };
        view::Done {
            title: i18n::text("progress.done_title"),
            path: (
                summary.path.parent().map(display_path).unwrap_or_default(),
                summary
                    .path
                    .file_name()
                    .map(|name| format!("{}{}", std::path::MAIN_SEPARATOR, name.to_string_lossy()))
                    .unwrap_or_default(),
            ),
            original_label: i18n::text("progress.original"),
            original: format::bytes(summary.original, language),
            archive: format::bytes(summary.archive, language),
            fraction: if summary.original == 0 {
                1.0
            } else {
                (summary.archive as f64 / summary.original as f64).min(1.0) as f32
            },
            saved,
            cards: [
                view::Card {
                    glyph: glyph::CLOCK,
                    label: i18n::text("progress.total_time"),
                    value: format::clock(summary.seconds.round() as u64),
                },
                view::Card {
                    glyph: glyph::SPEED,
                    label: i18n::text("progress.average_speed"),
                    value: format::speed(
                        summary.original as f64 / summary.seconds.max(0.001),
                        language,
                    ),
                },
                view::Card {
                    glyph: glyph::FILES,
                    label: i18n::text("progress.files"),
                    value: format::count(summary.files, language),
                },
            ],
        }
    }

    fn paint(&mut self) {
        unsafe {
            if self.target.is_none() {
                let mut client = RECT::default();
                let _ = GetClientRect(self.hwnd, &mut client);
                let properties = D2D1_RENDER_TARGET_PROPERTIES {
                    dpiX: self.dpi as f32,
                    dpiY: self.dpi as f32,
                    ..Default::default()
                };
                let window = D2D1_HWND_RENDER_TARGET_PROPERTIES {
                    hwnd: self.hwnd,
                    pixelSize: D2D_SIZE_U {
                        width: client.right as u32,
                        height: client.bottom as u32,
                    },
                    presentOptions: D2D1_PRESENT_OPTIONS_NONE,
                };
                self.target = self
                    .graphics
                    .d2d
                    .CreateHwndRenderTarget(&properties, &window)
                    .ok();
            }
            let Some(target) = self.target.clone() else {
                return;
            };
            target.BeginDraw();
            target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_CLEARTYPE);
            let canvas = Canvas {
                target: &target,
                graphics: &self.graphics,
            };
            match &self.outcome {
                Outcome::Working | Outcome::Cancelled => {
                    view::working(&canvas, &self.working_view())
                }
                Outcome::Done(summary) => view::done(&canvas, &self.done_view(summary)),
                Outcome::Failed(message) => view::failed(
                    &canvas,
                    &view::Failed {
                        title: i18n::text("progress.failed_title"),
                        message: message.clone(),
                    },
                ),
            }
            if target.EndDraw(None, None).is_err() {
                // Device lost (D2DERR_RECREATE_TARGET): rebuild on the next paint.
                self.target = None;
            }
        }
    }

    fn draw_button(&mut self, item: &DRAWITEMSTRUCT) {
        let Some(button) = self.buttons.iter().find(|b| b.id == item.CtlID as i32) else {
            return;
        };
        let kind = match (button.id, &self.outcome) {
            (CANCEL, _) => ButtonKind::Danger,
            (CLOSE, Outcome::Done(_)) => ButtonKind::Primary,
            _ => ButtonKind::Neutral,
        };
        let flags = item.itemState.0;
        let state = ButtonState {
            hot: unsafe { GetWindowLongPtrW(item.hwndItem, GWLP_USERDATA) } != 0,
            pressed: flags & ODS_SELECTED.0 != 0,
            focused: flags & ODS_FOCUS.0 != 0 && flags & ODS_NOFOCUSRECT.0 == 0,
            disabled: flags & ODS_DISABLED.0 != 0,
        };
        let (label, glyph) = (button.label, button.glyph);
        unsafe {
            if self.buttons_target.is_none() {
                let properties = D2D1_RENDER_TARGET_PROPERTIES {
                    pixelFormat: D2D1_PIXEL_FORMAT {
                        format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        alphaMode: D2D1_ALPHA_MODE_IGNORE,
                    },
                    ..Default::default()
                };
                self.buttons_target = self.graphics.d2d.CreateDCRenderTarget(&properties).ok();
            }
            let Some(target) = self.buttons_target.clone() else {
                return;
            };
            target.SetDpi(self.dpi as f32, self.dpi as f32);
            if target.BindDC(item.hDC, &item.rcItem).is_err() {
                return;
            }
            target.BeginDraw();
            target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_CLEARTYPE);
            let scale = 96.0 / self.dpi as f32;
            let size = (
                (item.rcItem.right - item.rcItem.left) as f32 * scale,
                (item.rcItem.bottom - item.rcItem.top) as f32 * scale,
            );
            let canvas = Canvas {
                target: &target,
                graphics: &self.graphics,
            };
            view::button(&canvas, size, label, glyph, kind, state);
            if target.EndDraw(None, None).is_err() {
                self.buttons_target = None;
            }
        }
    }

    fn open_folder(&self) {
        let Outcome::Done(summary) = &self.outcome else {
            return;
        };
        unsafe {
            let item = ILCreateFromPathW(&HSTRING::from(display_path(&summary.path)));
            if !item.is_null() {
                let _ = SHOpenFolderAndSelectItems(item, None, 0);
                ILFree(Some(item));
            }
        }
    }

    fn dpi_changed(&mut self, dpi: u32, suggested: &RECT) {
        self.dpi = dpi;
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                None,
                suggested.left,
                suggested.top,
                suggested.right - suggested.left,
                suggested.bottom - suggested.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            if let Some(target) = &self.target {
                target.SetDpi(dpi as f32, dpi as f32);
            }
        }
        self.layout();
    }
}

fn style() -> WINDOW_STYLE {
    WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_CLIPCHILDREN
}

unsafe extern "system" fn procedure(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let pointer = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut Window;
    if pointer.is_null() {
        return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
    }
    let window = unsafe { &mut *pointer };
    match message {
        WM_PAINT => {
            let mut paint = PAINTSTRUCT::default();
            unsafe { BeginPaint(hwnd, &mut paint) };
            window.paint();
            unsafe {
                let _ = EndPaint(hwnd, &paint);
            }
            LRESULT(0)
        }
        WM_SIZE => {
            if let Some(target) = &window.target {
                let size = D2D_SIZE_U {
                    width: (lparam.0 & 0xffff) as u32,
                    height: (lparam.0 >> 16 & 0xffff) as u32,
                };
                unsafe {
                    let _ = target.Resize(&size);
                }
            }
            LRESULT(0)
        }
        WM_DRAWITEM => {
            window.draw_button(unsafe { &*(lparam.0 as *const DRAWITEMSTRUCT) });
            LRESULT(1)
        }
        WM_COMMAND => {
            match (wparam.0 & 0xffff) as i32 {
                CANCEL if matches!(window.outcome, Outcome::Working) => window.request_cancel(),
                CANCEL | CLOSE if !matches!(window.outcome, Outcome::Working) => unsafe {
                    let _ = DestroyWindow(hwnd);
                },
                OPEN_FOLDER => window.open_folder(),
                _ => {}
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            if matches!(window.outcome, Outcome::Working) {
                window.request_cancel();
            } else {
                unsafe {
                    let _ = DestroyWindow(hwnd);
                }
            }
            LRESULT(0)
        }
        WM_TIMER => {
            window.tick();
            LRESULT(0)
        }
        WM_DPICHANGED => {
            let suggested = unsafe { *(lparam.0 as *const RECT) };
            window.dpi_changed((wparam.0 & 0xffff) as u32, &suggested);
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

/// Tracks hover for the owner-drawn buttons (stored in their GWLP_USERDATA).
unsafe extern "system" fn button_procedure(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _: usize,
    _: usize,
) -> LRESULT {
    unsafe {
        match message {
            WM_MOUSEMOVE if GetWindowLongPtrW(hwnd, GWLP_USERDATA) == 0 => {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 1);
                let mut track = TRACKMOUSEEVENT {
                    cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                let _ = TrackMouseEvent(&mut track);
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
            WM_MOUSELEAVE => {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
            WM_ERASEBKGND => return LRESULT(1),
            _ => {}
        }
        DefSubclassProc(hwnd, message, wparam, lparam)
    }
}

/// "projeto" for one item, "3 itens" for several; and the folder holding them.
fn describe(paths: &[PathBuf], language: Language) -> (String, String) {
    let first = std::path::absolute(&paths[0]).unwrap_or_else(|_| paths[0].clone());
    let name = match paths {
        [one] => one
            .file_name()
            .map_or_else(|| display_path(one), |n| n.to_string_lossy().into_owned()),
        many => i18n::message(
            "progress.items",
            &[&format::count(many.len() as u64, language)],
        ),
    };
    let folder = first.parent().map_or_else(String::new, display_path);
    (name, folder)
}

#[allow(clippy::manual_dangling_ptr)] // Win32 MAKEINTRESOURCE/menu IDs are integers cast to pointers.
pub fn run(paths: Vec<PathBuf>, config: Config) -> Result<()> {
    unsafe {
        // Per-monitor DPI for this window only; the launcher keeps its layout.
        let _ = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let com = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
    let result = run_window(paths, config);
    if com {
        unsafe { CoUninitialize() };
    }
    result
}

fn run_window(paths: Vec<PathBuf>, config: Config) -> Result<()> {
    let language = i18n::language();
    let (name, folder) = describe(&paths, language);
    let cancel = Arc::new(AtomicBool::new(false));
    let tracker = Arc::new(Tracker::default());
    let current = Arc::new(Mutex::new(String::new()));
    let preview = Arc::new(Mutex::new(None));
    let (sender, receiver) = mpsc::channel();
    let pointer = Box::into_raw(Box::new(Window {
        cancel: cancel.clone(),
        tracker: tracker.clone(),
        current: current.clone(),
        preview: preview.clone(),
        result: receiver,
        outcome: Outcome::Working,
        cancelling: false,
        started: Instant::now(),
        samples: VecDeque::new(),
        sampled: Phase::Scanning,
        name,
        folder,
        language,
        graphics: Graphics::new()?,
        target: None,
        buttons_target: None,
        hwnd: HWND::default(),
        dpi: 96,
        buttons: Vec::new(),
        taskbar: None,
        title: String::new(),
    }));
    // Owned here again once the message loop ends; until the window procedure
    // is attached below, nothing else can reach it.
    let window = unsafe { &mut *pointer };
    let created = unsafe { create(window) };
    if let Err(e) = created {
        drop(unsafe { Box::from_raw(pointer) });
        return Err(e);
    }
    let worker = thread::spawn(move || {
        let result = (|| {
            let manifest = scanner::scan(&paths, &config.ignore, &cancel)?;
            *preview.lock().unwrap_or_else(|e| e.into_inner()) =
                output::preview_name(&manifest, &config.output);
            archiver::create_tracked(
                &manifest,
                &config,
                &cancel,
                &|text| {
                    *current.lock().unwrap_or_else(|e| e.into_inner()) = text.to_owned();
                    if config.logging.enabled {
                        eprintln!("{text}");
                    }
                },
                &tracker,
            )
        })();
        let _ = sender.send(result);
    });
    unsafe {
        let mut message = MSG::default();
        loop {
            let result = GetMessageW(&mut message, None, 0, 0).0;
            if result <= 0 {
                break;
            }
            if !IsDialogMessageW(window.hwnd, &message).as_bool() {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }
    let window = unsafe { Box::from_raw(pointer) };
    window.cancel.store(true, Ordering::Relaxed);
    worker
        .join()
        .map_err(|_| error(i18n::text("progress.engine_failed")))?;
    match window.outcome {
        Outcome::Done(_) | Outcome::Cancelled => Ok(()),
        Outcome::Failed(message) => Err(Box::new(Reported(message))),
        Outcome::Working => Err(error(i18n::text("progress.closed"))),
    }
}

#[allow(clippy::manual_dangling_ptr)]
unsafe fn create(window: &mut Window) -> Result<()> {
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(procedure),
            hInstance: instance.into(),
            lpszClassName: w!("UnbloatProgress"),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hIcon: LoadIconW(Some(instance.into()), PCWSTR(1usize as *const u16))
                .unwrap_or_default(),
            hbrBackground: CreateSolidBrush(COLORREF(palette::WINDOW.colorref())),
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            return Err(windows::core::Error::from_thread().into());
        }
        let hwnd = CreateWindowExW(
            WS_EX_APPWINDOW,
            class.lpszClassName,
            PCWSTR(wide(i18n::text("progress.window_scanning")).as_ptr()),
            style(),
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            640,
            440,
            None,
            None,
            Some(instance.into()),
            None,
        )?;
        window.hwnd = hwnd;
        window.dpi = GetDpiForWindow(hwnd).max(96);
        // Dark caption to match the content; colors are Windows 11 only.
        let dark = BOOL(1);
        let attribute = |name: DWMWINDOWATTRIBUTE, value: *const std::ffi::c_void, size: usize| {
            let _ = DwmSetWindowAttribute(hwnd, name, value, size as u32);
        };
        attribute(
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &dark as *const _ as _,
            size_of::<BOOL>(),
        );
        let caption = palette::WINDOW.colorref();
        let text = palette::TEXT.colorref();
        attribute(
            DWMWA_CAPTION_COLOR,
            &caption as *const _ as _,
            size_of::<u32>(),
        );
        attribute(DWMWA_TEXT_COLOR, &text as *const _ as _, size_of::<u32>());
        for (id, key, glyph) in [
            (CANCEL, "common.cancel", Some(glyph::CLOSE)),
            (
                OPEN_FOLDER,
                "progress.open_folder",
                Some(glyph::FOLDER_OPEN),
            ),
            (CLOSE, "common.close", None),
        ] {
            let label = i18n::text(key);
            let button = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("BUTTON"),
                PCWSTR(wide(label).as_ptr()),
                WS_CHILD | WS_TABSTOP | WINDOW_STYLE(BS_OWNERDRAW as u32),
                0,
                0,
                0,
                0,
                Some(hwnd),
                Some(HMENU(id as usize as *mut _)),
                Some(instance.into()),
                None,
            )?;
            let _ = SetWindowSubclass(button, Some(button_procedure), 1, 0);
            window.buttons.push(Button {
                hwnd: button,
                id,
                label,
                glyph,
            });
        }
        window.layout();
        // Like dialogs: no focus ring until the keyboard is used (UIS_SET, UISF_HIDEFOCUS).
        SendMessageW(hwnd, WM_CHANGEUISTATE, Some(WPARAM(1 << 16 | 1)), None);
        // Center on the monitor the window opened on.
        let mut frame = RECT::default();
        let _ = GetWindowRect(hwnd, &mut frame);
        let mut monitor = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(
            MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST),
            &mut monitor,
        )
        .as_bool()
        {
            let work = monitor.rcWork;
            let (width, height) = (frame.right - frame.left, frame.bottom - frame.top);
            let _ = SetWindowPos(
                hwnd,
                None,
                work.left + (work.right - work.left - width) / 2,
                work.top + (work.bottom - work.top - height) / 2,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        window.taskbar =
            CoCreateInstance::<_, ITaskbarList3>(&TaskbarList, None, CLSCTX_INPROC_SERVER)
                .ok()
                .filter(|taskbar| taskbar.HrInit().is_ok());
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, window as *mut Window as isize);
        if SetTimer(Some(hwnd), TIMER, 50, None) == 0 {
            let _ = DestroyWindow(hwnd);
            return Err(windows::core::Error::from_thread().into());
        }
        let _ = ShowWindow(hwnd, SW_SHOW);
        Ok(())
    }
}
