//! Thin Direct2D / DirectWrite / DirectComposition renderer.
//! Everything is drawn on demand into a per-pixel-alpha composition swap chain.

use std::collections::HashMap;
use std::mem::ManuallyDrop;

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::DirectComposition::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::*;
use windows_numerics::{Matrix3x2, Vector2};

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const fn rgb(hex: u32) -> Self {
        Color {
            r: ((hex >> 16) & 0xff) as f32 / 255.0,
            g: ((hex >> 8) & 0xff) as f32 / 255.0,
            b: (hex & 0xff) as f32 / 255.0,
            a: 1.0,
        }
    }
    pub fn a(self, a: f32) -> Self {
        Color { a: self.a * a, ..self }
    }
    pub fn parse(s: &str) -> Option<Color> {
        let s = s.trim().trim_start_matches('#');
        if s.len() != 6 {
            return None;
        }
        u32::from_str_radix(s, 16).ok().map(Color::rgb)
    }
    pub fn to_bgr(self) -> u32 {
        let c = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
        (c(self.b) << 16) | (c(self.g) << 8) | c(self.r)
    }
    fn d2d(self) -> D2D1_COLOR_F {
        D2D1_COLOR_F { r: self.r, g: self.g, b: self.b, a: self.a }
    }
}

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Rect { x, y, w, h }
    }
    pub fn right(&self) -> f32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
    pub fn contains(&self, p: (f32, f32)) -> bool {
        p.0 >= self.x && p.0 < self.right() && p.1 >= self.y && p.1 < self.bottom()
    }
    pub fn inset(&self, d: f32) -> Rect {
        Rect::new(self.x + d, self.y + d, (self.w - 2.0 * d).max(0.0), (self.h - 2.0 * d).max(0.0))
    }
    pub fn intersect(&self, o: &Rect) -> Rect {
        let x = self.x.max(o.x);
        let y = self.y.max(o.y);
        let r = self.right().min(o.right());
        let b = self.bottom().min(o.bottom());
        Rect::new(x, y, (r - x).max(0.0), (b - y).max(0.0))
    }
    fn d2d(&self) -> D2D_RECT_F {
        D2D_RECT_F { left: self.x, top: self.y, right: self.right(), bottom: self.bottom() }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Font {
    Ui,
    Icon,
    Mono,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// Text style. Single-line text is vertically centred and ellipsized; `wrap`
/// text is top-aligned and wraps.
#[derive(Clone, Copy)]
pub struct Ts {
    pub size: f32,
    pub weight: u16,
    pub font: Font,
    pub align: Align,
    pub wrap: bool,
}

pub fn ts(size: f32) -> Ts {
    Ts { size, weight: 400, font: Font::Ui, align: Align::Left, wrap: false }
}

impl Ts {
    pub fn semi(mut self) -> Self {
        self.weight = 600;
        self
    }
    pub fn bold(mut self) -> Self {
        self.weight = 700;
        self
    }
    pub fn center(mut self) -> Self {
        self.align = Align::Center;
        self
    }
    pub fn right(mut self) -> Self {
        self.align = Align::Right;
        self
    }
    pub fn wrap(mut self) -> Self {
        self.wrap = true;
        self
    }
    pub fn icon(mut self) -> Self {
        self.font = Font::Icon;
        self
    }
    pub fn mono(mut self) -> Self {
        self.font = Font::Mono;
        self
    }
    fn key(&self) -> u64 {
        ((self.size * 4.0) as u64)
            | ((self.weight as u64) << 16)
            | ((self.font as u64) << 32)
            | ((self.align as u64) << 36)
            | ((self.wrap as u64) << 40)
    }
}

pub struct Gfx {
    factory: ID2D1Factory1,
    dw: IDWriteFactory,
    dc: ID2D1DeviceContext,
    swap: IDXGISwapChain1,
    dxgi: IDXGIDevice,
    d2d_device: ID2D1Device,
    _dcomp: IDCompositionDevice,
    _target: IDCompositionTarget,
    _visual: IDCompositionVisual,
    brush: ID2D1SolidColorBrush,
    /// Second brush, used to colour a highlighted run inside a line of text.
    hl: ID2D1SolidColorBrush,
    cap: ID2D1StrokeStyle,
    formats: HashMap<u64, IDWriteTextFormat>,
    pub w: u32,
    pub h: u32,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

fn v2(x: f32, y: f32) -> Vector2 {
    Vector2 { X: x, Y: y }
}

impl Gfx {
    pub fn new(hwnd: HWND, w: u32, h: u32, software: bool) -> Result<Gfx> {
        unsafe {
            let mut device = None;
            let driver = if software { D3D_DRIVER_TYPE_WARP } else { D3D_DRIVER_TYPE_HARDWARE };
            D3D11CreateDevice(
                None,
                driver,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                None,
            )?;
            let device: ID3D11Device = device.unwrap();
            let dxgi: IDXGIDevice = device.cast()?;
            let factory: ID2D1Factory1 = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let d2d_device = factory.CreateDevice(&dxgi)?;
            let dw: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            Self::build(factory, dw, dxgi, d2d_device, hwnd, w, h)
        }
    }

    /// A renderer for another window that shares this one's graphics device,
    /// so a second window costs almost no extra memory.
    pub fn sibling(&self, hwnd: HWND, w: u32, h: u32) -> Result<Gfx> {
        unsafe { Self::build(self.factory.clone(), self.dw.clone(), self.dxgi.clone(), self.d2d_device.clone(), hwnd, w, h) }
    }

    unsafe fn build(
        factory: ID2D1Factory1,
        dw: IDWriteFactory,
        dxgi: IDXGIDevice,
        d2d_device: ID2D1Device,
        hwnd: HWND,
        w: u32,
        h: u32,
    ) -> Result<Gfx> {
        let fac: IDXGIFactory2 = CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0))?;
        let desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: w.max(1),
            Height: h.max(1),
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            Scaling: DXGI_SCALING_STRETCH,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
            AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
            ..Default::default()
        };
        let swap = fac.CreateSwapChainForComposition(&dxgi, &desc, None)?;
        let dc = d2d_device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
        dc.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);

        let dcomp: IDCompositionDevice = DCompositionCreateDevice(&dxgi)?;
        let target = dcomp.CreateTargetForHwnd(hwnd, true)?;
        let visual = dcomp.CreateVisual()?;
        visual.SetContent(&swap)?;
        target.SetRoot(&visual)?;
        dcomp.Commit()?;

        let brush = dc.CreateSolidColorBrush(&Color::rgb(0xffffff).d2d(), None)?;
        let hl = dc.CreateSolidColorBrush(&Color::rgb(0xffffff).d2d(), None)?;
        let cap = factory.cast::<ID2D1Factory>()?.CreateStrokeStyle(
            &D2D1_STROKE_STYLE_PROPERTIES {
                startCap: D2D1_CAP_STYLE_ROUND,
                endCap: D2D1_CAP_STYLE_ROUND,
                dashCap: D2D1_CAP_STYLE_ROUND,
                lineJoin: D2D1_LINE_JOIN_ROUND,
                miterLimit: 1.0,
                dashStyle: D2D1_DASH_STYLE_SOLID,
                dashOffset: 0.0,
            },
            None,
        )?;

        let g = Gfx {
            factory,
            dw,
            dc,
            swap,
            dxgi,
            d2d_device,
            _dcomp: dcomp,
            _target: target,
            _visual: visual,
            brush,
            hl,
            cap,
            formats: HashMap::new(),
            w: w.max(1),
            h: h.max(1),
        };
        g.bind()?;
        Ok(g)
    }

    /// Shifts the drawn content vertically by `px` physical pixels (open animation).
    pub fn set_offset(&self, px: f32) {
        unsafe {
            let _ = self._visual.SetOffsetY2(px);
            let _ = self._dcomp.Commit();
        }
    }

    fn bind(&self) -> Result<()> {
        unsafe {
            let surface: IDXGISurface = self.swap.GetBuffer(0)?;
            let props = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 96.0,
                dpiY: 96.0,
                bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                colorContext: ManuallyDrop::new(None),
            };
            let bmp = self.dc.CreateBitmapFromDxgiSurface(&surface, Some(&props))?;
            self.dc.SetTarget(&bmp);
            Ok(())
        }
    }

    pub fn resize(&mut self, w: u32, h: u32) {
        let (w, h) = (w.max(1), h.max(1));
        if w == self.w && h == self.h {
            return;
        }
        unsafe {
            self.dc.SetTarget(None);
            if self.swap.ResizeBuffers(0, w, h, DXGI_FORMAT_UNKNOWN, DXGI_SWAP_CHAIN_FLAG(0)).is_ok() {
                self.w = w;
                self.h = h;
            }
            let _ = self.bind();
        }
    }

    /// One layout unit equals `scale` physical pixels.
    pub fn set_scale(&mut self, scale: f32) {
        unsafe { self.dc.SetDpi(96.0 * scale, 96.0 * scale) }
    }

    /// Release driver-side caches while the menu is hidden.
    pub fn trim(&self) {
        unsafe {
            if let Ok(d3) = self.dxgi.cast::<IDXGIDevice3>() {
                d3.Trim();
            }
        }
    }

    pub fn begin(&mut self) {
        unsafe {
            self.dc.BeginDraw();
            self.dc.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }));
        }
    }

    pub fn end(&mut self) {
        unsafe {
            let _ = self.dc.EndDraw(None, None);
            let _ = self.swap.Present(1, DXGI_PRESENT(0));
        }
    }

    fn set(&self, c: Color) {
        unsafe { self.brush.SetColor(&c.d2d()) }
    }

    pub fn fill_rect(&mut self, r: Rect, c: Color) {
        self.set(c);
        unsafe { self.dc.FillRectangle(&r.d2d(), &self.brush) }
    }

    pub fn fill_rr(&mut self, r: Rect, radius: f32, c: Color) {
        self.set(c);
        let rr = D2D1_ROUNDED_RECT { rect: r.d2d(), radiusX: radius, radiusY: radius };
        unsafe { self.dc.FillRoundedRectangle(&rr, &self.brush) }
    }

    pub fn stroke_rr(&mut self, r: Rect, radius: f32, c: Color, width: f32) {
        self.set(c);
        let rr = D2D1_ROUNDED_RECT { rect: r.inset(width / 2.0).d2d(), radiusX: radius, radiusY: radius };
        unsafe { self.dc.DrawRoundedRectangle(&rr, &self.brush, width, None) }
    }

    pub fn fill_circle(&mut self, cx: f32, cy: f32, r: f32, c: Color) {
        self.set(c);
        let e = D2D1_ELLIPSE { point: v2(cx, cy), radiusX: r, radiusY: r };
        unsafe { self.dc.FillEllipse(&e, &self.brush) }
    }

    pub fn stroke_circle(&mut self, cx: f32, cy: f32, r: f32, c: Color, width: f32) {
        self.set(c);
        let e = D2D1_ELLIPSE { point: v2(cx, cy), radiusX: r, radiusY: r };
        unsafe { self.dc.DrawEllipse(&e, &self.brush, width, None) }
    }

    pub fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, c: Color, width: f32) {
        self.set(c);
        unsafe { self.dc.DrawLine(v2(x0, y0), v2(x1, y1), &self.brush, width, &self.cap) }
    }

    /// Arc starting at 12 o'clock, sweeping clockwise through `frac` of a turn.
    pub fn arc(&mut self, cx: f32, cy: f32, r: f32, frac: f32, c: Color, width: f32) {
        let frac = frac.clamp(0.0, 1.0);
        if frac <= 0.001 {
            return;
        }
        if frac >= 0.999 {
            self.stroke_circle(cx, cy, r, c, width);
            return;
        }
        let ang = frac * std::f32::consts::TAU;
        let end = v2(cx + r * ang.sin(), cy - r * ang.cos());
        self.set(c);
        unsafe {
            let Ok(geo) = self.factory.CreatePathGeometry() else { return };
            let Ok(sink) = geo.Open() else { return };
            sink.BeginFigure(v2(cx, cy - r), D2D1_FIGURE_BEGIN_HOLLOW);
            sink.AddArc(&D2D1_ARC_SEGMENT {
                point: end,
                size: D2D_SIZE_F { width: r, height: r },
                rotationAngle: 0.0,
                sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                arcSize: if frac > 0.5 { D2D1_ARC_SIZE_LARGE } else { D2D1_ARC_SIZE_SMALL },
            });
            sink.EndFigure(D2D1_FIGURE_END_OPEN);
            let _ = sink.Close();
            self.dc.DrawGeometry(&geo, &self.brush, width, &self.cap);
        }
    }

    fn format(&mut self, st: Ts) -> Option<IDWriteTextFormat> {
        let key = st.key();
        if let Some(f) = self.formats.get(&key) {
            return Some(f.clone());
        }
        unsafe {
            let family = match st.font {
                Font::Ui => w!("Segoe UI Variable Text"),
                Font::Icon => w!("Segoe Fluent Icons"),
                Font::Mono => w!("Cascadia Mono"),
            };
            let f = self
                .dw
                .CreateTextFormat(
                    family,
                    None,
                    DWRITE_FONT_WEIGHT(st.weight as i32),
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    st.size,
                    w!(""),
                )
                .ok()?;
            let _ = f.SetTextAlignment(match st.align {
                Align::Left => DWRITE_TEXT_ALIGNMENT_LEADING,
                Align::Center => DWRITE_TEXT_ALIGNMENT_CENTER,
                Align::Right => DWRITE_TEXT_ALIGNMENT_TRAILING,
            });
            if st.wrap {
                let _ = f.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP);
                let _ = f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR);
            } else {
                let _ = f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);
                let _ = f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
                if st.font != Font::Icon {
                    if let Ok(sign) = self.dw.CreateEllipsisTrimmingSign(&f) {
                        let trim = DWRITE_TRIMMING {
                            granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                            delimiter: 0,
                            delimiterCount: 0,
                        };
                        let _ = f.SetTrimming(&trim, &sign);
                    }
                }
            }
            self.formats.insert(key, f.clone());
            Some(f)
        }
    }

    pub fn text(&mut self, s: &str, r: Rect, st: Ts, c: Color) {
        if s.is_empty() || r.w <= 0.0 {
            return;
        }
        let Some(f) = self.format(st) else { return };
        self.set(c);
        let ws = wide(s);
        unsafe {
            self.dc.DrawText(
                &ws,
                &f,
                &r.d2d(),
                &self.brush,
                D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT | D2D1_DRAW_TEXT_OPTIONS_CLIP,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
    }

    /// Single-line text with the UTF-16 range `(start, len)` drawn bold in `hc`.
    pub fn text_hl(&mut self, s: &str, r: Rect, st: Ts, c: Color, range: (u32, u32), hc: Color) {
        if range.1 == 0 {
            self.text(s, r, st, c);
            return;
        }
        let Some(l) = self.layout(s, st, r.w, r.h) else { return };
        unsafe {
            self.hl.SetColor(&hc.d2d());
            let tr = DWRITE_TEXT_RANGE { startPosition: range.0, length: range.1 };
            let _ = l.SetDrawingEffect(&self.hl, tr);
            let _ = l.SetFontWeight(DWRITE_FONT_WEIGHT(700), tr);
        }
        self.draw_layout(r.x, r.y, &l, c);
    }

    pub fn layout(&mut self, s: &str, st: Ts, max_w: f32, max_h: f32) -> Option<IDWriteTextLayout> {
        let f = self.format(st)?;
        let ws = wide(s);
        unsafe { self.dw.CreateTextLayout(&ws, &f, max_w.max(1.0), max_h.max(1.0)).ok() }
    }

    /// Returns (width, height) of the laid out text.
    pub fn measure(&mut self, s: &str, st: Ts, max_w: f32) -> (f32, f32) {
        let st = Ts { align: Align::Left, ..st };
        let Some(l) = self.layout(s, st, max_w, 4000.0) else { return (0.0, 0.0) };
        let mut m = DWRITE_TEXT_METRICS::default();
        unsafe {
            let _ = l.GetMetrics(&mut m);
        }
        (m.widthIncludingTrailingWhitespace, m.height)
    }

    pub fn draw_layout(&mut self, x: f32, y: f32, l: &IDWriteTextLayout, c: Color) {
        self.set(c);
        unsafe {
            self.dc.DrawTextLayout(v2(x, y), l, &self.brush, D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT);
        }
    }

    /// Position of the caret before UTF-16 offset `pos`: (x, y, height).
    pub fn caret_pos(l: &IDWriteTextLayout, pos: u32) -> (f32, f32, f32) {
        let (mut x, mut y) = (0.0f32, 0.0f32);
        let mut m = DWRITE_HIT_TEST_METRICS::default();
        unsafe {
            let _ = l.HitTestTextPosition(pos, false, &mut x, &mut y, &mut m);
        }
        (x, y, m.height)
    }

    pub fn push_clip(&mut self, r: Rect) {
        unsafe { self.dc.PushAxisAlignedClip(&r.d2d(), D2D1_ANTIALIAS_MODE_ALIASED) }
    }

    pub fn pop_clip(&mut self) {
        unsafe { self.dc.PopAxisAlignedClip() }
    }

    pub fn bitmap_from_bgra(&mut self, w: u32, h: u32, data: &[u8]) -> Option<ID2D1Bitmap1> {
        if w == 0 || h == 0 || data.len() < (w * h * 4) as usize {
            return None;
        }
        let props = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: D2D1_BITMAP_OPTIONS_NONE,
            colorContext: ManuallyDrop::new(None),
        };
        unsafe {
            self.dc
                .CreateBitmap(D2D_SIZE_U { width: w, height: h }, Some(data.as_ptr() as *const _), w * 4, &props)
                .ok()
        }
    }

    pub fn load_image(&mut self, path: &str) -> Option<ID2D1Bitmap1> {
        unsafe {
            let wic: IWICImagingFactory =
                CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
            let dec = wic
                .CreateDecoderFromFilename(&HSTRING::from(path), None, GENERIC_READ, WICDecodeMetadataCacheOnDemand)
                .ok()?;
            let frame = dec.GetFrame(0).ok()?;
            let conv = wic.CreateFormatConverter().ok()?;
            conv.Initialize(
                &frame,
                &GUID_WICPixelFormat32bppPBGRA,
                WICBitmapDitherTypeNone,
                None,
                0.0,
                WICBitmapPaletteTypeMedianCut,
            )
            .ok()?;
            self.dc.CreateBitmapFromWicBitmap(&conv, None).ok()
        }
    }

    pub fn draw_bitmap(&mut self, bmp: &ID2D1Bitmap1, r: Rect, opacity: f32) {
        unsafe {
            self.dc.DrawBitmap(
                bmp,
                Some(&r.d2d()),
                opacity,
                D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
                None,
                None,
            );
        }
    }

    /// Draws the bitmap cropped to a circle (used for the account picture).
    pub fn draw_bitmap_circle(&mut self, bmp: &ID2D1Bitmap1, cx: f32, cy: f32, r: f32) {
        unsafe {
            let size = bmp.GetSize();
            if size.width <= 0.0 || size.height <= 0.0 {
                return;
            }
            let Ok(brush) = self.dc.CreateBitmapBrush(bmp, None, None) else { return };
            let s = (2.0 * r) / size.width.min(size.height);
            let m = Matrix3x2 {
                M11: s,
                M12: 0.0,
                M21: 0.0,
                M22: s,
                M31: cx - size.width * s / 2.0,
                M32: cy - size.height * s / 2.0,
            };
            brush.SetTransform(&m);
            let e = D2D1_ELLIPSE { point: v2(cx, cy), radiusX: r, radiusY: r };
            self.dc.FillEllipse(&e, &brush);
        }
    }
}
