//! Native progress window. The engine runs on a worker thread; this thread
//! draws its live counters with Direct2D. Buttons stay real Win32 buttons
//! (owner-drawn) so keyboard navigation and screen readers keep working.
mod format;
pub(crate) mod paint;
pub(crate) mod surface;
pub(crate) mod view;

use paint::{glyph, palette};
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
use surface::{Button, Surface, button_state, wide};
use unbloat::{
    Result, archiver,
    config::Config,
    display_path, error,
    i18n::{self, Language},
    output, scanner,
    tracker::{Phase, Tracker},
};
use view::ButtonKind;
use windows::{
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::{Com::*, LibraryLoader::GetModuleHandleW, SystemInformation::GetLocalTime},
        UI::{Controls::*, HiDpi::*, Input::KeyboardAndMouse::*, Shell::*, WindowsAndMessaging::*},
    },
    core::{HSTRING, PCWSTR, w},
};

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
    surface: Surface,
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
            .map(|b| view::button_width(&self.surface.graphics, b.label, b.glyph))
            .collect();
        let rects = view::button_row(self.height(), &widths);
        for button in &self.buttons {
            let index = visible.iter().position(|id| *id == button.id);
            self.surface.place(button.hwnd, index.map(|i| rects[i]));
        }
        self.surface.resize(view::WIDTH, self.height());
    }

    fn set_title(&mut self, title: String) {
        if title != self.title {
            unsafe {
                let _ = SetWindowTextW(self.surface.hwnd, PCWSTR(wide(&title).as_ptr()));
            }
            self.title = title;
        }
    }

    fn taskbar(&self, state: TBPFLAG, fraction: Option<f64>) {
        if let Some(taskbar) = &self.taskbar {
            unsafe {
                let _ = taskbar.SetProgressState(self.surface.hwnd, state);
                if let Some(fraction) = fraction {
                    let _ = taskbar.SetProgressValue(
                        self.surface.hwnd,
                        (fraction * 1000.0) as u64,
                        1000,
                    );
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
            let _ = InvalidateRect(Some(self.surface.hwnd), None, false);
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
                    let _ = DestroyWindow(self.surface.hwnd);
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
            let _ = KillTimer(Some(self.surface.hwnd), TIMER);
            self.layout();
            if let Some(close) = self.buttons.iter().find(|b| b.id == CLOSE) {
                let _ = SetFocus(Some(close.hwnd));
            }
            // Draw attention when the user switched to another window meanwhile.
            let flash = FLASHWINFO {
                cbSize: size_of::<FLASHWINFO>() as u32,
                hwnd: self.surface.hwnd,
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
            // Rounded down: "100% smaller" would claim an empty archive.
            let smaller = (summary.original - summary.archive) * 100 / summary.original;
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
        // Each view is built first; drawing then only borrows the surface.
        match &self.outcome {
            Outcome::Working | Outcome::Cancelled => {
                let screen = self.working_view();
                self.surface.paint(|canvas| view::working(canvas, &screen));
            }
            Outcome::Done(summary) => {
                let screen = self.done_view(summary);
                self.surface.paint(|canvas| view::done(canvas, &screen));
            }
            Outcome::Failed(message) => {
                let screen = view::Failed {
                    title: i18n::text("progress.failed_title"),
                    message: message.clone(),
                };
                self.surface.paint(|canvas| view::failed(canvas, &screen));
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
        let (label, glyph, state) = (button.label, button.glyph, button_state(item));
        self.surface.paint_control(item, |canvas, size| {
            view::button(canvas, size, label, glyph, kind, state)
        });
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
            window.paint();
            LRESULT(0)
        }
        WM_SIZE => {
            window.surface.resized(lparam);
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
            window.surface.dpi_changed(wparam, lparam);
            window.layout();
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
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
        surface: Surface::new()?,
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
            if !IsDialogMessageW(window.surface.hwnd, &message).as_bool() {
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

#[allow(clippy::manual_dangling_ptr)] // MAKEINTRESOURCE: icon resource ID 1.
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
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            640,
            440,
            None,
            None,
            Some(instance.into()),
            None,
        )?;
        window.surface.attach(hwnd);
        for (id, key, glyph) in [
            (CANCEL, "common.cancel", Some(glyph::CLOSE)),
            (
                OPEN_FOLDER,
                "progress.open_folder",
                Some(glyph::FOLDER_OPEN),
            ),
            (CLOSE, "common.close", None),
        ] {
            window
                .buttons
                .push(window.surface.button(id, i18n::text(key), glyph)?);
        }
        window.layout();
        window.surface.center();
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
