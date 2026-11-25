use std::cell::RefCell;
use windows::core::{w, HSTRING};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint, FillRect,
    InvalidateRect, RoundRect, SelectObject, SetBkMode, SetTextColor, CLEARTYPE_QUALITY,
    CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH, DRAW_TEXT_FORMAT, DT_CENTER,
    DT_SINGLELINE, DT_VCENTER, FF_DONTCARE, FW_BOLD, FW_NORMAL, OUT_DEFAULT_PRECIS, PAINTSTRUCT,
    PS_SOLID, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, VK_CONTROL, VK_ESCAPE, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MENU,
    VK_RCONTROL, VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, GetSystemMetrics,
    LoadCursorW, PostQuitMessage, RegisterClassW, SetForegroundWindow, ShowWindow,
    TranslateMessage, CS_HREDRAW, CS_VREDRAW, IDC_ARROW, MSG, SM_CXSCREEN, SM_CYSCREEN, SW_SHOW,
    WM_DESTROY, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN, WM_MOUSEMOVE, WM_PAINT, WM_SYSKEYDOWN,
    WNDCLASSW, WS_CAPTION, WS_EX_TOPMOST, WS_SYSMENU,
};

use crate::config::HotkeyConfig;

const DLG_W: i32 = 380;
const DLG_H: i32 = 210;

const BTN_SAVE: RECT = RECT { left: 268, top: 128, right: 356, bottom: 160 };
const BTN_RESET: RECT = RECT { left: 148, top: 128, right: 256, bottom: 160 };
const BTN_CANCEL: RECT = RECT { left: 24, top: 128, right: 112, bottom: 160 };
const BOX_RECT: RECT = RECT { left: 24, top: 58, right: 356, bottom: 106 };

struct DialogState {
    pending: HotkeyConfig,
    confirmed: Option<HotkeyConfig>,
    hovered_btn: i32,
}

thread_local! {
    static DLG_STATE: RefCell<Option<DialogState>> = const { RefCell::new(None) };
    static DLG_CLASS_REG: RefCell<bool> = const { RefCell::new(false) };
}

fn pt_in(r: &RECT, x: i32, y: i32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

fn is_modifier_vk(vk: u32) -> bool {
    matches!(
        vk as u16,
        x if x == VK_SHIFT.0
            || x == VK_LSHIFT.0
            || x == VK_RSHIFT.0
            || x == VK_CONTROL.0
            || x == VK_LCONTROL.0
            || x == VK_RCONTROL.0
            || x == VK_MENU.0
            || x == VK_LMENU.0
            || x == VK_RMENU.0
            || x == VK_LWIN.0
            || x == VK_RWIN.0
    )
}

unsafe fn is_down(vk: u16) -> bool {
    (GetAsyncKeyState(vk as i32) as u16 & 0x8000) != 0
}

unsafe extern "system" fn dlg_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_ERASEBKGND => LRESULT(1),
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            let vk = wparam.0 as u32;
            if vk == VK_ESCAPE.0 as u32 {
                let _ = DestroyWindow(hwnd);
                return LRESULT(0);
            }
            if !is_modifier_vk(vk) {
                let win = is_down(VK_LWIN.0) || is_down(VK_RWIN.0);
                let ctrl = is_down(VK_CONTROL.0);
                let alt = is_down(VK_MENU.0);
                let shift = is_down(VK_SHIFT.0);
                if win || ctrl || alt || shift {
                    DLG_STATE.with(|cell| {
                        if let Some(s) = cell.borrow_mut().as_mut() {
                            s.pending = HotkeyConfig {
                                win,
                                ctrl,
                                alt,
                                shift,
                                vk,
                            };
                        }
                    });
                    let _ = InvalidateRect(hwnd, None, false);
                }
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let x = (lparam.0 & 0xFFFF) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
            let new_hover = if pt_in(&BTN_SAVE, x, y) {
                1
            } else if pt_in(&BTN_RESET, x, y) {
                2
            } else if pt_in(&BTN_CANCEL, x, y) {
                3
            } else {
                0
            };
            let changed = DLG_STATE.with(|cell| {
                if let Some(s) = cell.borrow_mut().as_mut() {
                    if s.hovered_btn != new_hover {
                        s.hovered_btn = new_hover;
                        return true;
                    }
                }
                false
            });
            if changed {
                let _ = InvalidateRect(hwnd, None, false);
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let x = (lparam.0 & 0xFFFF) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
            if pt_in(&BTN_SAVE, x, y) {
                DLG_STATE.with(|cell| {
                    if let Some(s) = cell.borrow_mut().as_mut() {
                        s.confirmed = Some(s.pending.clone());
                    }
                });
                let _ = DestroyWindow(hwnd);
            } else if pt_in(&BTN_RESET, x, y) {
                DLG_STATE.with(|cell| {
                    if let Some(s) = cell.borrow_mut().as_mut() {
                        s.pending = HotkeyConfig::default();
                    }
                });
                let _ = InvalidateRect(hwnd, None, false);
            } else if pt_in(&BTN_CANCEL, x, y) {
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);

            let bg_brush = CreateSolidBrush(COLORREF(0x00202020));
            let client = RECT { left: 0, top: 0, right: DLG_W, bottom: DLG_H };
            FillRect(hdc, &client, bg_brush);
            let _ = DeleteObject(bg_brush);

            let font = CreateFontW(
                -13,
                0,
                0,
                0,
                FW_NORMAL.0 as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET.0 as u32,
                OUT_DEFAULT_PRECIS.0 as u32,
                CLIP_DEFAULT_PRECIS.0 as u32,
                CLEARTYPE_QUALITY.0 as u32,
                (DEFAULT_PITCH.0 | FF_DONTCARE.0) as u32,
                w!("Segoe UI"),
            );
            let bold_font = CreateFontW(
                -16,
                0,
                0,
                0,
                FW_BOLD.0 as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET.0 as u32,
                OUT_DEFAULT_PRECIS.0 as u32,
                CLIP_DEFAULT_PRECIS.0 as u32,
                CLEARTYPE_QUALITY.0 as u32,
                (DEFAULT_PITCH.0 | FF_DONTCARE.0) as u32,
                w!("Segoe UI"),
            );

            SetBkMode(hdc, TRANSPARENT);
            let old_f = SelectObject(hdc, font);
            SetTextColor(hdc, COLORREF(0x00CCCCCC));
            let mut title_r = RECT { left: 24, top: 18, right: 356, bottom: 44 };
            let mut prompt: Vec<u16> = "Press a new shortcut (e.g. Ctrl + Shift + X):"
                .encode_utf16()
                .collect();
            DrawTextW(hdc, &mut prompt, &mut title_r, DRAW_TEXT_FORMAT(DT_SINGLELINE.0));

            // Shortcut box
            let box_br = CreateSolidBrush(COLORREF(0x002B2B2B));
            let box_pn = windows::Win32::Graphics::Gdi::CreatePen(
                PS_SOLID,
                1,
                COLORREF(0x00FFAA00),
            );
            let ob = SelectObject(hdc, box_br);
            let op = SelectObject(hdc, box_pn);
            let _ = RoundRect(hdc, BOX_RECT.left, BOX_RECT.top, BOX_RECT.right, BOX_RECT.bottom, 10, 10);
            SelectObject(hdc, ob);
            SelectObject(hdc, op);
            let _ = DeleteObject(box_br);
            let _ = DeleteObject(box_pn);

            let shortcut_text = DLG_STATE.with(|cell| {
                cell.borrow()
                    .as_ref()
                    .map(|s| s.pending.display())
                    .unwrap_or_default()
            });
            SelectObject(hdc, bold_font);
            SetTextColor(hdc, COLORREF(0x00FFFFFF));
            let mut box_r = BOX_RECT;
            let mut sc_wide: Vec<u16> = shortcut_text.encode_utf16().collect();
            DrawTextW(
                hdc,
                &mut sc_wide,
                &mut box_r,
                DRAW_TEXT_FORMAT(DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0),
            );

            // Buttons
            SelectObject(hdc, font);
            let hovered = DLG_STATE.with(|c| c.borrow().as_ref().map(|s| s.hovered_btn).unwrap_or(0));

            let draw_btn = |r: RECT, label: &str, primary: bool, is_hover: bool| {
                let fill = if primary {
                    if is_hover { COLORREF(0x00E09000) } else { COLORREF(0x00D47800) }
                } else if is_hover {
                    COLORREF(0x003D3D3D)
                } else {
                    COLORREF(0x002E2E2E)
                };
                let br = CreateSolidBrush(fill);
                let pn = windows::Win32::Graphics::Gdi::CreatePen(
                    PS_SOLID,
                    1,
                    COLORREF(0x004A4A4A),
                );
                let ob = SelectObject(hdc, br);
                let op = SelectObject(hdc, pn);
                let _ = RoundRect(hdc, r.left, r.top, r.right, r.bottom, 8, 8);
                SelectObject(hdc, ob);
                SelectObject(hdc, op);
                let _ = DeleteObject(br);
                let _ = DeleteObject(pn);

                SetTextColor(hdc, COLORREF(0x00FFFFFF));
                let mut tr = r;
                let mut w: Vec<u16> = HSTRING::from(label).as_wide().to_vec();
                DrawTextW(
                    hdc,
                    &mut w,
                    &mut tr,
                    DRAW_TEXT_FORMAT(DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0),
                );
            };

            draw_btn(BTN_CANCEL, "Cancel", false, hovered == 3);
            draw_btn(BTN_RESET, "Reset Default", false, hovered == 2);
            draw_btn(BTN_SAVE, "Save", true, hovered == 1);

            SelectObject(hdc, old_f);
            let _ = DeleteObject(font);
            let _ = DeleteObject(bold_font);
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Shows the "Change Shortcut" modal dialog and returns `Some(new_hotkey)` if saved.
pub fn show_change_shortcut_dialog(current: HotkeyConfig) -> Option<HotkeyConfig> {
    DLG_STATE.with(|cell| {
        *cell.borrow_mut() = Some(DialogState {
            pending: current,
            confirmed: None,
            hovered_btn: 0,
        });
    });

    unsafe {
        let hinstance = GetModuleHandleW(None).ok()?;
        let class_name = w!("TextExtractorShortcutDlg");

        DLG_CLASS_REG.with(|reg| {
            if !*reg.borrow() {
                let wc = WNDCLASSW {
                    style: CS_HREDRAW | CS_VREDRAW,
                    lpfnWndProc: Some(dlg_wnd_proc),
                    hInstance: hinstance.into(),
                    hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                    lpszClassName: class_name,
                    ..Default::default()
                };
                if RegisterClassW(&wc) != 0 {
                    *reg.borrow_mut() = true;
                }
            }
        });

        let sx = (GetSystemMetrics(SM_CXSCREEN) - DLG_W) / 2;
        let sy = (GetSystemMetrics(SM_CYSCREEN) - DLG_H) / 2;

        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST,
            class_name,
            w!("Text Extractor - Change Shortcut"),
            WS_CAPTION | WS_SYSMENU,
            sx,
            sy,
            DLG_W,
            DLG_H,
            None,
            None,
            hinstance,
            None,
        )
        .ok()?;

        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    DLG_STATE.with(|cell| cell.borrow_mut().take().and_then(|s| s.confirmed))
}
