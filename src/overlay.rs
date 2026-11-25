use std::cell::RefCell;
use windows::core::{w, HSTRING};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateBitmap, CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW,
    CreatePen, CreateSolidBrush, DeleteDC, DeleteObject, DrawTextW, EndPaint, FrameRect,
    InvalidateRect, LineTo, MoveToEx, RoundRect, ScreenToClient, SelectObject, SetBkMode,
    SetTextColor, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH,
    DRAW_TEXT_FORMAT, DT_END_ELLIPSIS, DT_LEFT, DT_SINGLELINE, DT_VCENTER, FF_DONTCARE, FW_NORMAL,
    HBITMAP, HDC, HGDIOBJ, OUT_DEFAULT_PRECIS, PAINTSTRUCT, PS_SOLID, SRCCOPY, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, ReleaseCapture, SetCapture, VK_ESCAPE, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconIndirect, CreatePopupMenu, CreateWindowExW, DefWindowProcW,
    DestroyIcon, DestroyMenu, DestroyWindow, DispatchMessageW, GetCursorPos, GetMessageW,
    GetSystemMetrics, LoadCursorW, PostQuitMessage, RegisterClassW, SetCursor, SetForegroundWindow,
    ShowWindow, TrackPopupMenu, TranslateMessage, CS_HREDRAW, CS_VREDRAW, HCURSOR, HICON,
    ICONINFO, IDC_ARROW, IDC_CROSS, MF_CHECKED, MF_SEPARATOR, MF_STRING, MF_UNCHECKED, MSG,
    SM_CXSCREEN, SW_HIDE, SW_SHOW, TPM_LEFTALIGN, TPM_RETURNCMD, TPM_TOPALIGN, WM_DESTROY,
    WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT,
    WM_RBUTTONDOWN, WM_SETCURSOR, WNDCLASSW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use crate::capture::ScreenCapture;
use crate::ocr::{self, OcrMode};

const TOOLBAR_H: i32 = 46;
const TOOLBAR_PAD: i32 = 6;
const LANG_BTN_W: i32 = 215;
const ICON_BTN_W: i32 = 34;
const BTN_H: i32 = 34;
const BTN_GAP: i32 = 4;
const TOOLBAR_W: i32 = TOOLBAR_PAD * 2 + LANG_BTN_W + BTN_GAP + ICON_BTN_W * 4 + BTN_GAP * 3;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ToolbarHit {
    None,
    LangDropdown,
    SingleLine,
    Table,
    Settings,
    Close,
    BarBackground,
}

pub struct OverlayResult {
    #[allow(dead_code)]
    pub extracted_text: Option<String>,
    pub selected_lang: Option<String>,
    pub mode: OcrMode,
    pub open_shortcut_settings: bool,
}

struct OverlayState {
    capture: ScreenCapture,
    lang_tag: Option<String>,
    lang_display: String,
    available_langs: Vec<(String, String)>,
    mode: OcrMode,
    toolbar_rect: RECT,
    hovered: ToolbarHit,
    dragging: bool,
    start: POINT,
    current: POINT,
    extracted_text: Option<String>,
    open_shortcut_settings: bool,
    back_dc: HDC,
    back_bmp: HBITMAP,
    old_back_bmp: HGDIOBJ,
    cross_cursor: HCURSOR,
    arrow_cursor: HCURSOR,
    custom_cross_owned: bool,
}

impl Drop for OverlayState {
    fn drop(&mut self) {
        unsafe {
            if !self.back_dc.is_invalid() {
                if !self.old_back_bmp.is_invalid() {
                    SelectObject(self.back_dc, self.old_back_bmp);
                }
                if !self.back_bmp.is_invalid() {
                    let _ = DeleteObject(self.back_bmp);
                }
                let _ = DeleteDC(self.back_dc);
            }
            if self.custom_cross_owned && !self.cross_cursor.is_invalid() {
                let _ = DestroyIcon(HICON(self.cross_cursor.0));
            }
        }
    }
}

/// Builds a crisp 32x32 white-with-black-outline crosshair cursor so it never
/// disappears on dark or hardware-accelerated backgrounds.
unsafe fn create_crisp_crosshair_cursor() -> (HCURSOR, bool) {
    const SIZE: usize = 32;
    const CENTER: isize = 15;
    let mut and_mask = [0xFFu8; (SIZE * SIZE) / 8];
    let mut xor_mask = [0x00u8; (SIZE * SIZE) / 8];

    let set_pixel = |x: isize, y: isize, white: bool, and_m: &mut [u8], xor_m: &mut [u8]| {
        if x < 0 || y < 0 || x >= SIZE as isize || y >= SIZE as isize {
            return;
        }
        let idx = (y as usize) * (SIZE / 8) + ((x as usize) / 8);
        let bit = 0x80u8 >> ((x as usize) % 8);
        and_m[idx] &= !bit; // opaque
        if white {
            xor_m[idx] |= bit;
        } else {
            xor_m[idx] &= !bit;
        }
    };

    // Draw black 3px-wide outline arms, then white 1px-wide inner arms
    for d in -10isize..=10 {
        if d.abs() < 2 {
            continue;
        }
        // Horizontal black outline
        set_pixel(CENTER + d, CENTER - 1, false, &mut and_mask, &mut xor_mask);
        set_pixel(CENTER + d, CENTER + 1, false, &mut and_mask, &mut xor_mask);
        // Vertical black outline
        set_pixel(CENTER - 1, CENTER + d, false, &mut and_mask, &mut xor_mask);
        set_pixel(CENTER + 1, CENTER + d, false, &mut and_mask, &mut xor_mask);
    }
    // End caps
    for &end in &[-11isize, 11isize] {
        for off in -1isize..=1 {
            set_pixel(CENTER + end, CENTER + off, false, &mut and_mask, &mut xor_mask);
            set_pixel(CENTER + off, CENTER + end, false, &mut and_mask, &mut xor_mask);
        }
    }
    // White inner crosshair
    for d in -10isize..=10 {
        if d.abs() < 2 {
            continue;
        }
        set_pixel(CENTER + d, CENTER, true, &mut and_mask, &mut xor_mask);
        set_pixel(CENTER, CENTER + d, true, &mut and_mask, &mut xor_mask);
    }
    // Center dot
    set_pixel(CENTER, CENTER, true, &mut and_mask, &mut xor_mask);

    let info = ICONINFO {
        fIcon: false.into(),
        xHotspot: CENTER as u32,
        yHotspot: CENTER as u32,
        hbmMask: CreateBitmap(SIZE as i32, SIZE as i32, 1, 1, Some(and_mask.as_ptr() as *const _)),
        hbmColor: HBITMAP::default(),
    };

    // Use 1bpp monochrome mask+xor by setting hbmMask to double-height (AND + XOR)
    let _ = DeleteObject(info.hbmMask);
    let mut combined = [0u8; ((SIZE * SIZE) / 8) * 2];
    combined[..128].copy_from_slice(&and_mask);
    combined[128..].copy_from_slice(&xor_mask);
    let mono_bmp = windows::Win32::Graphics::Gdi::CreateBitmap(
        SIZE as i32,
        (SIZE * 2) as i32,
        1,
        1,
        Some(combined.as_ptr() as *const _),
    );
    let info2 = ICONINFO {
        fIcon: false.into(),
        xHotspot: CENTER as u32,
        yHotspot: CENTER as u32,
        hbmMask: mono_bmp,
        hbmColor: HBITMAP::default(),
    };
    let res = CreateIconIndirect(&info2);
    let _ = DeleteObject(mono_bmp);

    match res {
        Ok(icon) => (HCURSOR(icon.0), true),
        Err(_) => (LoadCursorW(None, IDC_CROSS).unwrap_or_default(), false),
    }
}

thread_local! {
    static OVERLAY_STATE: RefCell<Option<OverlayState>> = const { RefCell::new(None) };
    static CLASS_REGISTERED: RefCell<bool> = const { RefCell::new(false) };
}

fn normalized_rect(a: POINT, b: POINT) -> (i32, i32, i32, i32) {
    let left = a.x.min(b.x);
    let top = a.y.min(b.y);
    let right = a.x.max(b.x);
    let bottom = a.y.max(b.y);
    (left, top, right - left, bottom - top)
}

fn pt_in_rect(r: &RECT, x: i32, y: i32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

fn button_rects(tb: &RECT) -> (RECT, RECT, RECT, RECT, RECT) {
    let top = tb.top + TOOLBAR_PAD;
    let bottom = top + BTN_H;
    let mut x = tb.left + TOOLBAR_PAD;

    let lang = RECT {
        left: x,
        top,
        right: x + LANG_BTN_W,
        bottom,
    };
    x += LANG_BTN_W + BTN_GAP;

    let single = RECT {
        left: x,
        top,
        right: x + ICON_BTN_W,
        bottom,
    };
    x += ICON_BTN_W + BTN_GAP;

    let table = RECT {
        left: x,
        top,
        right: x + ICON_BTN_W,
        bottom,
    };
    x += ICON_BTN_W + BTN_GAP;

    let settings = RECT {
        left: x,
        top,
        right: x + ICON_BTN_W,
        bottom,
    };
    x += ICON_BTN_W + BTN_GAP;

    let close = RECT {
        left: x,
        top,
        right: x + ICON_BTN_W,
        bottom,
    };

    (lang, single, table, settings, close)
}

fn hit_test_toolbar(tb: &RECT, x: i32, y: i32) -> ToolbarHit {
    if !pt_in_rect(tb, x, y) {
        return ToolbarHit::None;
    }
    let (lang, single, table, settings, close) = button_rects(tb);
    if pt_in_rect(&lang, x, y) {
        ToolbarHit::LangDropdown
    } else if pt_in_rect(&single, x, y) {
        ToolbarHit::SingleLine
    } else if pt_in_rect(&table, x, y) {
        ToolbarHit::Table
    } else if pt_in_rect(&settings, x, y) {
        ToolbarHit::Settings
    } else if pt_in_rect(&close, x, y) {
        ToolbarHit::Close
    } else {
        ToolbarHit::BarBackground
    }
}

unsafe fn draw_toolbar(hdc: windows::Win32::Graphics::Gdi::HDC, state: &OverlayState) {
    let tb = &state.toolbar_rect;

    // Outer dark rounded pill (#222222 fill, #3D3D3D border)
    let bg_brush = CreateSolidBrush(COLORREF(0x00222222));
    let border_pen = CreatePen(PS_SOLID, 1, COLORREF(0x003D3D3D));
    let old_brush = SelectObject(hdc, bg_brush);
    let old_pen = SelectObject(hdc, border_pen);
    let _ = RoundRect(hdc, tb.left, tb.top, tb.right, tb.bottom, 16, 16);
    SelectObject(hdc, old_brush);
    SelectObject(hdc, old_pen);
    let _ = DeleteObject(bg_brush);
    let _ = DeleteObject(border_pen);

    let (lang_r, single_r, table_r, settings_r, close_r) = button_rects(tb);

    // 1. Language Dropdown Pill
    let lang_fill = if state.hovered == ToolbarHit::LangDropdown {
        COLORREF(0x00383838)
    } else {
        COLORREF(0x002C2C2C)
    };
    let l_brush = CreateSolidBrush(lang_fill);
    let l_pen = CreatePen(PS_SOLID, 1, COLORREF(0x00454545));
    let ob = SelectObject(hdc, l_brush);
    let op = SelectObject(hdc, l_pen);
    let _ = RoundRect(hdc, lang_r.left, lang_r.top, lang_r.right, lang_r.bottom, 10, 10);
    SelectObject(hdc, ob);
    SelectObject(hdc, op);
    let _ = DeleteObject(l_brush);
    let _ = DeleteObject(l_pen);

    // Language text
    let ui_font = CreateFontW(
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
    let old_font = SelectObject(hdc, ui_font);
    SetBkMode(hdc, TRANSPARENT);
    SetTextColor(hdc, COLORREF(0x00F2F2F2));

    let mut text_rect = RECT {
        left: lang_r.left + 12,
        top: lang_r.top,
        right: lang_r.right - 26,
        bottom: lang_r.bottom,
    };
    let mut lang_wide: Vec<u16> = state.lang_display.encode_utf16().collect();
    let fmt = DRAW_TEXT_FORMAT(DT_LEFT.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_END_ELLIPSIS.0);
    DrawTextW(hdc, &mut lang_wide, &mut text_rect, fmt);

    SelectObject(hdc, old_font);
    let _ = DeleteObject(ui_font);

    // Chevron down inside language dropdown
    let chev_pen = CreatePen(PS_SOLID, 1, COLORREF(0x00CCCCCC));
    let op = SelectObject(hdc, chev_pen);
    let cx = lang_r.right - 15;
    let cy = (lang_r.top + lang_r.bottom) / 2 - 1;
    let _ = MoveToEx(hdc, cx - 4, cy - 2, None);
    let _ = LineTo(hdc, cx, cy + 2);
    let _ = LineTo(hdc, cx + 5, cy - 3);
    SelectObject(hdc, op);
    let _ = DeleteObject(chev_pen);

    // Helper to draw icon button background
    let draw_btn_bg = |r: &RECT, active: bool, hovered: bool, is_close: bool| {
        if active || hovered {
            let color = if is_close && hovered {
                COLORREF(0x00232B9E) // subtle dark red hover
            } else if active {
                COLORREF(0x0066451C) // accent blue-ish highlight in BGR
            } else {
                COLORREF(0x00363636)
            };
            let border = if active {
                COLORREF(0x00FFAA00)
            } else {
                color
            };
            let br = CreateSolidBrush(color);
            let pn = CreatePen(PS_SOLID, 1, border);
            let ob = SelectObject(hdc, br);
            let op = SelectObject(hdc, pn);
            let _ = RoundRect(hdc, r.left, r.top, r.right, r.bottom, 8, 8);
            SelectObject(hdc, ob);
            SelectObject(hdc, op);
            let _ = DeleteObject(br);
            let _ = DeleteObject(pn);
        }
    };

    let icon_pen = CreatePen(PS_SOLID, 1, COLORREF(0x00EEEEEE));
    let op = SelectObject(hdc, icon_pen);
    let null_brush = windows::Win32::Graphics::Gdi::GetStockObject(
        windows::Win32::Graphics::Gdi::NULL_BRUSH,
    );
    let ob = SelectObject(hdc, null_brush);

    // 2. Single-Line Icon (rounded box + horizontal line inside)
    draw_btn_bg(
        &single_r,
        state.mode == OcrMode::SingleLine,
        state.hovered == ToolbarHit::SingleLine,
        false,
    );
    let sx = (single_r.left + single_r.right) / 2;
    let sy = (single_r.top + single_r.bottom) / 2;
    SelectObject(hdc, icon_pen);
    SelectObject(hdc, null_brush);
    let _ = RoundRect(hdc, sx - 7, sy - 6, sx + 8, sy + 7, 3, 3);
    let _ = MoveToEx(hdc, sx - 4, sy, None);
    let _ = LineTo(hdc, sx + 5, sy);

    // 3. Table Icon (3x3 grid)
    draw_btn_bg(
        &table_r,
        state.mode == OcrMode::Table,
        state.hovered == ToolbarHit::Table,
        false,
    );
    let tx = (table_r.left + table_r.right) / 2;
    let ty = (table_r.top + table_r.bottom) / 2;
    SelectObject(hdc, icon_pen);
    SelectObject(hdc, null_brush);
    let _ = RoundRect(hdc, tx - 7, ty - 7, tx + 8, ty + 8, 3, 3);
    let _ = MoveToEx(hdc, tx - 2, ty - 6, None);
    let _ = LineTo(hdc, tx - 2, ty + 7);
    let _ = MoveToEx(hdc, tx + 3, ty - 6, None);
    let _ = LineTo(hdc, tx + 3, ty + 7);
    let _ = MoveToEx(hdc, tx - 6, ty - 2, None);
    let _ = LineTo(hdc, tx + 7, ty - 2);
    let _ = MoveToEx(hdc, tx - 6, ty + 3, None);
    let _ = LineTo(hdc, tx + 7, ty + 3);

    // 4. Settings Icon (Gear glyph via Segoe MDL2 Assets / Fluent Icons)
    draw_btn_bg(
        &settings_r,
        false,
        state.hovered == ToolbarHit::Settings,
        false,
    );
    let gear_font = CreateFontW(
        -15,
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
        w!("Segoe MDL2 Assets"),
    );
    let of2 = SelectObject(hdc, gear_font);
    SetTextColor(hdc, COLORREF(0x00EEEEEE));
    let mut gear_glyph: [u16; 1] = [0xE713];
    let mut sr = settings_r;
    DrawTextW(
        hdc,
        &mut gear_glyph,
        &mut sr,
        DRAW_TEXT_FORMAT(
            windows::Win32::Graphics::Gdi::DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0,
        ),
    );
    SelectObject(hdc, of2);
    let _ = DeleteObject(gear_font);

    // 5. Close Icon (X)
    draw_btn_bg(
        &close_r,
        false,
        state.hovered == ToolbarHit::Close,
        true,
    );
    let cx2 = (close_r.left + close_r.right) / 2;
    let cy2 = (close_r.top + close_r.bottom) / 2;
    SelectObject(hdc, icon_pen);
    let _ = MoveToEx(hdc, cx2 - 5, cy2 - 5, None);
    let _ = LineTo(hdc, cx2 + 6, cy2 + 6);
    let _ = MoveToEx(hdc, cx2 + 5, cy2 - 5, None);
    let _ = LineTo(hdc, cx2 - 6, cy2 + 6);

    SelectObject(hdc, ob);
    SelectObject(hdc, op);
    let _ = DeleteObject(icon_pen);
}

unsafe fn show_language_popup(hwnd: HWND) {
    let Ok(menu) = CreatePopupMenu() else {
        return;
    };

    let mut lang_rect = RECT::default();
    let langs_count = OVERLAY_STATE.with(|cell| {
        if let Some(state) = cell.borrow().as_ref() {
            let (lr, _, _, _, _) = button_rects(&state.toolbar_rect);
            lang_rect = lr;

            let auto_flag = if state.lang_tag.is_none() {
                MF_CHECKED
            } else {
                MF_UNCHECKED
            };
            let _ = AppendMenuW(
                menu,
                MF_STRING | auto_flag,
                1,
                w!("Auto (Windows Profile)"),
            );

            for (idx, (tag, name)) in state.available_langs.iter().enumerate() {
                let flag = if state.lang_tag.as_deref() == Some(tag.as_str()) {
                    MF_CHECKED
                } else {
                    MF_UNCHECKED
                };
                let label = HSTRING::from(format!("{name} ({tag})"));
                let _ = AppendMenuW(menu, MF_STRING | flag, idx + 2, &label);
            }
            state.available_langs.len()
        } else {
            0
        }
    });

    if langs_count == 0 && lang_rect.right == 0 {
        let _ = DestroyMenu(menu);
        return;
    }

    let mut pt = POINT {
        x: lang_rect.left,
        y: lang_rect.bottom + 4,
    };
    let _ = windows::Win32::Graphics::Gdi::ClientToScreen(hwnd, &mut pt);
    let _ = SetForegroundWindow(hwnd);
    let cmd = TrackPopupMenu(
        menu,
        TPM_LEFTALIGN | TPM_TOPALIGN | TPM_RETURNCMD,
        pt.x,
        pt.y,
        0,
        hwnd,
        None,
    );
    let _ = DestroyMenu(menu);

    if cmd.0 == 1 {
        OVERLAY_STATE.with(|cell| {
            if let Some(state) = cell.borrow_mut().as_mut() {
                state.lang_tag = None;
                state.lang_display = ocr::active_language_display_name(None);
            }
        });
        let _ = InvalidateRect(hwnd, None, false);
    } else if cmd.0 >= 2 {
        let idx = (cmd.0 - 2) as usize;
        OVERLAY_STATE.with(|cell| {
            if let Some(state) = cell.borrow_mut().as_mut() {
                if let Some((tag, name)) = state.available_langs.get(idx) {
                    state.lang_tag = Some(tag.clone());
                    state.lang_display = name.clone();
                }
            }
        });
        let _ = InvalidateRect(hwnd, None, false);
    }
}

unsafe fn show_settings_popup(hwnd: HWND) {
    let Ok(menu) = CreatePopupMenu() else {
        return;
    };

    let mut btn_rect = RECT::default();
    OVERLAY_STATE.with(|cell| {
        if let Some(state) = cell.borrow().as_ref() {
            let (_, _, _, sr, _) = button_rects(&state.toolbar_rect);
            btn_rect = sr;

            let f_norm = if state.mode == OcrMode::Normal { MF_CHECKED } else { MF_UNCHECKED };
            let f_single = if state.mode == OcrMode::SingleLine { MF_CHECKED } else { MF_UNCHECKED };
            let f_table = if state.mode == OcrMode::Table { MF_CHECKED } else { MF_UNCHECKED };

            let _ = AppendMenuW(menu, MF_STRING | f_norm, 10, w!("Normal (Preserve Lines)"));
            let _ = AppendMenuW(menu, MF_STRING | f_single, 11, w!("Single Line Mode"));
            let _ = AppendMenuW(menu, MF_STRING | f_table, 12, w!("Table Mode (Tab-Separated)"));
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
            let _ = AppendMenuW(menu, MF_STRING, 14, w!("Change Shortcut..."));
            let _ = AppendMenuW(menu, MF_STRING, 13, w!("Cancel Capture (Esc)"));
        }
    });

    let mut pt = POINT {
        x: btn_rect.left,
        y: btn_rect.bottom + 4,
    };
    let _ = windows::Win32::Graphics::Gdi::ClientToScreen(hwnd, &mut pt);
    let _ = SetForegroundWindow(hwnd);
    let cmd = TrackPopupMenu(
        menu,
        TPM_LEFTALIGN | TPM_TOPALIGN | TPM_RETURNCMD,
        pt.x,
        pt.y,
        0,
        hwnd,
        None,
    );
    let _ = DestroyMenu(menu);

    match cmd.0 {
        10 => {
            OVERLAY_STATE.with(|c| {
                if let Some(s) = c.borrow_mut().as_mut() {
                    s.mode = OcrMode::Normal;
                }
            });
            let _ = InvalidateRect(hwnd, None, false);
        }
        11 => {
            OVERLAY_STATE.with(|c| {
                if let Some(s) = c.borrow_mut().as_mut() {
                    s.mode = OcrMode::SingleLine;
                }
            });
            let _ = InvalidateRect(hwnd, None, false);
        }
        12 => {
            OVERLAY_STATE.with(|c| {
                if let Some(s) = c.borrow_mut().as_mut() {
                    s.mode = OcrMode::Table;
                }
            });
            let _ = InvalidateRect(hwnd, None, false);
        }
        13 => {
            let _ = DestroyWindow(hwnd);
        }
        14 => {
            OVERLAY_STATE.with(|c| {
                if let Some(s) = c.borrow_mut().as_mut() {
                    s.open_shortcut_settings = true;
                }
            });
            let _ = DestroyWindow(hwnd);
        }
        _ => {}
    }
}

unsafe extern "system" fn overlay_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_ERASEBKGND => LRESULT(1),
        WM_SETCURSOR => {
            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            let _ = ScreenToClient(hwnd, &mut pt);

            let cursor = OVERLAY_STATE.with(|cell| {
                cell.borrow().as_ref().map(|s| {
                    let over_tb = !s.dragging && pt_in_rect(&s.toolbar_rect, pt.x, pt.y);
                    if over_tb {
                        s.arrow_cursor
                    } else {
                        s.cross_cursor
                    }
                })
            });

            if let Some(cur) = cursor {
                if !cur.is_invalid() {
                    SetCursor(cur);
                }
            }
            LRESULT(1)
        }
        WM_LBUTTONDOWN => {
            let x = (lparam.0 & 0xFFFF) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;

            let hit = OVERLAY_STATE.with(|cell| {
                cell.borrow()
                    .as_ref()
                    .map(|s| hit_test_toolbar(&s.toolbar_rect, x, y))
                    .unwrap_or(ToolbarHit::None)
            });

            match hit {
                ToolbarHit::LangDropdown => {
                    show_language_popup(hwnd);
                    return LRESULT(0);
                }
                ToolbarHit::SingleLine => {
                    OVERLAY_STATE.with(|cell| {
                        if let Some(s) = cell.borrow_mut().as_mut() {
                            s.mode = if s.mode == OcrMode::SingleLine {
                                OcrMode::Normal
                            } else {
                                OcrMode::SingleLine
                            };
                        }
                    });
                    let _ = InvalidateRect(hwnd, None, false);
                    return LRESULT(0);
                }
                ToolbarHit::Table => {
                    OVERLAY_STATE.with(|cell| {
                        if let Some(s) = cell.borrow_mut().as_mut() {
                            s.mode = if s.mode == OcrMode::Table {
                                OcrMode::Normal
                            } else {
                                OcrMode::Table
                            };
                        }
                    });
                    let _ = InvalidateRect(hwnd, None, false);
                    return LRESULT(0);
                }
                ToolbarHit::Settings => {
                    show_settings_popup(hwnd);
                    return LRESULT(0);
                }
                ToolbarHit::Close => {
                    let _ = DestroyWindow(hwnd);
                    return LRESULT(0);
                }
                ToolbarHit::BarBackground => {
                    return LRESULT(0);
                }
                ToolbarHit::None => {}
            }

            OVERLAY_STATE.with(|cell| {
                if let Some(state) = cell.borrow_mut().as_mut() {
                    state.dragging = true;
                    state.start = POINT { x, y };
                    state.current = POINT { x, y };
                }
            });
            SetCapture(hwnd);
            let _ = InvalidateRect(hwnd, None, false);
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let x = (lparam.0 & 0xFFFF) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;

            let dirty_rect: Option<Option<RECT>> = OVERLAY_STATE.with(|cell| {
                if let Some(state) = cell.borrow_mut().as_mut() {
                    if state.dragging {
                        let (ox, oy, ow, oh) = normalized_rect(state.start, state.current);
                        let shift_held = (GetAsyncKeyState(VK_SHIFT.0 as i32) as u16 & 0x8000) != 0;
                        if shift_held {
                            let dx = x - state.current.x;
                            let dy = y - state.current.y;
                            state.start.x += dx;
                            state.start.y += dy;
                        }
                        state.current = POINT { x, y };
                        let (nx, ny, nw, nh) = normalized_rect(state.start, state.current);
                        let u = RECT {
                            left: ox.min(nx) - 4,
                            top: oy.min(ny) - 4,
                            right: (ox + ow).max(nx + nw) + 4,
                            bottom: (oy + oh).max(ny + nh) + 4,
                        };
                        return Some(Some(u));
                    }
                    let new_hit = hit_test_toolbar(&state.toolbar_rect, x, y);
                    if new_hit != state.hovered {
                        state.hovered = new_hit;
                        return Some(Some(state.toolbar_rect));
                    }
                }
                None
            });
            if let Some(opt_r) = dirty_rect {
                let _ = InvalidateRect(hwnd, opt_r.as_ref().map(|r| r as *const RECT), false);
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let was_dragging = OVERLAY_STATE.with(|cell| {
                cell.borrow().as_ref().map(|s| s.dragging).unwrap_or(false)
            });
            if !was_dragging {
                return LRESULT(0);
            }

            let _ = ReleaseCapture();
            let _ = ShowWindow(hwnd, SW_HIDE);

            let x = (lparam.0 & 0xFFFF) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;

            OVERLAY_STATE.with(|cell| {
                if let Some(state) = cell.borrow_mut().as_mut() {
                    state.dragging = false;
                    state.current = POINT { x, y };

                    let (rx, ry, rw, rh) = normalized_rect(state.start, state.current);
                    let lang = state.lang_tag.as_deref();
                    let mode = state.mode;

                    let text = if rw >= 5 && rh >= 5 {
                        let buf = state.capture.crop_region(rx, ry, rw, rh);
                        ocr::recognize_region(&buf, rw as u32, rh as u32, lang, mode)
                            .unwrap_or_default()
                    } else {
                        let pad_x = 300;
                        let pad_y = 120;
                        let cx = (x - pad_x).max(0);
                        let cy = (y - pad_y).max(0);
                        let cw = (pad_x * 2).min(state.capture.width - cx);
                        let ch = (pad_y * 2).min(state.capture.height - cy);
                        let buf = state.capture.crop_region(cx, cy, cw, ch);
                        ocr::recognize_word_at(
                            &buf,
                            cw as u32,
                            ch as u32,
                            (x - cx) as f32,
                            (y - cy) as f32,
                            lang,
                        )
                        .unwrap_or_default()
                    };

                    if !text.is_empty() {
                        let _ = ocr::copy_to_clipboard(&text);
                        state.extracted_text = Some(text);
                    }
                }
            });

            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_RBUTTONDOWN => {
            let _ = ReleaseCapture();
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_KEYDOWN => {
            let vk = wparam.0 as u16;
            if vk == VK_ESCAPE.0 {
                let _ = ReleaseCapture();
                let _ = DestroyWindow(hwnd);
            } else if vk == 0x53 {
                // 'S' -> toggle Single Line Mode
                let tb = OVERLAY_STATE.with(|c| {
                    if let Some(s) = c.borrow_mut().as_mut() {
                        s.mode = if s.mode == OcrMode::SingleLine {
                            OcrMode::Normal
                        } else {
                            OcrMode::SingleLine
                        };
                        return Some(s.toolbar_rect);
                    }
                    None
                });
                if let Some(r) = tb {
                    let _ = InvalidateRect(hwnd, Some(&r), false);
                }
            } else if vk == 0x54 {
                // 'T' -> toggle Table Mode
                let tb = OVERLAY_STATE.with(|c| {
                    if let Some(s) = c.borrow_mut().as_mut() {
                        s.mode = if s.mode == OcrMode::Table {
                            OcrMode::Normal
                        } else {
                            OcrMode::Table
                        };
                        return Some(s.toolbar_rect);
                    }
                    None
                });
                if let Some(r) = tb {
                    let _ = InvalidateRect(hwnd, Some(&r), false);
                }
            }
            LRESULT(0)
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);

            OVERLAY_STATE.with(|cell| {
                if let Some(state) = cell.borrow_mut().as_mut() {
                    let w = state.capture.width;
                    let h = state.capture.height;

                    if state.back_dc.is_invalid() {
                        state.back_dc = CreateCompatibleDC(hdc);
                        state.back_bmp = CreateCompatibleBitmap(hdc, w, h);
                        state.old_back_bmp = SelectObject(state.back_dc, state.back_bmp);
                    }

                    let back_dc = state.back_dc;
                    let rx_p = ps.rcPaint.left.max(0);
                    let ry_p = ps.rcPaint.top.max(0);
                    let rw_p = (ps.rcPaint.right - rx_p).max(0);
                    let rh_p = (ps.rcPaint.bottom - ry_p).max(0);

                    let src_dc = CreateCompatibleDC(hdc);

                    // 1. Draw dimmed background only in dirty region
                    let old_src = SelectObject(src_dc, state.capture.dark_bmp);
                    let _ = BitBlt(back_dc, rx_p, ry_p, rw_p, rh_p, src_dc, rx_p, ry_p, SRCCOPY);

                    // 2. If dragging, draw bright cutout + crisp cyan/white border
                    if state.dragging {
                        let (rx, ry, rw, rh) = normalized_rect(state.start, state.current);
                        if rw > 0 && rh > 0 {
                            SelectObject(src_dc, state.capture.bright_bmp);
                            let _ = BitBlt(back_dc, rx, ry, rw, rh, src_dc, rx, ry, SRCCOPY);

                            let border_rect = RECT {
                                left: rx,
                                top: ry,
                                right: rx + rw,
                                bottom: ry + rh,
                            };
                            let brush = CreateSolidBrush(COLORREF(0x00FFAA00));
                            let _ = FrameRect(back_dc, &border_rect, brush);
                            let _ = DeleteObject(brush);

                            if rw > 2 && rh > 2 {
                                let inner_rect = RECT {
                                    left: rx + 1,
                                    top: ry + 1,
                                    right: rx + rw - 1,
                                    bottom: ry + rh - 1,
                                };
                                let inner_brush = CreateSolidBrush(COLORREF(0x00FFFFFF));
                                let _ = FrameRect(back_dc, &inner_rect, inner_brush);
                                let _ = DeleteObject(inner_brush);
                            }
                        }
                    }

                    SelectObject(src_dc, old_src);
                    let _ = DeleteDC(src_dc);

                    // 3. Draw floating top toolbar
                    draw_toolbar(back_dc, state);

                    let _ = BitBlt(hdc, rx_p, ry_p, rw_p, rh_p, back_dc, rx_p, ry_p, SRCCOPY);
                }
            });

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

/// Captures the screen and shows the fullscreen selection overlay with floating top toolbar.
pub fn run_capture_overlay(
    lang_tag: Option<String>,
    available_langs: Vec<(String, String)>,
    mode: OcrMode,
) -> Option<OverlayResult> {
    let capture = ScreenCapture::capture_virtual_screen()?;
    let vx = capture.x;
    let vy = capture.y;
    let vw = capture.width;
    let vh = capture.height;

    let primary_w = unsafe { GetSystemMetrics(SM_CXSCREEN) };
    let tb_x = (-vx) + (primary_w - TOOLBAR_W) / 2;
    let tb_y = (-vy) + 18;
    let toolbar_rect = RECT {
        left: tb_x,
        top: tb_y,
        right: tb_x + TOOLBAR_W,
        bottom: tb_y + TOOLBAR_H,
    };

    let lang_display = ocr::active_language_display_name(lang_tag.as_deref());
    let (cross_cursor, custom_cross_owned) = unsafe { create_crisp_crosshair_cursor() };
    let arrow_cursor = unsafe { LoadCursorW(None, IDC_ARROW).unwrap_or_default() };

    OVERLAY_STATE.with(|cell| {
        *cell.borrow_mut() = Some(OverlayState {
            capture,
            lang_tag,
            lang_display,
            available_langs,
            mode,
            toolbar_rect,
            hovered: ToolbarHit::None,
            dragging: false,
            start: POINT::default(),
            current: POINT::default(),
            extracted_text: None,
            open_shortcut_settings: false,
            back_dc: HDC::default(),
            back_bmp: HBITMAP::default(),
            old_back_bmp: HGDIOBJ::default(),
            cross_cursor,
            arrow_cursor,
            custom_cross_owned,
        });
    });

    unsafe {
        let hinstance = GetModuleHandleW(None).ok()?;
        let class_name = w!("TextExtractorOverlayClass");

        CLASS_REGISTERED.with(|reg| {
            if !*reg.borrow() {
                let wc = WNDCLASSW {
                    style: CS_HREDRAW | CS_VREDRAW,
                    lpfnWndProc: Some(overlay_wnd_proc),
                    hInstance: hinstance.into(),
                    hCursor: LoadCursorW(None, IDC_CROSS).unwrap_or_default(),
                    lpszClassName: class_name,
                    ..Default::default()
                };
                if RegisterClassW(&wc) != 0 {
                    *reg.borrow_mut() = true;
                }
            }
        });

        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            class_name,
            w!("Text Extractor"),
            WS_POPUP,
            vx,
            vy,
            vw,
            vh,
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

    OVERLAY_STATE.with(|cell| {
        cell.borrow_mut().take().map(|s| OverlayResult {
            extracted_text: s.extracted_text.clone(),
            selected_lang: s.lang_tag.clone(),
            mode: s.mode,
            open_shortcut_settings: s.open_shortcut_settings,
        })
    })
}
