//! Native first-launch/install window. Opening the executable must explain its use.
use std::{
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::Command,
    sync::mpsc::{self, Receiver},
    thread,
};
use unbloat::{Result, error};
use windows::{
    Win32::{
        Foundation::*,
        Graphics::Gdi::{COLOR_BTNFACE, DEFAULT_GUI_FONT, GetStockObject, HBRUSH},
        System::{
            Console::{
                ATTACH_PARENT_PROCESS, AttachConsole, FreeConsole, GetStdHandle, STD_OUTPUT_HANDLE,
            },
            LibraryLoader::GetModuleHandleW,
        },
        UI::{Input::KeyboardAndMouse::EnableWindow, WindowsAndMessaging::*},
    },
    core::{PCWSTR, w},
};

const INSTALL: usize = 10;
const CLOSE: usize = 20;
const DESCRIPTION: usize = 30;
const USAGE: &str = "Selecione arquivos e/ou pastas no mesmo diretório e clique com o botão direito em 'Compactar com Unbloat'.\r\n\r\nNo Windows 11, abra 'Mostrar mais opções'. O arquivo .7z será salvo ao lado dos itens selecionados.";

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

/// GUI release builds still support CLI output and redirected stdout pipes.
pub fn attach_parent_console() {
    unsafe {
        if !GetStdHandle(STD_OUTPUT_HANDLE).is_ok_and(|handle| !handle.is_invalid()) {
            let _ = AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
}

fn install(script: &Path) -> Result<()> {
    // Pass the path as data rather than interpolating it into PowerShell code.
    // The packaged installer needs only Windows PowerShell, never Cargo/Rust.
    let powershell = PathBuf::from(
        std::env::var_os("SystemRoot")
            .ok_or_else(|| error("Diretório do Windows não encontrado (SystemRoot)."))?,
    )
    .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let output = Command::new(powershell)
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command",
            "$ErrorActionPreference = 'Stop'; [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false); & $env:UNBLOAT_INSTALL_SCRIPT"])
        .env("UNBLOAT_INSTALL_SCRIPT", script)
        .current_dir(script.parent().ok_or_else(|| error("Pasta do instalador inválida."))?)
        .creation_flags(0x08000000) // CREATE_NO_WINDOW; the native window reports progress/errors.
        .output()?;
    if !output.status.success() {
        let details = format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return Err(error(format!(
            "Não foi possível instalar a integração.\n\n{}",
            details.trim().chars().take(3000).collect::<String>()
        )));
    }
    Ok(())
}

struct State {
    installer: PathBuf,
    label: HWND,
    install_button: HWND,
    close_button: HWND,
    pending: Option<Receiver<Result<()>>>,
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
        WM_COMMAND if wparam.0 & 0xffff == INSTALL => {
            if state.pending.is_some() {
                return LRESULT(0);
            }
            let script = state.installer.clone();
            let (sender, receiver) = mpsc::channel();
            state.pending = Some(receiver);
            unsafe {
                let _ = EnableWindow(state.install_button, false);
                let _ = EnableWindow(state.close_button, false);
                let _ = SetWindowTextW(
                    state.label,
                    w!(
                        "Instalando a integração do botão direito…\r\n\r\nAguarde a conclusão. Esta janela continuará aberta para mostrar o resultado."
                    ),
                );
            }
            thread::spawn(move || {
                let _ = sender.send(install(&script));
            });
            LRESULT(0)
        }
        WM_CLOSE | WM_COMMAND if message == WM_CLOSE || wparam.0 & 0xffff == CLOSE => {
            if state.pending.is_none() {
                unsafe {
                    let _ = DestroyWindow(hwnd);
                }
            }
            LRESULT(0)
        }
        WM_TIMER => {
            let result = state
                .pending
                .as_ref()
                .and_then(|receiver| match receiver.try_recv() {
                    Ok(result) => Some(result),
                    Err(mpsc::TryRecvError::Empty) => None,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        Some(Err(error("O instalador foi interrompido.")))
                    }
                });
            if let Some(result) = result {
                state.pending = None;
                unsafe {
                    let _ = EnableWindow(state.close_button, true);
                }
                match result {
                    Ok(()) => {
                        let text = wide(&format!("Integração instalada!\r\n\r\n{USAGE}"));
                        unsafe {
                            let _ = SetWindowTextW(state.label, PCWSTR(text.as_ptr()));
                        }
                    }
                    Err(error) => {
                        unsafe {
                            let _ = EnableWindow(state.install_button, true);
                            let _ = SetWindowTextW(
                                state.label,
                                w!(
                                    "A instalação não foi concluída.\r\n\r\nConfira a mensagem de erro e tente novamente."
                                ),
                            );
                        }
                        crate::ui::show_error(&error.to_string());
                    }
                }
            }
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
        let _ = FreeConsole();
    } // Debug builds may have inherited/created a console.
    let directory = std::env::current_exe()?
        .parent()
        .ok_or_else(|| error("Pasta da aplicação inválida."))?
        .to_path_buf();
    let installer = directory.join("install.ps1");
    let installed = directory.join(".unbloat-install.json").is_file();
    let text = if installed {
        format!("Unbloat já está instalado.\r\n\r\n{USAGE}")
    } else if installer.is_file() {
        String::from(
            "Ship the code. Not the node_modules.\r\n\r\nClique em 'Instalar integração' para adicionar 'Compactar com Unbloat' ao botão direito de arquivos e pastas.\r\n\r\nNo Windows 11, a opção fica em 'Mostrar mais opções'.",
        )
    } else {
        format!(
            "Abra o unbloat.exe dentro da pasta completa do pacote, junto de install.ps1, da DLL e do config.toml.\r\n\r\n{USAGE}"
        )
    };
    let can_install = installer.is_file() && !installed;
    let mut state = Box::new(State {
        installer,
        label: HWND::default(),
        install_button: HWND::default(),
        close_button: HWND::default(),
        pending: None,
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
            hbrBackground: HBRUSH((COLOR_BTNFACE.0 + 1) as *mut _),
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            return Err(windows::core::Error::from_thread().into());
        }
        let hwnd = CreateWindowExW(
            WS_EX_APPWINDOW,
            class.lpszClassName,
            w!("Unbloat — Instalação e uso"),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            640,
            290,
            None,
            None,
            Some(instance.into()),
            None,
        )?;
        state.label = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("STATIC"),
            PCWSTR(wide(&text).as_ptr()),
            WS_CHILD | WS_VISIBLE,
            24,
            24,
            570,
            145,
            Some(hwnd),
            Some(HMENU(DESCRIPTION as *mut _)),
            Some(instance.into()),
            None,
        )?;
        state.install_button = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("BUTTON"),
            w!("Instalar integração"),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP,
            24,
            190,
            210,
            32,
            Some(hwnd),
            Some(HMENU(INSTALL as *mut _)),
            Some(instance.into()),
            None,
        )?;
        state.close_button = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("BUTTON"),
            w!("Fechar"),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP,
            484,
            190,
            110,
            32,
            Some(hwnd),
            Some(HMENU(CLOSE as *mut _)),
            Some(instance.into()),
            None,
        )?;
        let font = GetStockObject(DEFAULT_GUI_FONT);
        for control in [state.label, state.install_button, state.close_button] {
            SendMessageW(
                control,
                WM_SETFONT,
                Some(WPARAM(font.0 as usize)),
                Some(LPARAM(1)),
            );
        }
        let _ = EnableWindow(state.install_button, can_install);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, (&mut *state as *mut State) as isize);
        if SetTimer(Some(hwnd), 1, 200, None) == 0 {
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
        let _ = KillTimer(Some(hwnd), 1);
    }
    Ok(())
}
