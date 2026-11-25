use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use windows::core::{HSTRING, Result};
use windows::Globalization::Language;
use windows::Graphics::Imaging::{BitmapAlphaMode, BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::{OcrEngine, OcrResult};
use windows::Storage::Streams::DataWriter;
use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};

const CF_UNICODETEXT: u32 = 13;
const MINIMUM_DIMENSION: u32 = 64;
const PADDING: u32 = 8;
const ENHANCED_SCALE: f64 = 1.5;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum OcrMode {
    Normal,
    SingleLine,
    Table,
}

/// Returns a list of `(language_tag, display_name)` supported by Windows OCR on this machine.
pub fn list_available_languages() -> Vec<(String, String)> {
    let mut langs = Vec::new();
    if let Ok(list) = OcrEngine::AvailableRecognizerLanguages() {
        for lang in list {
            let tag = lang.LanguageTag().map(|s| s.to_string()).unwrap_or_default();
            let name = lang.DisplayName().map(|s| s.to_string()).unwrap_or_else(|_| tag.clone());
            if !tag.is_empty() {
                langs.push((tag, name));
            }
        }
    }
    langs
}

/// Resolves the display name of the currently active OCR language.
pub fn active_language_display_name(lang_tag: Option<&str>) -> String {
    if let Some(tag) = lang_tag {
        if !tag.is_empty() && tag != "auto" {
            if let Ok(lang) = Language::CreateLanguage(&HSTRING::from(tag)) {
                if let Ok(name) = lang.DisplayName() {
                    return name.to_string();
                }
            }
            return tag.to_string();
        }
    }

    if let Ok(engine) = OcrEngine::TryCreateFromUserProfileLanguages() {
        if let Ok(lang) = engine.RecognizerLanguage() {
            if let Ok(name) = lang.DisplayName() {
                return name.to_string();
            }
        }
    }

    "Auto (Profile)".to_string()
}

fn create_ocr_engine(lang_tag: Option<&str>) -> Result<OcrEngine> {
    if let Some(tag) = lang_tag {
        if !tag.is_empty() && tag != "auto" {
            if let Ok(lang) = Language::CreateLanguage(&HSTRING::from(tag)) {
                if let Ok(engine) = OcrEngine::TryCreateFromLanguage(&lang) {
                    return Ok(engine);
                }
            }
        }
    }
    OcrEngine::TryCreateFromUserProfileLanguages()
}

#[inline]
fn cubic_weight(t: f32) -> f32 {
    // Catmull-Rom spline (a = -0.5)
    let x = t.abs();
    if x <= 1.0 {
        (1.5 * x - 2.5) * x * x + 1.0
    } else if x < 2.0 {
        ((-0.5 * x + 2.5) * x - 4.0) * x + 2.0
    } else {
        0.0
    }
}

/// PowerToys-equivalent `BitmapPreprocessor.Prepare`:
/// - Upscales small/medium regions by 1.5x using bicubic interpolation
/// - Pads dimensions smaller than `MINIMUM_DIMENSION` (64px) with `PADDING` (8px)
///   filled with the top-left background pixel color.
fn prepare_bitmap_for_ocr(
    bgra: &[u8],
    width: u32,
    height: u32,
    is_word_mode: bool,
) -> (Vec<u8>, u32, u32, f64, f64, u32, u32) {
    let max_dim = OcrEngine::MaxImageDimension().unwrap_or(2600) as f64;
    let scale = if !is_word_mode
        && (width as f64 * ENHANCED_SCALE) <= max_dim
        && (height as f64 * ENHANCED_SCALE) <= max_dim
    {
        ENHANCED_SCALE
    } else if (width as f64) <= max_dim && (height as f64) <= max_dim {
        1.0
    } else {
        (max_dim / width as f64).min(max_dim / height as f64)
    };

    let scaled_w = ((width as f64 * scale).round() as u32).max(1);
    let scaled_h = ((height as f64 * scale).round() as u32).max(1);

    let pad_h = scaled_w < MINIMUM_DIMENSION;
    let pad_v = scaled_h < MINIMUM_DIMENSION;
    let out_w = if pad_h {
        (scaled_w + PADDING * 2).max(MINIMUM_DIMENSION + PADDING * 2)
    } else {
        scaled_w
    };
    let out_h = if pad_v {
        (scaled_h + PADDING * 2).max(MINIMUM_DIMENSION + PADDING * 2)
    } else {
        scaled_h
    };
    let offset_x = if pad_h { PADDING } else { 0 };
    let offset_y = if pad_v { PADDING } else { 0 };

    // Background fill from top-left pixel (0, 0)
    let bg_b = bgra[0];
    let bg_g = bgra[1];
    let bg_r = bgra[2];
    let mut out = vec![0u8; (out_w * out_h * 4) as usize];
    for px in out.chunks_exact_mut(4) {
        px[0] = bg_b;
        px[1] = bg_g;
        px[2] = bg_r;
        px[3] = 255;
    }

    let sw = width as i32;
    let sh = height as i32;
    let scale_x = width as f32 / scaled_w as f32;
    let scale_y = height as f32 / scaled_h as f32;

    for dy in 0..scaled_h {
        let sy_f = (dy as f32 + 0.5) * scale_y - 0.5;
        let iy = sy_f.floor() as i32;
        let fy = sy_f - iy as f32;
        let wy = [
            cubic_weight(fy + 1.0),
            cubic_weight(fy),
            cubic_weight(fy - 1.0),
            cubic_weight(fy - 2.0),
        ];

        for dx in 0..scaled_w {
            let sx_f = (dx as f32 + 0.5) * scale_x - 0.5;
            let ix = sx_f.floor() as i32;
            let fx = sx_f - ix as f32;
            let wx = [
                cubic_weight(fx + 1.0),
                cubic_weight(fx),
                cubic_weight(fx - 1.0),
                cubic_weight(fx - 2.0),
            ];

            let mut acc_b = 0.0f32;
            let mut acc_g = 0.0f32;
            let mut acc_r = 0.0f32;

            for (ky, &w_y) in wy.iter().enumerate() {
                let py = (iy + ky as i32 - 1).clamp(0, sh - 1) as usize;
                let row_off = py * (width as usize) * 4;
                for (kx, &w_x) in wx.iter().enumerate() {
                    let px_idx = (ix + kx as i32 - 1).clamp(0, sw - 1) as usize;
                    let idx = row_off + px_idx * 4;
                    let w = w_x * w_y;
                    acc_b += bgra[idx] as f32 * w;
                    acc_g += bgra[idx + 1] as f32 * w;
                    acc_r += bgra[idx + 2] as f32 * w;
                }
            }

            let dst_idx = (((dy + offset_y) * out_w + (dx + offset_x)) * 4) as usize;
            out[dst_idx] = acc_b.round().clamp(0.0, 255.0) as u8;
            out[dst_idx + 1] = acc_g.round().clamp(0.0, 255.0) as u8;
            out[dst_idx + 2] = acc_r.round().clamp(0.0, 255.0) as u8;
            out[dst_idx + 3] = 255;
        }
    }

    (
        out,
        out_w,
        out_h,
        scaled_w as f64 / width as f64,
        scaled_h as f64 / height as f64,
        offset_x,
        offset_y,
    )
}

fn create_software_bitmap(bgra: &[u8], width: u32, height: u32) -> Result<SoftwareBitmap> {
    let writer = DataWriter::new()?;
    writer.WriteBytes(bgra)?;
    let buffer = writer.DetachBuffer()?;
    SoftwareBitmap::CreateCopyWithAlphaFromBuffer(
        &buffer,
        BitmapPixelFormat::Bgra8,
        width as i32,
        height as i32,
        BitmapAlphaMode::Ignore,
    )
}

fn format_ocr_result(ocr_result: &OcrResult, mode: OcrMode) -> String {
    match mode {
        OcrMode::Normal => {
            let mut lines_out = Vec::new();
            if let Ok(lines) = ocr_result.Lines() {
                for line in lines {
                    if let Ok(text) = line.Text() {
                        let s = text.to_string();
                        if !s.is_empty() {
                            lines_out.push(s);
                        }
                    }
                }
            }
            lines_out.join("\r\n")
        }
        OcrMode::SingleLine => {
            let mut lines_out = Vec::new();
            if let Ok(lines) = ocr_result.Lines() {
                for line in lines {
                    if let Ok(text) = line.Text() {
                        let s = text.to_string().trim().to_string();
                        if !s.is_empty() {
                            lines_out.push(s);
                        }
                    }
                }
            }
            lines_out.join(" ")
        }
        OcrMode::Table => {
            #[derive(Clone)]
            struct WordBox {
                x: f32,
                y: f32,
                w: f32,
                h: f32,
                text: String,
            }

            let mut all_words = Vec::new();
            if let Ok(lines) = ocr_result.Lines() {
                for line in lines {
                    if let Ok(words) = line.Words() {
                        for word in words {
                            if let (Ok(rect), Ok(text)) = (word.BoundingRect(), word.Text()) {
                                all_words.push(WordBox {
                                    x: rect.X,
                                    y: rect.Y,
                                    w: rect.Width,
                                    h: rect.Height,
                                    text: text.to_string(),
                                });
                            }
                        }
                    }
                }
            }

            if all_words.is_empty() {
                return String::new();
            }

            all_words.sort_by(|a, b| {
                let ca = a.y + a.h * 0.5;
                let cb = b.y + b.h * 0.5;
                ca.partial_cmp(&cb).unwrap_or(std::cmp::Ordering::Equal)
            });

            let mut rows: Vec<Vec<WordBox>> = Vec::new();
            for word in all_words {
                let w_center = word.y + word.h * 0.5;
                if let Some(last_row) = rows.last_mut() {
                    let row_avg_y: f32 =
                        last_row.iter().map(|w| w.y + w.h * 0.5).sum::<f32>() / last_row.len() as f32;
                    let row_avg_h: f32 =
                        last_row.iter().map(|w| w.h).sum::<f32>() / last_row.len() as f32;
                    if (w_center - row_avg_y).abs() <= row_avg_h * 0.6 {
                        last_row.push(word);
                        continue;
                    }
                }
                rows.push(vec![word]);
            }

            let mut lines_out = Vec::with_capacity(rows.len());
            for mut row in rows {
                row.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal));
                let mut line_str = String::new();
                for (i, w) in row.iter().enumerate() {
                    if i > 0 {
                        let prev = &row[i - 1];
                        let gap = w.x - (prev.x + prev.w);
                        if gap > prev.h * 1.4 {
                            line_str.push('\t');
                        } else {
                            line_str.push(' ');
                        }
                    }
                    line_str.push_str(&w.text);
                }
                lines_out.push(line_str);
            }

            lines_out.join("\r\n")
        }
    }
}

/// Runs Windows OCR on a cropped BGRA32 image buffer with PowerToys-equivalent preprocessing.
pub fn recognize_region(
    bgra: &[u8],
    width: u32,
    height: u32,
    lang_tag: Option<&str>,
    mode: OcrMode,
) -> Result<String> {
    if width == 0 || height == 0 || bgra.is_empty() {
        return Ok(String::new());
    }

    let (prep_buf, prep_w, prep_h, _, _, _, _) = prepare_bitmap_for_ocr(bgra, width, height, false);
    let engine = create_ocr_engine(lang_tag)?;
    let bitmap = create_software_bitmap(&prep_buf, prep_w, prep_h)?;
    let ocr_result = engine.RecognizeAsync(&bitmap)?.get()?;

    Ok(format_ocr_result(&ocr_result, mode))
}

/// Runs Windows OCR on a buffer and returns the single word under `(click_x, click_y)`, if any.
pub fn recognize_word_at(
    bgra: &[u8],
    width: u32,
    height: u32,
    click_x: f32,
    click_y: f32,
    lang_tag: Option<&str>,
) -> Result<String> {
    if width == 0 || height == 0 || bgra.is_empty() {
        return Ok(String::new());
    }

    let (prep_buf, prep_w, prep_h, sx, sy, ox, oy) =
        prepare_bitmap_for_ocr(bgra, width, height, true);
    let tx = click_x * sx as f32 + ox as f32;
    let ty = click_y * sy as f32 + oy as f32;

    let engine = create_ocr_engine(lang_tag)?;
    let bitmap = create_software_bitmap(&prep_buf, prep_w, prep_h)?;
    let ocr_result = engine.RecognizeAsync(&bitmap)?.get()?;

    if let Ok(lines) = ocr_result.Lines() {
        for line in lines {
            if let Ok(words) = line.Words() {
                for word in words {
                    if let Ok(rect) = word.BoundingRect() {
                        if tx >= rect.X
                            && tx <= rect.X + rect.Width
                            && ty >= rect.Y
                            && ty <= rect.Y + rect.Height
                        {
                            return Ok(word.Text()?.to_string());
                        }
                    }
                }
            }
        }
    }

    Ok(String::new())
}

/// Copies UTF-8 text to the Windows clipboard as UTF-16 (`CF_UNICODETEXT`).
pub fn copy_to_clipboard(text: &str) -> Result<()> {
    let wide: Vec<u16> = OsStr::new(text)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let byte_len = wide.len() * std::mem::size_of::<u16>();

    unsafe {
        OpenClipboard(None)?;
        if let Err(e) = EmptyClipboard() {
            let _ = CloseClipboard();
            return Err(e);
        }

        let hmem: HGLOBAL = match GlobalAlloc(GMEM_MOVEABLE, byte_len) {
            Ok(h) => h,
            Err(e) => {
                let _ = CloseClipboard();
                return Err(e);
            }
        };

        let ptr = GlobalLock(hmem) as *mut u16;
        if ptr.is_null() {
            let _ = GlobalFree(hmem);
            let _ = CloseClipboard();
            return Err(windows::core::Error::from_win32());
        }

        std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr, wide.len());
        let _ = GlobalUnlock(hmem);

        if let Err(e) = SetClipboardData(CF_UNICODETEXT, HANDLE(hmem.0)) {
            let _ = GlobalFree(hmem);
            let _ = CloseClipboard();
            return Err(e);
        }

        let _ = CloseClipboard();
    }

    Ok(())
}
