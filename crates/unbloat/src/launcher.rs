//! Native first-launch window. Opening the executable explains its use and
//! installs or uninstalls the Explorer integration.
use crate::setup::{self, Package};
use crate::ui::{
    paint::{palette, strong_ranges},
    surface::{Button, Surface, button_state, wide},
    view::{
        ButtonKind,
        notice::{self, Notice, Tone},
    },
};
use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver},
    thread,
    time::Instant,
};
use unbloat::{Result, error, i18n};
use windows::{
    Win32::{
        Foundation::*,
        Graphics::Gdi::{CreateSolidBrush, InvalidateRect},
        System::{
            Console::{
                ATTACH_PARENT_PROCESS, AttachConsole, FreeConsole, GetStdHandle, STD_OUTPUT_HANDLE,
            },
            LibraryLoader::GetModuleHandleW,
        },
        UI::{
            Controls::{DRAWITEMSTRUCT, ODT_STATIC},
            HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetThreadDpiAwarenessContext},
            Input::KeyboardAndMouse::EnableWindow,
            WindowsAndMessaging::*,
        },
    },
    core::{PCWSTR, w},
};

const INSTALL: i32 = 10;
const CLOSE: i32 = 2; // IDCANCEL: Esc
const DESCRIPTION: i32 = 30;
const UNINSTALL: i32 = 40;
const TIMER: usize = 1;

/// GUI release builds still support CLI output and redirected stdout pipes.
pub fn attach_parent_console() {
    unsafe {
        if !GetStdHandle(STD_OUTPUT_HANDLE).is_ok_and(|handle| !handle.is_invalid()) {
            let _ = AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    Install,
    Uninstall,
}

enum Screen {
    Welcome,
    Installed { now: bool },
    Incomplete,
    Working(Action),
    Uninstalled,
    Failed(Action, String),
}

struct State {
    /// A complete package beside this executable, when it isn't the installed copy.
    package: Option<Package>,
    /// The installation this window installs to or removes.
    target: PathBuf,
    screen: Screen,
    pending: Option<(Action, Receiver<Result<()>>)>,
    started: Instant,
    surface: Surface,
    /// The text, owner-drawn: screen readers and tests read its window text.
    description: HWND,
    buttons: Vec<Button>,
}

impl State {
    fn notice(&self) -> Notice {
        let text = |key| i18n::text(key).to_owned();
        let (tone, title, body) = match &self.screen {
            Screen::Welcome => (
                Tone::Info,
                text("launcher.welcome_title"),
                text("launcher.welcome"),
            ),
            Screen::Installed { now: true } => (
                Tone::Success,
                text("launcher.installed"),
                text("launcher.usage"),
            ),
            Screen::Installed { now: false } => (
                Tone::Success,
                text("launcher.already_installed"),
                text("launcher.usage"),
            ),
            Screen::Incomplete => (
                Tone::Warning,
                text("launcher.incomplete_title"),
                text("launcher.incomplete_package"),
            ),
            Screen::Working(Action::Install) => (
                Tone::Working,
                text("launcher.installing"),
                text("launcher.installing_body"),
            ),
            Screen::Working(Action::Uninstall) => (
                Tone::Working,
                text("launcher.uninstalling"),
                text("launcher.uninstalling_body"),
            ),
            Screen::Uninstalled => (
                Tone::Neutral,
                text("launcher.uninstalled"),
                text("launcher.uninstalled_body"),
            ),
            Screen::Failed(action, message) => (
                Tone::Danger,
                text(match action {
                    Action::Install => "launcher.failed",
                    Action::Uninstall => "launcher.uninstall_failed",
                }),
                // An error is plain text, never markup.
                message.replace("**", "*\u{200b}*"),
            ),
        };
        Notice {
            tone,
            title,
            body,
            pulse: (self.started.elapsed().as_millis() % 1600) as f32 / 1600.0,
        }
    }

    /// Sizes the window to the current text and places the controls.
    fn show(&mut self, screen: Screen) {
        self.screen = screen;
        let notice = self.notice();
        let height = notice::height(&self.surface.graphics, &notice);
        // What a screen reader announces: the title, then the body without markup.
        let plain = format!("{}\r\n\r\n{}", notice.title, strong_ranges(&notice.body).0)
            .replace("\r\n", "\n")
            .replace('\n', "\r\n");
        unsafe {
            let _ = SetWindowTextW(self.description, PCWSTR(wide(&plain).as_ptr()));
        }
        let content = crate::ui::paint::Rect::new(0.0, 0.0, notice::WIDTH, height - notice::FOOTER);
        self.surface.place(self.description, Some(content));
        let widths: Vec<f32> = self
            .buttons
            .iter()
            .map(|b| crate::ui::view::button_width(&self.surface.graphics, b.label, b.glyph))
            .collect();
        for (button, rect) in self.buttons.iter().zip(notice::buttons(height, &widths)) {
            self.surface.place(button.hwnd, Some(rect));
        }
        self.surface.resize(notice::WIDTH, height);
        unsafe {
            let _ = InvalidateRect(Some(self.description), None, false);
        }
    }

    fn enable(&self, install: bool, uninstall: bool) {
        for button in &self.buttons {
            let on = match button.id {
                INSTALL => install,
                _ => uninstall,
            };
            unsafe {
                let _ = EnableWindow(button.hwnd, on);
            }
        }
    }

    fn start(&mut self, action: Action) {
        if self.pending.is_some() {
            return;
        }
        let target = self.target.clone();
        let job: Box<dyn FnOnce() -> Result<()> + Send> = match action {
            Action::Install => {
                let Some(package) = self.package.clone() else {
                    return;
                };
                Box::new(move || setup::install(&package, &target).map(drop))
            }
            Action::Uninstall => {
                let question = wide(i18n::text("launcher.uninstall_confirm"));
                let title = wide(i18n::text("app.name"));
                let answer = unsafe {
                    MessageBoxW(
                        Some(self.surface.hwnd),
                        PCWSTR(question.as_ptr()),
                        PCWSTR(title.as_ptr()),
                        MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2,
                    )
                };
                if answer != IDYES {
                    return;
                }
                Box::new(move || setup::uninstall(&target).map(drop))
            }
        };
        let (sender, receiver) = mpsc::channel();
        self.pending = Some((action, receiver));
        self.enable(false, false);
        self.show(Screen::Working(action));
        thread::spawn(move || {
            let _ = sender.send(job());
        });
    }

    fn tick(&mut self) {
        let Some((action, receiver)) = &self.pending else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => {
                // Animate the progress bar.
                unsafe {
                    let _ = InvalidateRect(Some(self.description), None, false);
                }
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => Err(error(i18n::text("launcher.interrupted"))),
        };
        let action = *action;
        self.pending = None;
        let can_install = self.package.is_some();
        match (action, result) {
            (Action::Install, Ok(())) => {
                self.enable(false, true);
                self.show(Screen::Installed { now: true });
            }
            (Action::Uninstall, Ok(())) => {
                self.enable(can_install, false);
                self.show(Screen::Uninstalled);
            }
            (action, Err(e)) => {
                self.enable(can_install, setup::is_installed(&self.target));
                self.show(Screen::Failed(action, e.to_string()));
            }
        }
    }

    fn draw(&mut self, item: &DRAWITEMSTRUCT) {
        if item.CtlType == ODT_STATIC {
            let notice = self.notice();
            self.surface
                .paint_control(item, |canvas, size| notice::content(canvas, size, &notice));
            return;
        }
        let Some(button) = self.buttons.iter().find(|b| b.id == item.CtlID as i32) else {
            return;
        };
        let kind = match button.id {
            INSTALL => ButtonKind::Primary,
            _ => ButtonKind::Danger,
        };
        let (label, glyph, state) = (button.label, button.glyph, button_state(item));
        self.surface.paint_control(item, |canvas, size| {
            crate::ui::view::button(canvas, size, label, glyph, kind, state)
        });
    }
}

unsafe extern "system" fn procedure(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let pointer = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut State;
    if pointer.is_null() {
        return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
    }
    let state = unsafe { &mut *pointer };
    match message {
        WM_PAINT => {
            let height = notice::height(&state.surface.graphics, &state.notice());
            state.surface.paint(|canvas| notice::frame(canvas, height));
            LRESULT(0)
        }
        WM_SIZE => {
            state.surface.resized(lparam);
            LRESULT(0)
        }
        WM_DRAWITEM => {
            state.draw(unsafe { &*(lparam.0 as *const DRAWITEMSTRUCT) });
            LRESULT(1)
        }
        WM_DPICHANGED => {
            state.surface.dpi_changed(wparam, lparam);
            let screen = std::mem::replace(&mut state.screen, Screen::Welcome);
            state.show(screen);
            LRESULT(0)
        }
        WM_COMMAND => {
            match (wparam.0 & 0xffff) as i32 {
                INSTALL => state.start(Action::Install),
                UNINSTALL => state.start(Action::Uninstall),
                CLOSE if state.pending.is_none() => unsafe {
                    let _ = DestroyWindow(hwnd);
                },
                _ => {}
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            if state.pending.is_none() {
                unsafe {
                    let _ = DestroyWindow(hwnd);
                }
            }
            LRESULT(0)
        }
        WM_TIMER => {
            state.tick();
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe {
                PostQuitMessage(0);
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

#[allow(clippy::manual_dangling_ptr)] // Win32 resource/control IDs are integer-valued pointers.
pub fn run() -> Result<()> {
    unsafe {
        let _ = FreeConsole(); // Debug builds may have inherited/created a console.
        let _ = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let directory = std::env::current_exe()?
        .parent()
        .ok_or_else(|| error(i18n::text("common.app_directory")))?
        .to_path_buf();
    // The installed copy manages itself; a package manages the user's installation.
    let running_installed = setup::is_installed(&directory);
    let target = if running_installed {
        directory
    } else {
        setup::target_dir()?
    };
    let package = if running_installed {
        None
    } else {
        Package::current().ok()
    };
    let installed = setup::is_installed(&target);
    let screen = if installed {
        Screen::Installed { now: false }
    } else if package.is_some() {
        Screen::Welcome
    } else {
        Screen::Incomplete
    };
    let can_install = package.is_some();
    let mut state = Box::new(State {
        package,
        target,
        screen: Screen::Welcome,
        pending: None,
        started: Instant::now(),
        surface: Surface::new()?,
        description: HWND::default(),
        buttons: Vec::new(),
    });
    let hwnd = unsafe {
        let instance = GetModuleHandleW(None)?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(procedure),
            hInstance: instance.into(),
            lpszClassName: w!("UnbloatWelcome"),
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
            PCWSTR(wide(i18n::text("launcher.title")).as_ptr()),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            600,
            300,
            None,
            None,
            Some(instance.into()),
            None,
        )?;
        state.surface.attach(hwnd);
        state.description = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("STATIC"),
            PCWSTR::null(),
            WS_CHILD | WINDOW_STYLE(0x0D), // SS_OWNERDRAW
            0,
            0,
            0,
            0,
            Some(hwnd),
            Some(HMENU(DESCRIPTION as usize as *mut _)),
            Some(instance.into()),
            None,
        )?;
        state.buttons = vec![
            state
                .surface
                .button(INSTALL, i18n::text("launcher.install"), None)?,
            state
                .surface
                .button(UNINSTALL, i18n::text("launcher.uninstall"), None)?,
        ];
        state.enable(can_install, installed);
        state.show(screen);
        state.surface.center();
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, (&mut *state as *mut State) as isize);
        if SetTimer(Some(hwnd), TIMER, 50, None) == 0 {
            let _ = DestroyWindow(hwnd);
            return Err(windows::core::Error::from_thread().into());
        }
        let _ = ShowWindow(hwnd, SW_SHOW);
        hwnd
    };
    unsafe {
        let mut message = MSG::default();
        loop {
            let result = GetMessageW(&mut message, None, 0, 0).0;
            if result == -1 {
                return Err(windows::core::Error::from_thread().into());
            }
            if result == 0 {
                break;
            }
            if !IsDialogMessageW(hwnd, &message).as_bool() {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        let _ = KillTimer(Some(hwnd), TIMER);
    }
    Ok(())
}
