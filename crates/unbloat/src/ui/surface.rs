//! Window plumbing shared by the progress window and the launcher: Direct2D
//! drawing at the monitor's DPI, a dark caption, centering, and real Win32
//! buttons drawn by the window (keyboard and screen readers keep working).
use super::{
    paint::{Canvas, Graphics, Rect, palette},
    view::ButtonState,
};
use unbloat::Result;
use windows::{
    Win32::{
        Foundation::*,
        Graphics::{
            Direct2D::{Common::*, *},
            Dwm::*,
            Dxgi::Common::*,
            Gdi::*,
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::{Controls::*, HiDpi::*, Input::KeyboardAndMouse::*, Shell::*, WindowsAndMessaging::*},
    },
    core::{BOOL, PCWSTR, w},
};

pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

/// An owner-drawn push button; the window decides how it looks.
pub struct Button {
    pub hwnd: HWND,
    pub id: i32,
    pub label: &'static str,
    pub glyph: Option<char>,
}

pub struct Surface {
    pub graphics: Graphics,
    pub hwnd: HWND,
    pub dpi: u32,
    target: Option<ID2D1HwndRenderTarget>,
    controls: Option<ID2D1DCRenderTarget>,
}

impl Surface {
    pub fn new() -> Result<Self> {
        Ok(Self {
            graphics: Graphics::new()?,
            hwnd: HWND::default(),
            dpi: 96,
            target: None,
            controls: None,
        })
    }

    /// Adopts a freshly created top-level window: DPI, dark caption, and no
    /// focus rings until the keyboard is used (like dialogs).
    pub fn attach(&mut self, hwnd: HWND) {
        self.hwnd = hwnd;
        unsafe {
            self.dpi = GetDpiForWindow(hwnd).max(96);
            let attribute =
                |name: DWMWINDOWATTRIBUTE, value: *const std::ffi::c_void, size: usize| {
                    let _ = DwmSetWindowAttribute(hwnd, name, value, size as u32);
                };
            let dark = BOOL(1);
            attribute(
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                &dark as *const _ as _,
                size_of::<BOOL>(),
            );
            // Caption colors are Windows 11 only; Windows 10 keeps the dark default.
            let caption = palette::WINDOW.colorref();
            let text = palette::TEXT.colorref();
            attribute(
                DWMWA_CAPTION_COLOR,
                &caption as *const _ as _,
                size_of::<u32>(),
            );
            attribute(DWMWA_TEXT_COLOR, &text as *const _ as _, size_of::<u32>());
            SendMessageW(hwnd, WM_CHANGEUISTATE, Some(WPARAM(1 << 16 | 1)), None); // UIS_SET, UISF_HIDEFOCUS
        }
    }

    pub fn px(&self, dips: f32) -> i32 {
        (dips * self.dpi as f32 / 96.0).round() as i32
    }

    /// Positions and shows a child window, or hides it when `rect` is `None`.
    pub fn place(&self, child: HWND, rect: Option<Rect>) {
        unsafe {
            match rect {
                Some(r) => {
                    let _ = SetWindowPos(
                        child,
                        None,
                        self.px(r.x),
                        self.px(r.y),
                        self.px(r.right()) - self.px(r.x),
                        self.px(r.bottom()) - self.px(r.y),
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                    let _ = ShowWindow(child, SW_SHOW);
                }
                None => {
                    let _ = ShowWindow(child, SW_HIDE);
                }
            }
        }
    }

    /// Sizes the window so its client area is `width` × `height` DIPs.
    pub fn resize(&self, width: f32, height: f32) {
        unsafe {
            let style = WINDOW_STYLE(GetWindowLongW(self.hwnd, GWL_STYLE) as u32);
            let extended = WINDOW_EX_STYLE(GetWindowLongW(self.hwnd, GWL_EXSTYLE) as u32);
            let mut frame = RECT {
                left: 0,
                top: 0,
                right: self.px(width),
                bottom: self.px(height),
            };
            let _ = AdjustWindowRectExForDpi(&mut frame, style, false, extended, self.dpi);
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

    /// Centers the window on the work area of its monitor.
    pub fn center(&self) {
        unsafe {
            let mut frame = RECT::default();
            let _ = GetWindowRect(self.hwnd, &mut frame);
            let mut monitor = MONITORINFO {
                cbSize: size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            let nearest = MonitorFromWindow(self.hwnd, MONITOR_DEFAULTTONEAREST);
            if GetMonitorInfoW(nearest, &mut monitor).as_bool() {
                let work = monitor.rcWork;
                let (width, height) = (frame.right - frame.left, frame.bottom - frame.top);
                let _ = SetWindowPos(
                    self.hwnd,
                    None,
                    work.left + (work.right - work.left - width) / 2,
                    work.top + (work.bottom - work.top - height) / 2,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
        }
    }

    /// Draws the whole client area (WM_PAINT).
    pub fn paint(&mut self, draw: impl FnOnce(&Canvas)) {
        unsafe {
            let mut paint = PAINTSTRUCT::default();
            BeginPaint(self.hwnd, &mut paint);
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
            if let Some(target) = self.target.clone() {
                target.BeginDraw();
                target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_CLEARTYPE);
                draw(&Canvas {
                    target: &target,
                    graphics: &self.graphics,
                });
                if target.EndDraw(None, None).is_err() {
                    // Device lost (D2DERR_RECREATE_TARGET): rebuild on the next paint.
                    self.target = None;
                }
            }
            let _ = EndPaint(self.hwnd, &paint);
        }
    }

    /// Draws an owner-drawn control (WM_DRAWITEM); `draw` gets its size in DIPs.
    pub fn paint_control(&mut self, item: &DRAWITEMSTRUCT, draw: impl FnOnce(&Canvas, (f32, f32))) {
        unsafe {
            if self.controls.is_none() {
                let properties = D2D1_RENDER_TARGET_PROPERTIES {
                    pixelFormat: D2D1_PIXEL_FORMAT {
                        format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        alphaMode: D2D1_ALPHA_MODE_IGNORE,
                    },
                    ..Default::default()
                };
                self.controls = self.graphics.d2d.CreateDCRenderTarget(&properties).ok();
            }
            let Some(target) = self.controls.clone() else {
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
            draw(
                &Canvas {
                    target: &target,
                    graphics: &self.graphics,
                },
                size,
            );
            if target.EndDraw(None, None).is_err() {
                self.controls = None;
            }
        }
    }

    /// WM_SIZE: the render target follows the client area.
    pub fn resized(&self, lparam: LPARAM) {
        if let Some(target) = &self.target {
            let size = D2D_SIZE_U {
                width: (lparam.0 & 0xffff) as u32,
                height: (lparam.0 >> 16 & 0xffff) as u32,
            };
            unsafe {
                let _ = target.Resize(&size);
            }
        }
    }

    /// WM_DPICHANGED: takes the suggested frame; the caller lays out again.
    pub fn dpi_changed(&mut self, wparam: WPARAM, lparam: LPARAM) {
        self.dpi = (wparam.0 & 0xffff) as u32;
        unsafe {
            let suggested = *(lparam.0 as *const RECT);
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
                target.SetDpi(self.dpi as f32, self.dpi as f32);
            }
        }
    }

    /// A hidden owner-drawn button; `place` shows it.
    #[allow(clippy::manual_dangling_ptr)] // Control IDs are integers cast to pointers.
    pub fn button(&self, id: i32, label: &'static str, glyph: Option<char>) -> Result<Button> {
        unsafe {
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("BUTTON"),
                PCWSTR(wide(label).as_ptr()),
                WS_CHILD | WS_TABSTOP | WINDOW_STYLE(BS_OWNERDRAW as u32),
                0,
                0,
                0,
                0,
                Some(self.hwnd),
                Some(HMENU(id as usize as *mut _)),
                Some(GetModuleHandleW(None)?.into()),
                None,
            )?;
            let _ = SetWindowSubclass(hwnd, Some(button_procedure), 1, 0);
            Ok(Button {
                hwnd,
                id,
                label,
                glyph,
            })
        }
    }
}

pub fn button_state(item: &DRAWITEMSTRUCT) -> ButtonState {
    let flags = item.itemState.0;
    ButtonState {
        hot: unsafe { GetWindowLongPtrW(item.hwndItem, GWLP_USERDATA) } != 0,
        pressed: flags & ODS_SELECTED.0 != 0,
        focused: flags & ODS_FOCUS.0 != 0 && flags & ODS_NOFOCUSRECT.0 == 0,
        disabled: flags & ODS_DISABLED.0 != 0,
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
