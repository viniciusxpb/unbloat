use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};
use unbloat::{Result, archiver, config::Config, error, scanner};
use windows::{
    Win32::{
        Foundation::*,
        Graphics::Gdi::COLOR_WINDOW,
        System::LibraryLoader::GetModuleHandleW,
        UI::{Input::KeyboardAndMouse::EnableWindow, WindowsAndMessaging::*},
    },
    core::{PCWSTR, w},
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
            w!("Unbloat"),
            MB_OK | MB_ICONERROR,
        );
    }
}
struct State {
    cancel: Arc<AtomicBool>,
    status: Arc<Mutex<String>>,
    result: mpsc::Receiver<Result<PathBuf>>,
    finished: Option<Result<PathBuf>>,
    label: HWND,
    button: HWND,
    ticks_after_success: u32,
}
unsafe extern "system" fn procedure(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let pointer = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut State;
    if !pointer.is_null() {
        let state = unsafe { &mut *pointer };
        match message {
            WM_CLOSE | WM_COMMAND => {
                if message == WM_COMMAND && wparam.0 & 0xffff != 1 {
                    return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
                }
                state.cancel.store(true, Ordering::Relaxed);
                unsafe {
                    let _ = SetWindowTextW(state.label, w!("Cancelando com segurança…"));
                    let _ = EnableWindow(state.button, false);
                }
                if state.finished.is_some() {
                    unsafe {
                        let _ = DestroyWindow(hwnd);
                    }
                }
                return LRESULT(0);
            }
            WM_TIMER => {
                if state.finished.is_none() {
                    let text = state
                        .status
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .clone();
                    if !state.cancel.load(Ordering::Relaxed) {
                        unsafe {
                            let _ = SetWindowTextW(state.label, PCWSTR(wide(&text).as_ptr()));
                        }
                    }
                    match state.result.try_recv() {
                        Ok(result) => state.finished = Some(result),
                        Err(mpsc::TryRecvError::Disconnected) => {
                            state.finished =
                                Some(Err(error("O processo de compactação foi interrompido.")))
                        }
                        Err(mpsc::TryRecvError::Empty) => {}
                    }
                    if let Some(result) = &state.finished {
                        if state.cancel.load(Ordering::Relaxed) || result.is_err() {
                            unsafe {
                                let _ = DestroyWindow(hwnd);
                            }
                        } else if let Ok(path) = result {
                            let text = wide(&format!("Concluído: {}", path.display()));
                            unsafe {
                                let _ = SetWindowTextW(state.label, PCWSTR(text.as_ptr()));
                                let _ = SetWindowTextW(state.button, w!("Fechar"));
                            }
                        }
                    }
                } else {
                    state.ticks_after_success += 1;
                    if state.ticks_after_success >= 10 {
                        unsafe {
                            let _ = DestroyWindow(hwnd);
                        }
                    }
                }
                return LRESULT(0);
            }
            WM_DESTROY => {
                unsafe {
                    PostQuitMessage(0);
                }
                return LRESULT(0);
            }
            _ => {}
        }
    }
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

#[allow(clippy::manual_dangling_ptr)] // Win32 MAKEINTRESOURCE/menu IDs are integers cast to pointers.
pub fn run(paths: Vec<PathBuf>, config: Config) -> Result<()> {
    let cancel = Arc::new(AtomicBool::new(false));
    let status = Arc::new(Mutex::new(String::from("Analisando arquivos…")));
    let (tx, rx) = mpsc::channel();
    let mut state = Box::new(State {
        cancel: cancel.clone(),
        status: status.clone(),
        result: rx,
        finished: None,
        label: HWND::default(),
        button: HWND::default(),
        ticks_after_success: 0,
    });
    let hwnd = unsafe {
        let instance = GetModuleHandleW(None)?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(procedure),
            hInstance: instance.into(),
            lpszClassName: w!("UnbloatProgress"),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hIcon: LoadIconW(Some(instance.into()), PCWSTR(1usize as *const u16))
                .unwrap_or_default(),
            hbrBackground: windows::Win32::Graphics::Gdi::HBRUSH((COLOR_WINDOW.0 + 1) as *mut _),
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            return Err(windows::core::Error::from_thread().into());
        }
        let hwnd = CreateWindowExW(
            WS_EX_APPWINDOW,
            class.lpszClassName,
            w!("Unbloat — Compactando projetos"),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            580,
            180,
            None,
            None,
            Some(instance.into()),
            None,
        )?;
        state.label = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("STATIC"),
            w!("Analisando arquivos…"),
            WS_CHILD | WS_VISIBLE,
            20,
            20,
            525,
            55,
            Some(hwnd),
            None,
            Some(instance.into()),
            None,
        )?;
        state.button = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("BUTTON"),
            w!("Cancelar"),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP,
            435,
            90,
            110,
            28,
            Some(hwnd),
            Some(HMENU(1usize as *mut _)),
            Some(instance.into()),
            None,
        )?;
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, (&mut *state as *mut State) as isize);
        if SetTimer(Some(hwnd), 1, 200, None) == 0 {
            let _ = DestroyWindow(hwnd);
            return Err(windows::core::Error::from_thread().into());
        }
        let _ = ShowWindow(hwnd, SW_SHOW);
        hwnd
    };
    let worker_cancel = cancel.clone();
    let worker = thread::spawn(move || {
        let result = (|| {
            let manifest = scanner::scan(&paths, &config.ignore, &worker_cancel)?;
            archiver::create(&manifest, &config, &worker_cancel, &|text| {
                *status.lock().unwrap_or_else(|e| e.into_inner()) = text.to_owned();
                if config.logging.enabled {
                    eprintln!("{text}");
                }
            })
        })();
        let _ = tx.send(result);
    });
    unsafe {
        let mut message = MSG::default();
        loop {
            let result = GetMessageW(&mut message, None, 0, 0).0;
            if result <= 0 {
                break;
            }
            if !IsDialogMessageW(hwnd, &message).as_bool() {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        let _ = KillTimer(Some(hwnd), 1);
    }
    let was_cancelled = cancel.swap(true, Ordering::Relaxed);
    worker
        .join()
        .map_err(|_| error("Falha inesperada no motor."))?;
    match state.finished.take() {
        Some(Ok(_)) => Ok(()),
        Some(Err(_)) if was_cancelled => Ok(()),
        Some(Err(e)) => Err(e),
        None => Err(error("Operação encerrada.")),
    }
}
