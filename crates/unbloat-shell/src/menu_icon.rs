//! Explorer expects an HBITMAP for the icon beside a context-menu label.
//! Render our embedded icon to a 32-bit DIB and keep it alive with its handler.
use windows::{
    Win32::{
        Foundation::{E_FAIL, HMODULE},
        Graphics::Gdi::*,
        UI::WindowsAndMessaging::*,
    },
    core::{Error, PCWSTR, Result},
};

struct Icon(HICON);
impl Drop for Icon {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyIcon(self.0);
        }
    }
}
struct MemoryDc(HDC);
impl Drop for MemoryDc {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteDC(self.0);
        }
    }
}
pub struct MenuIcon(HBITMAP);
impl Drop for MenuIcon {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(HGDIOBJ(self.0.0));
        }
    }
}
impl MenuIcon {
    #[allow(clippy::manual_dangling_ptr)] // MAKEINTRESOURCE: icon resource ID 1.
    pub fn load(module: HMODULE) -> Result<Self> {
        unsafe {
            let width = GetSystemMetrics(SM_CXSMICON).max(16);
            let height = GetSystemMetrics(SM_CYSMICON).max(16);
            let icon = Icon(HICON(
                LoadImageW(
                    Some(module.into()),
                    PCWSTR(1usize as *const u16),
                    IMAGE_ICON,
                    width,
                    height,
                    LR_DEFAULTCOLOR,
                )?
                .0,
            ));
            let dc = MemoryDc(CreateCompatibleDC(None));
            if dc.0.is_invalid() {
                return Err(Error::from_thread());
            }
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width,
                    biHeight: -height,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut pixels = std::ptr::null_mut();
            let bitmap = Self(CreateDIBSection(
                None,
                &info,
                DIB_RGB_COLORS,
                &mut pixels,
                None,
                0,
            )?);
            if pixels.is_null() {
                return Err(Error::from(E_FAIL));
            }
            std::ptr::write_bytes(pixels.cast::<u8>(), 0, (width * height * 4) as usize);
            let previous = SelectObject(dc.0, HGDIOBJ(bitmap.0.0));
            if previous.is_invalid() {
                return Err(Error::from_thread());
            }
            let drawn = DrawIconEx(dc.0, 0, 0, icon.0, width, height, 0, None, DI_NORMAL);
            SelectObject(dc.0, previous);
            drawn?;
            let _ = GdiFlush();
            Ok(bitmap)
        }
    }

    pub fn bitmap(&self) -> HBITMAP {
        self.0
    }
}
