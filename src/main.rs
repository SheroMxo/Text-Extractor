#![windows_subsystem = "windows"]

mod capture;
mod config;
mod hotkey_dialog;
mod ocr;
mod overlay;

use std::cell::RefCell;
use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_SINGLETHREADED};
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_NOREPEAT,
};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
    DispatchMessageW, GetCursorPos, GetMessageW, LoadIconW, PostQuitMessage, RegisterClassW,
    SetForegroundWindow, TrackPopupMenu, TranslateMessage, IDI_APPLICATION, MF_CHECKED, MF_POPUP,
    MF_SEPARATOR, MF_STRING, MF_UNCHECKED, MSG, TPM_BOTTOMALIGN, TPM_LEFTALIGN, WM_APP,
    WM_COMMAND, WM_DESTROY, WM_HOTKEY, WM_LBUTTONUP, WM_RBUTTONUP, WNDCLASSW,
};

use config::{AppConfig, HotkeyConfig};

const HOTKEY_ID: i32 = 1;
const WM_TRAYICON: u32 = WM_APP + 1;

const IDM_CAPTURE: usize = 1001;
const IDM_EXIT: usize = 1002;
const IDM_SHORTCUT: usize = 1003;
const IDM_LANG_AUTO: usize = 2000;
const IDM_LANG_BASE: usize = 2001;

struct AppState {
    config: AppConfig,
    available_langs: Vec<(String, String)>,
    mode: ocr::OcrMode,
    overlay_active: bool,
}

thread_local! {
    static APP_STATE: RefCell<Option<AppState>> = const { RefCell::new(None) };
}

unsafe fn register_app_hotkey(hwnd: HWND, hk: &HotkeyConfig) {
    let _ = UnregisterHotKey(hwnd, HOTKEY_ID);
    let mods = HOT_KEY_MODIFIERS(hk.modifiers_flags() | MOD_NOREPEAT.0);
    let _ = RegisterHotKey(hwnd, HOTKEY_ID, mods, hk.vk);
}

unsafe fn load_app_icon(hinstance: HINSTANCE) -> windows::Win32::UI::WindowsAndMessaging::HICON {
    LoadIconW(hinstance, PCWSTR(1 as *const u16))
        .or_else(|_| LoadIconW(None, IDI_APPLICATION))
        .unwrap_or_default()
}

unsafe fn update_tray_tooltip(hwnd: HWND, hinstance: HINSTANCE, hk: &HotkeyConfig, add: bool) {
    let mut nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        uFlags: if add {
            NIF_ICON | NIF_MESSAGE | NIF_TIP
        } else {
            NIF_TIP
        },
        uCallbackMessage: WM_TRAYICON,
        hIcon: if add {
            load_app_icon(hinstance)
        } else {
            Default::default()
        },
        ..Default::default()
    };

    let tip_str = format!("Text Extractor ({})", hk.display());
    let tip: Vec<u16> = tip_str.encode_utf16().collect();
    let copy_len = tip.len().min(nid.szTip.len() - 1);
    nid.szTip[..copy_len].copy_from_slice(&tip[..copy_len]);
    let _ = Shell_NotifyIconW(if add { NIM_ADD } else { NIM_MODIFY }, &nid);
}

unsafe fn open_shortcut_settings(hwnd: HWND) {
    let _ = UnregisterHotKey(hwnd, HOTKEY_ID);
    let current = APP_STATE.with(|cell| {
        cell.borrow()
            .as_ref()
            .map(|s| s.config.hotkey.clone())
            .unwrap_or_default()
    });

    if let Some(new_hk) = hotkey_dialog::show_change_shortcut_dialog(current.clone()) {
        APP_STATE.with(|cell| {
            if let Some(s) = cell.borrow_mut().as_mut() {
                s.config.hotkey = new_hk.clone();
                s.config.save();
            }
        });
        register_app_hotkey(hwnd, &new_hk);
        update_tray_tooltip(hwnd, HINSTANCE::default(), &new_hk, false);
    } else {
        register_app_hotkey(hwnd, &current);
    }
}

fn trigger_capture(hwnd: HWND) {
    let already_active = APP_STATE.with(|cell| {
        if let Some(s) = cell.borrow_mut().as_mut() {
            if s.overlay_active {
                return true;
            }
            s.overlay_active = true;
        }
        false
    });

    if already_active {
        return;
    }

    let (lang, langs, mode) = APP_STATE.with(|cell| {
        if let Some(s) = cell.borrow().as_ref() {
            (s.config.lang.clone(), s.available_langs.clone(), s.mode)
        } else {
            (None, Vec::new(), ocr::OcrMode::Normal)
        }
    });

    let mut wants_shortcut_dlg = false;
    if let Some(res) = overlay::run_capture_overlay(lang, langs, mode) {
        wants_shortcut_dlg = res.open_shortcut_settings;
        APP_STATE.with(|cell| {
            if let Some(s) = cell.borrow_mut().as_mut() {
                s.config.lang = res.selected_lang;
                s.mode = res.mode;
                s.config.save();
            }
        });
    }

    APP_STATE.with(|cell| {
        if let Some(s) = cell.borrow_mut().as_mut() {
            s.overlay_active = false;
        }
    });

    if wants_shortcut_dlg {
        unsafe { open_shortcut_settings(hwnd); }
    }
}

unsafe fn show_tray_menu(hwnd: HWND) {
    let Ok(menu) = CreatePopupMenu() else {
        return;
    };

    let hk_label = APP_STATE.with(|cell| {
        cell.borrow()
            .as_ref()
            .map(|s| format!("Capture Text ({})", s.config.hotkey.display()))
            .unwrap_or_else(|| "Capture Text".to_string())
    });
    let cap_hstr = HSTRING::from(hk_label);
    let _ = AppendMenuW(menu, MF_STRING, IDM_CAPTURE, &cap_hstr);
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);

    if let Ok(lang_menu) = CreatePopupMenu() {
        APP_STATE.with(|cell| {
            if let Some(state) = cell.borrow().as_ref() {
                let auto_flag = if state.config.lang.is_none() {
                    MF_CHECKED
                } else {
                    MF_UNCHECKED
                };
                let _ = AppendMenuW(
                    lang_menu,
                    MF_STRING | auto_flag,
                    IDM_LANG_AUTO,
                    w!("Auto (Windows Profile)"),
                );

                for (idx, (tag, name)) in state.available_langs.iter().enumerate() {
                    let checked = if state.config.lang.as_deref() == Some(tag.as_str()) {
                        MF_CHECKED
                    } else {
                        MF_UNCHECKED
                    };
                    let label = HSTRING::from(format!("{name} ({tag})"));
                    let _ = AppendMenuW(lang_menu, MF_STRING | checked, IDM_LANG_BASE + idx, &label);
                }
            }
        });

        let _ = AppendMenuW(menu, MF_POPUP, lang_menu.0 as usize, w!("OCR Language"));
    }

    let _ = AppendMenuW(menu, MF_STRING, IDM_SHORTCUT, w!("Change Shortcut..."));
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
    let _ = AppendMenuW(menu, MF_STRING, IDM_EXIT, w!("Exit"));

    let mut pt = POINT::default();
    let _ = GetCursorPos(&mut pt);
    let _ = SetForegroundWindow(hwnd);
    let _ = TrackPopupMenu(menu, TPM_LEFTALIGN | TPM_BOTTOMALIGN, pt.x, pt.y, 0, hwnd, None);
    let _ = DestroyMenu(menu);
}

unsafe extern "system" fn main_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_HOTKEY => {
            if wparam.0 as i32 == HOTKEY_ID {
                trigger_capture(hwnd);
            }
            LRESULT(0)
        }
        WM_TRAYICON => {
            let mouse_msg = (lparam.0 & 0xFFFF) as u32;
            if mouse_msg == WM_LBUTTONUP {
                trigger_capture(hwnd);
            } else if mouse_msg == WM_RBUTTONUP {
                show_tray_menu(hwnd);
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            let cmd = wparam.0 & 0xFFFF;
            match cmd {
                IDM_CAPTURE => trigger_capture(hwnd),
                IDM_SHORTCUT => open_shortcut_settings(hwnd),
                IDM_EXIT => {
                    let _ = DestroyWindow(hwnd);
                }
                IDM_LANG_AUTO => {
                    APP_STATE.with(|cell| {
                        if let Some(s) = cell.borrow_mut().as_mut() {
                            s.config.lang = None;
                            s.config.save();
                        }
                    });
                }
                id if id >= IDM_LANG_BASE => {
                    let idx = id - IDM_LANG_BASE;
                    APP_STATE.with(|cell| {
                        if let Some(s) = cell.borrow_mut().as_mut() {
                            if let Some((tag, _)) = s.available_langs.get(idx) {
                                s.config.lang = Some(tag.clone());
                                s.config.save();
                            }
                        }
                    });
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            let _ = UnregisterHotKey(hwnd, HOTKEY_ID);
            let nid = NOTIFYICONDATAW {
                cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
                hWnd: hwnd,
                uID: 1,
                ..Default::default()
            };
            let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn main() {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let _ = RoInitialize(RO_INIT_SINGLETHREADED);

        let _mutex = CreateMutexW(None, true, w!("Local\\TextExtractor_Rust_Mutex"));
        if windows::Win32::Foundation::GetLastError()
            == windows::Win32::Foundation::ERROR_ALREADY_EXISTS
        {
            return;
        }

        let cfg = AppConfig::load();
        let available_langs = ocr::list_available_languages();

        if std::env::args().any(|a| a == "--capture") {
            let _ = overlay::run_capture_overlay(cfg.lang, available_langs, ocr::OcrMode::Normal);
            return;
        }

        let init_hk = cfg.hotkey.clone();
        APP_STATE.with(|cell| {
            *cell.borrow_mut() = Some(AppState {
                config: cfg,
                available_langs,
                mode: ocr::OcrMode::Normal,
                overlay_active: false,
            });
        });

        let Ok(hinstance) = GetModuleHandleW(None) else {
            return;
        };
        let class_name = w!("TextExtractorHostClass");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(main_wnd_proc),
            hInstance: hinstance.into(),
            lpszClassName: class_name,
            ..Default::default()
        };
        RegisterClassW(&wc);

        let Ok(hwnd) = CreateWindowExW(
            Default::default(),
            class_name,
            w!("Text Extractor"),
            Default::default(),
            0,
            0,
            0,
            0,
            None,
            None,
            hinstance,
            None,
        ) else {
            return;
        };

        register_app_hotkey(hwnd, &init_hk);
        update_tray_tooltip(hwnd, hinstance.into(), &init_hk, true);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
