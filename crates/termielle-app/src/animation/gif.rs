//! Streaming GIF decode through Windows Imaging Component.
//!
//! Only the current frame is decoded, into one composited BGRA canvas plus one
//! scratch canvas that exists for `previous` disposal. Frames are never
//! pre-decoded, and the file bytes stay owned for the lifetime of the decoder
//! because `IWICStream::InitializeFromMemory` does not copy its buffer.
//!
//! COM is initialized per `open`/`Drop` pair with apartment threading, so all
//! animation operations must stay on the thread that opened the animation;
//! [`GifAnimation`] is deliberately not `Send` or `Sync`.

use std::marker::PhantomData;
use std::path::Path;

use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPBGRA, IWICBitmapDecoder,
    IWICBitmapFrameDecode, IWICFormatConverter, IWICImagingFactory, IWICMetadataQueryReader,
    WICBitmapDitherTypeNone, WICBitmapPaletteTypeCustom, WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoUninitialize,
};
use windows::Win32::System::Variant::{VT_UI1, VT_UI2};
use windows::core::{PCWSTR, w};

/// How a frame's rectangle is cleared after it is displayed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Disposal {
    None,
    Keep,
    Background,
    Previous,
}

/// The region a frame occupied on the logical screen.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Rect {
    left: u32,
    top: u32,
    width: u32,
    height: u32,
}

/// Why a frame could not be decoded.
///
/// Variants carry no path or payload, so the overlay can log the numeric code
/// without leaking anything about the animation that failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum AnimationError {
    #[error("could not read the animation file")]
    ReadFailed,
    #[error("the animation contains no frames")]
    Empty,
    #[error("the animation is not a supported image")]
    Unsupported,
    #[error("image decoding failed with code {0}")]
    Win32(u32),
}

/// One composited frame in premultiplied BGRA, ready for the overlay.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrameBuffer {
    pub width: u32,
    pub height: u32,
    pub pixels_pbgra: Vec<u8>,
    pub delay_ms: u32,
    pub loop_index: u64,
}

impl FrameBuffer {
    /// The alpha byte of one pixel; out-of-bounds pixels are transparent.
    pub fn alpha_at(&self, x: u32, y: u32) -> u8 {
        if x >= self.width || y >= self.height {
            return 0;
        }
        self.pixels_pbgra[((y * self.width + x) * 4 + 3) as usize]
    }

    /// Whether any pixel exactly equals `pixel` in BGRA order.
    pub fn contains_pixel_bgra(&self, pixel: [u8; 4]) -> bool {
        self.pixels_pbgra
            .chunks_exact(4)
            .any(|chunk| chunk == pixel)
    }
}

/// A frame source: a streaming GIF or a single procedural still.
pub enum AnimationSource {
    Gif(GifAnimation),
    Still(FrameBuffer),
}

/// A streaming GIF decoder bound to one thread.
///
/// Field order matters: `_com` is declared last so it drops last, guaranteeing
/// `CoUninitialize` runs only after every WIC interface in this struct has been
/// released. Uninitializing the apartment while COM objects are still alive is
/// undefined behavior and crashes in practice.
pub struct GifAnimation {
    /// Owned so `IWICStream::InitializeFromMemory` has a live buffer.
    _bytes: Vec<u8>,
    factory: IWICImagingFactory,
    decoder: IWICBitmapDecoder,
    canvas: FrameBuffer,
    scratch: FrameBuffer,
    frame_index: u32,
    frame_count: u32,
    loop_index: u64,
    previous_rect: Option<Rect>,
    previous_disposal: Disposal,
    /// COM objects are apartment-bound; this struct may not cross threads.
    _thread: PhantomData<*const ()>,
    /// Keeps the apartment initialized for as long as the COM objects live.
    _com: ComInitialized,
}

impl GifAnimation {
    /// Decodes the animation at `path` and prepares the first frame.
    pub fn open(path: &Path) -> Result<Self, AnimationError> {
        let com = ComInitialized::initialize()?;
        let bytes = std::fs::read(path).map_err(|_| AnimationError::ReadFailed)?;

        let factory: IWICImagingFactory =
            unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER) }
                .map_err(win32)?;

        let stream = unsafe { factory.CreateStream() }.map_err(win32)?;
        unsafe { stream.InitializeFromMemory(&bytes) }.map_err(win32)?;

        let decoder = unsafe {
            factory.CreateDecoderFromStream(
                &stream,
                std::ptr::null(),
                WICDecodeMetadataCacheOnDemand,
            )
        }
        .map_err(win32)?;

        let frame_count = unsafe { decoder.GetFrameCount() }.map_err(win32)?;
        if frame_count == 0 {
            return Err(AnimationError::Empty);
        }

        let (width, height) = logical_screen_size(&decoder)?;
        let blank = |delay_ms| FrameBuffer {
            width,
            height,
            pixels_pbgra: vec![0u8; (width * height * 4) as usize],
            delay_ms,
            loop_index: 0,
        };

        Ok(Self {
            _bytes: bytes,
            factory,
            decoder,
            canvas: blank(0),
            scratch: blank(0),
            frame_index: 0,
            frame_count,
            loop_index: 0,
            previous_rect: None,
            previous_disposal: Disposal::None,
            _thread: PhantomData,
            _com: com,
        })
    }

    /// Decodes the next frame into the reused canvas and returns it.
    ///
    /// The returned reference is the same buffer every call; the overlay must
    /// copy it before the next call if it needs to keep the pixels.
    pub fn next_frame(&mut self) -> Result<&FrameBuffer, AnimationError> {
        if self.frame_index >= self.frame_count {
            self.frame_index = 0;
            self.loop_index += 1;
        }

        // The previous frame's disposal applies before the next frame draws.
        if let Some(rect) = self.previous_rect {
            match self.previous_disposal {
                Disposal::Background => clear_rect(&mut self.canvas, rect),
                Disposal::Previous => self
                    .canvas
                    .pixels_pbgra
                    .copy_from_slice(&self.scratch.pixels_pbgra),
                Disposal::None | Disposal::Keep => {}
            }
        }

        let frame = unsafe { self.decoder.GetFrame(self.frame_index) }.map_err(win32)?;
        let (width, height) = frame_size(&frame)?;
        let (left, top, delay_hundredths, disposal) = frame_metadata(&frame)?;

        if disposal == Disposal::Previous {
            self.scratch
                .pixels_pbgra
                .copy_from_slice(&self.canvas.pixels_pbgra);
        }

        let converted = self.convert(&frame)?;
        composite(&mut self.canvas, &converted, left, top);

        self.canvas.delay_ms = clamp_delay(delay_hundredths);
        self.canvas.loop_index = self.loop_index;

        self.previous_rect = Some(Rect {
            left,
            top,
            width,
            height,
        });
        self.previous_disposal = disposal;
        self.frame_index += 1;

        Ok(&self.canvas)
    }

    /// Restarts the animation at loop zero with a cleared canvas.
    pub fn reset(&mut self) -> Result<(), AnimationError> {
        self.frame_index = 0;
        self.loop_index = 0;
        self.previous_rect = None;
        self.previous_disposal = Disposal::None;
        self.canvas.pixels_pbgra.fill(0);
        self.scratch.pixels_pbgra.fill(0);
        Ok(())
    }

    /// Bytes held by the composited canvas.
    pub fn canvas_bytes(&self) -> usize {
        self.canvas.pixels_pbgra.len()
    }

    /// Bytes held by the previous-disposal scratch canvas.
    pub fn scratch_bytes(&self) -> usize {
        self.scratch.pixels_pbgra.len()
    }

    /// Converts one decoded frame to premultiplied BGRA at its native size.
    fn convert(&self, frame: &IWICBitmapFrameDecode) -> Result<FrameBuffer, AnimationError> {
        let converter: IWICFormatConverter =
            unsafe { self.factory.CreateFormatConverter() }.map_err(win32)?;
        unsafe {
            converter.Initialize(
                frame,
                &GUID_WICPixelFormat32bppPBGRA,
                WICBitmapDitherTypeNone,
                None,
                0.0,
                WICBitmapPaletteTypeCustom,
            )
        }
        .map_err(win32)?;

        let mut width = 0;
        let mut height = 0;
        unsafe { converter.GetSize(&mut width, &mut height) }.map_err(win32)?;

        let stride = width * 4;
        let mut pixels = vec![0u8; (stride * height) as usize];
        unsafe { converter.CopyPixels(std::ptr::null(), stride, &mut pixels) }.map_err(win32)?;

        Ok(FrameBuffer {
            width,
            height,
            pixels_pbgra: pixels,
            delay_ms: 0,
            loop_index: 0,
        })
    }
}

/// RAII for one `CoInitializeEx`/`CoUninitialize` pair.
struct ComInitialized;

impl ComInitialized {
    fn initialize() -> Result<Self, AnimationError> {
        // SAFETY: standard apartment init with no reserved pointer.
        let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        hr.ok()
            .map_err(|error| AnimationError::Win32(win32_code(&error)))?;
        Ok(Self)
    }
}

impl Drop for ComInitialized {
    fn drop(&mut self) {
        // SAFETY: balances the matching `CoInitializeEx` from `initialize`.
        unsafe { CoUninitialize() };
    }
}

/// The logical screen size from GIF metadata, falling back to the first
/// frame's dimensions when the metadata is absent.
fn logical_screen_size(decoder: &IWICBitmapDecoder) -> Result<(u32, u32), AnimationError> {
    if let Ok(reader) = unsafe { decoder.GetMetadataQueryReader() } {
        if let (Some(width), Some(height)) = (
            prop_u16(&reader, w!("/logscrdesc/Width")),
            prop_u16(&reader, w!("/logscrdesc/Height")),
        ) {
            if width > 0 && height > 0 {
                return Ok((width as u32, height as u32));
            }
        }
    }

    let first = unsafe { decoder.GetFrame(0) }.map_err(win32)?;
    let (width, height) = frame_size(&first)?;
    if width == 0 || height == 0 {
        return Err(AnimationError::Unsupported);
    }
    Ok((width, height))
}

/// The decoded size of one frame.
fn frame_size(frame: &IWICBitmapFrameDecode) -> Result<(u32, u32), AnimationError> {
    let mut width = 0;
    let mut height = 0;
    unsafe { frame.GetSize(&mut width, &mut height) }.map_err(win32)?;
    Ok((width, height))
}

/// Reads a frame's offset, delay, and disposal metadata, defaulting every
/// missing property to its neutral value.
fn frame_metadata(
    frame: &IWICBitmapFrameDecode,
) -> Result<(u32, u32, u32, Disposal), AnimationError> {
    let query = unsafe { frame.GetMetadataQueryReader() };
    let Ok(reader) = query else {
        return Ok((0, 0, 0, Disposal::Keep));
    };

    let left = prop_u16(&reader, w!("/imgdesc/Left")).unwrap_or(0) as u32;
    let top = prop_u16(&reader, w!("/imgdesc/Top")).unwrap_or(0) as u32;
    let delay_hundredths = prop_u16(&reader, w!("/grctlext/Delay")).unwrap_or(0) as u32;
    let disposal = match prop_u8(&reader, w!("/grctlext/Disposal")).unwrap_or(0) {
        2 => Disposal::Background,
        3 => Disposal::Previous,
        _ => Disposal::Keep,
    };

    Ok((left, top, delay_hundredths, disposal))
}

/// Reads a `VT_UI2` metadata property, or `None` when absent or mistyped.
fn prop_u16(reader: &IWICMetadataQueryReader, name: PCWSTR) -> Option<u16> {
    let mut value = PROPVARIANT::default();
    // SAFETY: `value` is a live out-parameter exactly as long as `PROPVARIANT`,
    // and the union reads below only happen after `vt` confirms the member.
    unsafe {
        if reader.GetMetadataByName(name, &mut value).is_err() {
            return None;
        }
        if value.Anonymous.Anonymous.vt != VT_UI2 {
            return None;
        }
        Some(value.Anonymous.Anonymous.Anonymous.uiVal)
    }
}

/// Reads a `VT_UI1` metadata property, or `None` when absent or mistyped.
fn prop_u8(reader: &IWICMetadataQueryReader, name: PCWSTR) -> Option<u8> {
    let mut value = PROPVARIANT::default();
    // SAFETY: as `prop_u16`: the out-parameter is live and `vt` gates the read.
    unsafe {
        if reader.GetMetadataByName(name, &mut value).is_err() {
            return None;
        }
        if value.Anonymous.Anonymous.vt != VT_UI1 {
            return None;
        }
        Some(value.Anonymous.Anonymous.Anonymous.bVal)
    }
}

/// Delays are GIF hundredths of a second; missing or zero delays clamp to 20 ms.
fn clamp_delay(delay_hundredths: u32) -> u32 {
    if delay_hundredths == 0 {
        20
    } else {
        delay_hundredths * 10
    }
}

/// Draws one premultiplied-BGRA frame over the canvas at its offset.
fn composite(canvas: &mut FrameBuffer, frame: &FrameBuffer, left: u32, top: u32) {
    for y in 0..frame.height {
        for x in 0..frame.width {
            let cx = left + x;
            let cy = top + y;
            if cx >= canvas.width || cy >= canvas.height {
                continue;
            }
            let source = ((y * frame.width + x) * 4) as usize;
            let dest = ((cy * canvas.width + cx) * 4) as usize;
            let alpha = frame.pixels_pbgra[source + 3] as u32;
            let inverse = 255 - alpha;
            canvas.pixels_pbgra[dest] = (frame.pixels_pbgra[source] as u32
                + canvas.pixels_pbgra[dest] as u32 * inverse / 255)
                as u8;
            canvas.pixels_pbgra[dest + 1] = (frame.pixels_pbgra[source + 1] as u32
                + canvas.pixels_pbgra[dest + 1] as u32 * inverse / 255)
                as u8;
            canvas.pixels_pbgra[dest + 2] = (frame.pixels_pbgra[source + 2] as u32
                + canvas.pixels_pbgra[dest + 2] as u32 * inverse / 255)
                as u8;
            canvas.pixels_pbgra[dest + 3] =
                (alpha + canvas.pixels_pbgra[dest + 3] as u32 * inverse / 255) as u8;
        }
    }
}

/// Zeroes one rectangle of the canvas.
fn clear_rect(canvas: &mut FrameBuffer, rect: Rect) {
    for y in rect.top..(rect.top + rect.height).min(canvas.height) {
        for x in rect.left..(rect.left + rect.width).min(canvas.width) {
            let dest = ((y * canvas.width + x) * 4) as usize;
            canvas.pixels_pbgra[dest..dest + 4].fill(0);
        }
    }
}

/// Extracts the numeric Win32 HRESULT from a `windows` error.
fn win32_code(error: &windows::core::Error) -> u32 {
    error.code().0 as u32
}

fn win32(error: windows::core::Error) -> AnimationError {
    AnimationError::Win32(win32_code(&error))
}
