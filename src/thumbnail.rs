//! Decodes downloaded chat images once, off the UI thread, at display size. GPUI would otherwise
//! keep every image (and every GIF frame) at full resolution for the life of the process.
use gpui_kit::RenderImage;
use image::{
    AnimationDecoder, Frame, ImageFormat, ImageReader, Limits, RgbaImage,
    codecs::gif::GifDecoder,
    imageops::{FilterType, resize},
};
use std::{io::Cursor, sync::Arc};

/// Longest side in device pixels: a 480-point bubble on a Retina display.
const MAX_SIDE: u32 = 960;
/// Decoded budget per animation; larger GIFs show their first frame only.
const MAX_ANIMATION_BYTES: usize = 12 * 1024 * 1024;
/// Stop decoding source frames past this (decompression-bomb guard).
const MAX_SOURCE_BYTES: usize = 128 * 1024 * 1024;

/// BGRA frames sized for display, plus their decoded byte size.
pub(crate) fn decode(mime: &str, bytes: &[u8]) -> Option<(Arc<RenderImage>, usize)> {
    let format = ImageFormat::from_mime_type(mime)?;
    let mut frames: Vec<Frame> = if format == ImageFormat::Gif {
        let mut frames = Vec::new();
        let mut decoded = 0;
        for frame in GifDecoder::new(Cursor::new(bytes)).ok()?.into_frames() {
            let Ok(frame) = frame else { break };
            decoded += frame.buffer().len();
            frames.push(frame);
            if decoded > MAX_SOURCE_BYTES {
                frames.truncate(1);
                break;
            }
        }
        frames
    } else {
        let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
        let mut limits = Limits::default();
        limits.max_alloc = Some(MAX_SOURCE_BYTES as u64);
        reader.limits(limits);
        vec![Frame::new(reader.decode().ok()?.into_rgba8())]
    };
    let (width, height) = frames.first()?.buffer().dimensions();
    if width == 0 || height == 0 {
        return None;
    }
    let scale = (MAX_SIDE as f32 / width.max(height) as f32).min(1.0);
    let (width, height) = (
        ((width as f32 * scale).round() as u32).max(1),
        ((height as f32 * scale).round() as u32).max(1),
    );
    let frame_bytes = width as usize * height as usize * 4;
    if frames.len() * frame_bytes > MAX_ANIMATION_BYTES {
        frames.truncate(1);
    }
    let frames: Vec<Frame> = frames
        .into_iter()
        .map(|frame| {
            let delay = frame.delay();
            let mut buffer: RgbaImage = if scale < 1.0 {
                resize(frame.buffer(), width, height, FilterType::Triangle)
            } else {
                frame.into_buffer()
            };
            // GPUI renders BGRA.
            for pixel in buffer.as_chunks_mut::<4>().0 {
                pixel.swap(0, 2);
            }
            Frame::from_parts(buffer, 0, 0, delay)
        })
        .collect();
    let size = frames.len() * frame_bytes;
    Some((Arc::new(RenderImage::new(frames)), size))
}
