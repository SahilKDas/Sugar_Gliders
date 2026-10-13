//! Windows' own software ClearType rasterizer; tiny-skia still draws the UI.
#![allow(non_snake_case, clippy::too_many_arguments)] // COM interface signatures.
use std::{cell::RefCell, collections::HashMap, ffi::c_void, rc::Rc};
type Carets = Rc<Vec<(usize, f32)>>;
type CachedCarets = Rc<RefCell<Option<Carets>>>;
use unicode_segmentation::UnicodeSegmentation;
use windows::{
    core::{implement, w, IUnknown, Ref, Result, BOOL},
    Win32::{
        Foundation::{COLORREF, E_POINTER, HWND},
        Graphics::{DirectWrite::*, Gdi::*},
    },
};

#[derive(Clone)]
struct Line {
    layout: IDWriteTextLayout,
    width: f32,
    baseline: f32,
    positions: CachedCarets,
}
struct Surface {
    target: IDWriteBitmapRenderTarget,
    width: u32,
    height: u32,
}
pub struct NativeText {
    factory: IDWriteFactory,
    params: RefCell<IDWriteRenderingParams>,
    monitor: RefCell<HMONITOR>,
    lines: RefCell<HashMap<(String, u32), Line>>,
    wraps: RefCell<HashMap<(String, u32, u32), Vec<String>>>,
    surface: RefCell<Surface>,
}
impl NativeText {
    pub fn new() -> Result<Self> {
        // SAFETY: DirectWrite factory/interface pointers are owned by the Windows bindings.
        unsafe {
            let factory: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let params = factory.CreateRenderingParams()?;
            let target = factory
                .GetGdiInterop()?
                .CreateBitmapRenderTarget(None, 64, 64)?;
            Ok(Self {
                factory,
                params: RefCell::new(params),
                monitor: RefCell::new(HMONITOR::default()),
                lines: RefCell::new(HashMap::new()),
                wraps: RefCell::new(HashMap::new()),
                surface: RefCell::new(Surface {
                    target,
                    width: 64,
                    height: 64,
                }),
            })
        }
    }
    pub fn clear(&self) {
        self.lines.borrow_mut().clear();
        self.wraps.borrow_mut().clear();
    }
    pub fn monitor(&self, hwnd: *mut c_void) {
        // SAFETY: the calling UI thread holds the live window for this HWND.
        unsafe {
            let monitor = MonitorFromWindow(HWND(hwnd), MONITOR_DEFAULTTONEAREST);
            if *self.monitor.borrow() != monitor {
                if let Ok(params) = self.factory.CreateMonitorRenderingParams(monitor) {
                    *self.params.borrow_mut() = params;
                    *self.monitor.borrow_mut() = monitor;
                }
            }
        }
    }
    fn line(&self, text: &str, size: f32) -> Result<Line> {
        let key = (text.to_owned(), size.to_bits());
        if let Some(line) = self.lines.borrow().get(&key) {
            return Ok(line.clone());
        }
        // SAFETY: UTF-16 input is held through layout creation, which copies the string.
        // Cached inputs are visible UI text or password bullets, never plaintext passwords.
        let line = unsafe {
            let format = self.factory.CreateTextFormat(
                w!("Segoe UI"),
                None,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                size,
                w!("en-us"),
            )?;
            format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            let utf16: Vec<u16> = text.encode_utf16().collect();
            let layout = self
                .factory
                .CreateTextLayout(&utf16, &format, 100_000.0, size * 4.0)?;
            let mut metrics = DWRITE_TEXT_METRICS::default();
            layout.GetMetrics(&mut metrics)?;
            let mut lines = [DWRITE_LINE_METRICS::default(); 1];
            let mut count = 0;
            layout.GetLineMetrics(Some(&mut lines), &mut count)?;
            Line {
                layout,
                width: metrics.widthIncludingTrailingWhitespace,
                baseline: lines[0].baseline,
                positions: Rc::new(RefCell::new(None)),
            }
        };
        let mut cache = self.lines.borrow_mut();
        if cache.len() >= 256 {
            cache.clear();
        }
        cache.insert(key, line.clone());
        Ok(line)
    }
    pub fn hit_test(&self, text: &str, size: f32, x: f32) -> Result<usize> {
        let line = self.line(text, size)?;
        let mut trailing = BOOL::default();
        let mut inside = BOOL::default();
        let mut metrics = DWRITE_HIT_TEST_METRICS::default();
        // SAFETY: the layout owns the string; all output pointers refer to local values.
        unsafe {
            line.layout.HitTestPoint(
                x,
                line.baseline * 0.5,
                &mut trailing,
                &mut inside,
                &mut metrics,
            )?;
        }
        let position = metrics.textPosition
            + if trailing.as_bool() {
                metrics.length
            } else {
                0
            };
        let mut utf16 = 0;
        for (byte, c) in text.char_indices() {
            if utf16 >= position {
                return Ok(byte);
            }
            utf16 += c.len_utf16() as u32;
        }
        Ok(text.len())
    }
    #[cfg(test)]
    pub fn cached_fragment(&self, fragment: &str) -> bool {
        self.lines
            .borrow()
            .keys()
            .any(|(text, _)| text.contains(fragment))
    }
    pub fn positions(&self, text: &str, size: f32) -> Result<Carets> {
        let line = self.line(text, size)?;
        if let Some(positions) = line.positions.borrow().as_ref() {
            return Ok(positions.clone());
        }
        let mut positions = Vec::new();
        let mut utf16 = 0;
        for (offset, g) in text
            .grapheme_indices(true)
            .chain(std::iter::once((text.len(), "")))
        {
            let mut x = 0.0;
            let mut y = 0.0;
            let mut hit = DWRITE_HIT_TEST_METRICS::default();
            // SAFETY: positions are valid UTF-16 cluster boundaries within the owned layout.
            unsafe {
                line.layout
                    .HitTestTextPosition(utf16, false, &mut x, &mut y, &mut hit)?;
            }
            positions.push((offset, x));
            utf16 += g.encode_utf16().count() as u32;
        }
        let positions = Rc::new(positions);
        *line.positions.borrow_mut() = Some(positions.clone());
        Ok(positions)
    }
    pub fn wrap(&self, text: &str, size: f32, max: f32) -> Result<Vec<String>> {
        let key = (text.to_owned(), size.to_bits(), max.to_bits());
        if let Some(lines) = self.wraps.borrow().get(&key) {
            return Ok(lines.clone());
        }
        // SAFETY: DirectWrite copies the UTF-16 string and reports bounded line lengths.
        unsafe {
            let format = self.factory.CreateTextFormat(
                w!("Segoe UI"),
                None,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                size,
                w!("en-us"),
            )?;
            let utf16: Vec<u16> = text.encode_utf16().collect();
            let layout = self
                .factory
                .CreateTextLayout(&utf16, &format, max.max(1.0), 100_000.0)?;
            let mut count = 0;
            let _ = layout.GetLineMetrics(None, &mut count);
            let mut metrics = vec![DWRITE_LINE_METRICS::default(); count as usize];
            layout.GetLineMetrics(Some(&mut metrics), &mut count)?;
            let mut offset = 0;
            let mut lines = Vec::new();
            for line in metrics.iter().take(count as usize) {
                let end = offset + line.length as usize;
                if end > utf16.len() {
                    return Err(windows::core::Error::from_hresult(E_POINTER));
                }
                lines.push(
                    String::from_utf16_lossy(&utf16[offset..end])
                        .trim_end_matches(['\r', '\n'])
                        .to_owned(),
                );
                offset = end;
            }
            let mut cache = self.wraps.borrow_mut();
            if cache.len() >= 256 {
                cache.clear();
            }
            cache.insert(key, lines.clone());
            Ok(lines)
        }
    }
    pub fn measure(&self, text: &str, size: f32) -> Result<f32> {
        Ok(self.line(text, size)?.width)
    }
    pub fn draw(
        &self,
        p: &mut tiny_skia::Pixmap,
        text: &str,
        x: f32,
        baseline: f32,
        size: f32,
        color: u32,
        max: f32,
        scale: f32,
    ) -> Result<()> {
        if text.is_empty() || max <= 0.0 {
            return Ok(());
        }
        let line = self.line(text, size)?;
        let left = (x * scale).floor() as i32 - 2;
        let top = ((baseline - line.baseline) * scale).floor() as i32 - 2;
        let width = ((max.min(line.width + size) + 4.0) * scale).ceil().max(1.0) as u32;
        let height = (size * 4.0 * scale + 4.0).ceil().max(1.0) as u32;
        let mut surface = self.surface.borrow_mut();
        // SAFETY: render-target DIB memory is accessed synchronously on this UI thread;
        // the target remains alive, and dimensions/stride/bit depth are validated first.
        unsafe {
            if width > surface.width || height > surface.height {
                let new_width = width.max(surface.width);
                let new_height = height.max(surface.height);
                surface.target.Resize(new_width, new_height)?;
                surface.width = new_width;
                surface.height = new_height;
            }
            surface.target.SetPixelsPerDip(scale)?;
            let dc = surface.target.GetMemoryDC();
            let mut dib = DIBSECTION::default();
            if GetObjectW(
                GetCurrentObject(dc, OBJ_BITMAP),
                std::mem::size_of::<DIBSECTION>() as i32,
                Some(&mut dib as *mut _ as *mut c_void),
            ) == 0
                || dib.dsBm.bmBits.is_null()
                || dib.dsBm.bmBitsPixel != 32
                || dib.dsBm.bmWidthBytes < surface.width as i32 * 4
                || dib.dsBm.bmHeight < surface.height as i32
            {
                return Err(windows::core::Error::from_hresult(E_POINTER));
            }
            let stride = dib.dsBm.bmWidthBytes as usize;
            let pointer = dib.dsBm.bmBits as *mut u8;
            let length = stride * surface.height as usize;
            // DirectWrite guarantees a top-down DIB. GetObject normalizes the reported
            // height to positive; its sign cannot be used to infer scanline orientation.
            let row = |y: u32| y as usize;
            {
                let bytes = std::slice::from_raw_parts_mut(
                    dib.dsBm.bmBits as *mut u8,
                    stride * surface.height as usize,
                );
                let _ = GdiFlush();
                // ClearType blends into the actual opaque background, including selections.
                for sy in 0..height {
                    for sx in 0..width {
                        let px = left + sx as i32;
                        let py = top + sy as i32;
                        let dst = row(sy) * stride + sx as usize * 4;
                        if px >= 0 && py >= 0 && px < p.width() as i32 && py < p.height() as i32 {
                            let src = (py as usize * p.width() as usize + px as usize) * 4;
                            bytes[dst..dst + 4].copy_from_slice(&[
                                p.data()[src + 2],
                                p.data()[src + 1],
                                p.data()[src],
                                255,
                            ]);
                        } else {
                            bytes[dst..dst + 4].fill(255);
                        }
                    }
                }
            }
            let renderer: IDWriteTextRenderer = Renderer {
                target: surface.target.clone(),
                params: self.params.borrow().clone(),
                color: COLORREF(((color & 255) << 16) | (color & 0xff00) | (color >> 16)),
                scale,
            }
            .into();
            line.layout.Draw(
                None,
                &renderer,
                (x * scale - left as f32) / scale,
                (baseline * scale - top as f32) / scale - line.baseline,
            )?;
            let _ = GdiFlush();
            let bytes = std::slice::from_raw_parts(pointer, length);
            for sy in 0..height {
                for sx in 0..width {
                    let px = left + sx as i32;
                    let py = top + sy as i32;
                    if px < 0
                        || py < 0
                        || px >= p.width() as i32
                        || py >= p.height() as i32
                        || px as f32 >= (x + max) * scale
                    {
                        continue;
                    }
                    let src = row(sy) * stride + sx as usize * 4;
                    let dst = (py as usize * p.width() as usize + px as usize) * 4;
                    p.data_mut()[dst..dst + 4].copy_from_slice(&[
                        bytes[src + 2],
                        bytes[src + 1],
                        bytes[src],
                        255,
                    ]);
                }
            }
        }
        Ok(())
    }
}

#[implement(IDWriteTextRenderer)]
struct Renderer {
    target: IDWriteBitmapRenderTarget,
    params: IDWriteRenderingParams,
    color: COLORREF,
    scale: f32,
}
impl IDWritePixelSnapping_Impl for Renderer_Impl {
    fn IsPixelSnappingDisabled(&self, _: *const c_void) -> Result<BOOL> {
        Ok(false.into())
    }
    fn GetCurrentTransform(&self, _: *const c_void, transform: *mut DWRITE_MATRIX) -> Result<()> {
        if transform.is_null() {
            return Err(windows::core::Error::from_hresult(E_POINTER));
        }
        // SAFETY: DirectWrite supplies the writable transform output for the callback.
        unsafe {
            transform.write(DWRITE_MATRIX {
                m11: 1.0,
                m22: 1.0,
                ..Default::default()
            });
        }
        Ok(())
    }
    fn GetPixelsPerDip(&self, _: *const c_void) -> Result<f32> {
        Ok(self.scale)
    }
}
impl IDWriteTextRenderer_Impl for Renderer_Impl {
    fn DrawGlyphRun(
        &self,
        _: *const c_void,
        x: f32,
        y: f32,
        mode: DWRITE_MEASURING_MODE,
        run: *const DWRITE_GLYPH_RUN,
        _: *const DWRITE_GLYPH_RUN_DESCRIPTION,
        _: Ref<'_, IUnknown>,
    ) -> Result<()> {
        // SAFETY: DirectWrite's glyph run is valid for the duration of this callback.
        unsafe {
            self.target
                .DrawGlyphRun(x, y, mode, run, &self.params, self.color, None)
        }
    }
    fn DrawUnderline(
        &self,
        _: *const c_void,
        _: f32,
        _: f32,
        _: *const DWRITE_UNDERLINE,
        _: Ref<'_, IUnknown>,
    ) -> Result<()> {
        Ok(())
    }
    fn DrawStrikethrough(
        &self,
        _: *const c_void,
        _: f32,
        _: f32,
        _: *const DWRITE_STRIKETHROUGH,
        _: Ref<'_, IUnknown>,
    ) -> Result<()> {
        Ok(())
    }
    fn DrawInlineObject(
        &self,
        _: *const c_void,
        _: f32,
        _: f32,
        _: Ref<'_, IDWriteInlineObject>,
        _: BOOL,
        _: BOOL,
        _: Ref<'_, IUnknown>,
    ) -> Result<()> {
        Ok(())
    }
}

pub fn caret_preferences() -> (f32, f32) {
    use windows::Win32::UI::WindowsAndMessaging::*;
    // SAFETY: these read-only system queries write a single u32 when requested.
    unsafe {
        let milliseconds = GetCaretBlinkTime();
        let mut width = 1u32;
        let _ = SystemParametersInfoW(
            SPI_GETCARETWIDTH,
            0,
            Some(&mut width as *mut _ as *mut c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
        (
            if milliseconds == u32::MAX || milliseconds == 0 {
                f32::INFINITY
            } else {
                milliseconds as f32 / 1000.0
            },
            width.max(1) as f32,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn grown_render_target_preserves_upright_baseline_and_background() {
        let native = NativeText::new().unwrap();
        let mut warm = tiny_skia::Pixmap::new(600, 250).unwrap();
        warm.fill(tiny_skia::Color::WHITE);
        native
            .draw(
                &mut warm,
                "Grow reusable target",
                10.0,
                90.0,
                60.0,
                0,
                580.0,
                1.0,
            )
            .unwrap();
        let mut image = tiny_skia::Pixmap::new(120, 80).unwrap();
        image.fill(tiny_skia::Color::WHITE);
        native
            .draw(&mut image, "F", 10.0, 28.0, 20.0, 0, 90.0, 1.0)
            .unwrap();
        let rows: Vec<_> = image
            .data()
            .as_chunks::<4>()
            .0
            .iter()
            .enumerate()
            .filter(|(_, p)| p[0] < 128 || p[1] < 128 || p[2] < 128)
            .map(|(i, _)| i / 120)
            .collect();
        assert!(!rows.is_empty());
        assert!(*rows.iter().min().unwrap() < 20);
        assert!(*rows.iter().max().unwrap() <= 29);
        assert!(image
            .data()
            .as_chunks::<4>()
            .0
            .iter()
            .skip(40 * 120)
            .all(|p| *p == [255, 255, 255, 255]));
    }
    #[test]
    fn real_cleartype_blends_independent_rgb_channels_into_opaque_pixels() {
        let native = NativeText::new().unwrap();
        // Explicit RGB geometry makes the test independent of the workstation setting.
        *native.params.borrow_mut() = unsafe {
            native
                .factory
                .CreateCustomRenderingParams(
                    2.2,
                    1.0,
                    1.0,
                    DWRITE_PIXEL_GEOMETRY_RGB,
                    DWRITE_RENDERING_MODE_CLEARTYPE_NATURAL_SYMMETRIC,
                )
                .unwrap()
        };
        let mut image = tiny_skia::Pixmap::new(240, 70).unwrap();
        image.fill(tiny_skia::Color::WHITE);
        native
            .draw(
                &mut image,
                "ClearType AV office",
                10.3,
                35.0,
                17.0,
                0,
                210.0,
                1.0,
            )
            .unwrap();
        assert!(image
            .data()
            .as_chunks::<4>()
            .0
            .iter()
            .any(|p| p[0] != p[1] || p[1] != p[2]));
        assert!(image.data().as_chunks::<4>().0.iter().all(|p| p[3] == 255));
        assert!(image
            .data()
            .as_chunks::<4>()
            .0
            .iter()
            .any(|p| p[0] < 50 && p[1] < 50 && p[2] < 50));
    }
    #[test]
    fn shaped_caret_positions_are_cached_and_unicode_boundaries_are_valid() {
        let native = NativeText::new().unwrap();
        let value = "AV office e\u{301} 🍁";
        let first = native.positions(value, 16.0).unwrap();
        let second = native.positions(value, 16.0).unwrap();
        assert!(Rc::ptr_eq(&first, &second));
        assert_eq!(first.last().unwrap().0, value.len());
        assert!(first
            .iter()
            .all(|(i, x)| value.is_char_boundary(*i) && x.is_finite()));
        assert!((first.last().unwrap().1 - native.measure(value, 16.0).unwrap()).abs() < 0.01);
        assert!(
            native
                .wrap("A cozy native text box with proper wrapping", 16.0, 100.0)
                .unwrap()
                .len()
                > 1
        );
    }
}

#[link(name = "user32")]
unsafe extern "system" {
    fn GetDoubleClickTime() -> u32;
}
pub fn double_click_ms() -> u128 {
    unsafe { GetDoubleClickTime() as u128 }
}
