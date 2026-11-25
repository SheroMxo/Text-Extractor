use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC,
    SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CAPTUREBLT, DIB_RGB_COLORS, HBITMAP,
    SRCCOPY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};

pub struct ScreenCapture {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub bright_bmp: HBITMAP,
    pub dark_bmp: HBITMAP,
    pub bright_bits: *const u8,
}

impl ScreenCapture {
    /// Captures the entire virtual desktop into two 32-bit top-down BGRA DIBs:
    /// one original (`bright_bmp`) and one dimmed (`dark_bmp`) for the overlay background.
    pub fn capture_virtual_screen() -> Option<Self> {
        unsafe {
            let x = GetSystemMetrics(SM_XVIRTUALSCREEN);
            let y = GetSystemMetrics(SM_YVIRTUALSCREEN);
            let width = GetSystemMetrics(SM_CXVIRTUALSCREEN);
            let height = GetSystemMetrics(SM_CYVIRTUALSCREEN);

            if width <= 0 || height <= 0 {
                return None;
            }

            let screen_dc = GetDC(None);
            if screen_dc.is_invalid() {
                return None;
            }

            let mut bmi = BITMAPINFO::default();
            bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            bmi.bmiHeader.biWidth = width;
            bmi.bmiHeader.biHeight = -height; // top-down DIB
            bmi.bmiHeader.biPlanes = 1;
            bmi.bmiHeader.biBitCount = 32;
            bmi.bmiHeader.biCompression = BI_RGB.0;

            let mut bright_ptr: *mut std::ffi::c_void = std::ptr::null_mut();
            let bright_bmp =
                CreateDIBSection(screen_dc, &bmi, DIB_RGB_COLORS, &mut bright_ptr, None, 0)
                    .ok()?;

            let mut dark_ptr: *mut std::ffi::c_void = std::ptr::null_mut();
            let dark_bmp =
                CreateDIBSection(screen_dc, &bmi, DIB_RGB_COLORS, &mut dark_ptr, None, 0).ok()?;

            let mem_dc = CreateCompatibleDC(screen_dc);
            let old_obj = SelectObject(mem_dc, bright_bmp);

            let _ = BitBlt(
                mem_dc,
                0,
                0,
                width,
                height,
                screen_dc,
                x,
                y,
                SRCCOPY | CAPTUREBLT,
            );

            SelectObject(mem_dc, old_obj);
            let _ = DeleteDC(mem_dc);
            ReleaseDC(None, screen_dc);

            // Create dimmed copy (~50% brightness) in dark_ptr
            let byte_count = (width as usize) * (height as usize) * 4;
            let src = std::slice::from_raw_parts(bright_ptr as *const u8, byte_count);
            let dst = std::slice::from_raw_parts_mut(dark_ptr as *mut u8, byte_count);

            for (s, d) in src.chunks_exact(4).zip(dst.chunks_exact_mut(4)) {
                d[0] = s[0] >> 1; // B
                d[1] = s[1] >> 1; // G
                d[2] = s[2] >> 1; // R
                d[3] = 255;       // A
            }

            Some(Self {
                x,
                y,
                width,
                height,
                bright_bmp,
                dark_bmp,
                bright_bits: bright_ptr as *const u8,
            })
        }
    }

    /// Crops a rectangular region `(rx, ry, rw, rh)` (in capture-relative coordinates)
    /// into a contiguous top-down BGRA32 buffer.
    pub fn crop_region(&self, rx: i32, ry: i32, rw: i32, rh: i32) -> Vec<u8> {
        let rx = rx.clamp(0, self.width);
        let ry = ry.clamp(0, self.height);
        let rw = rw.clamp(0, self.width - rx);
        let rh = rh.clamp(0, self.height - ry);

        if rw <= 0 || rh <= 0 {
            return Vec::new();
        }

        let stride = (self.width as usize) * 4;
        let row_bytes = (rw as usize) * 4;
        let mut out = vec![0u8; row_bytes * (rh as usize)];

        unsafe {
            let src = std::slice::from_raw_parts(
                self.bright_bits,
                (self.width as usize) * (self.height as usize) * 4,
            );
            for row in 0..(rh as usize) {
                let src_off = ((ry as usize) + row) * stride + (rx as usize) * 4;
                let dst_off = row * row_bytes;
                out[dst_off..dst_off + row_bytes]
                    .copy_from_slice(&src[src_off..src_off + row_bytes]);
            }
        }

        out
    }
}

impl Drop for ScreenCapture {
    fn drop(&mut self) {
        unsafe {
            if !self.bright_bmp.0.is_null() {
                let _ = DeleteObject(self.bright_bmp);
            }
            if !self.dark_bmp.0.is_null() {
                let _ = DeleteObject(self.dark_bmp);
            }
        }
    }
}
