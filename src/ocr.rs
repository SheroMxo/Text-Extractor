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

            // Sort top-to-bottom by vertical center
            all_words.sort_by(|a, b| {
                let ca = a.y + a.h * 0.5;
                let cb = b.y + b.h * 0.5;
                ca.partial_cmp(&cb).unwrap_or(std::cmp::Ordering::Equal)
            });

            // Group words into rows based on vertical overlap
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

/// Runs Windows OCR on a cropped BGRA32 image buffer and formats it according to `mode`.
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

    let engine = create_ocr_engine(lang_tag)?;
    let bitmap = create_software_bitmap(bgra, width, height)?;
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

    let engine = create_ocr_engine(lang_tag)?;
    let bitmap = create_software_bitmap(bgra, width, height)?;
    let ocr_result = engine.RecognizeAsync(&bitmap)?.get()?;

    if let Ok(lines) = ocr_result.Lines() {
        for line in lines {
            if let Ok(words) = line.Words() {
                for word in words {
                    if let Ok(rect) = word.BoundingRect() {
                        if click_x >= rect.X
                            && click_x <= rect.X + rect.Width
                            && click_y >= rect.Y
                            && click_y <= rect.Y + rect.Height
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
