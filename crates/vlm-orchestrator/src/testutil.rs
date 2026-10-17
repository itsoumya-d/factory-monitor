//! Test-only helpers shared across module test suites.
#![allow(dead_code)]

use std::io::Cursor;

/// Synthesize a real JPEG of the requested dimensions so payload-cap and
/// EOD-framing tests exercise genuine header parsing.
pub(crate) fn jpeg_bytes(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbImage::from_fn(w, h, |x, y| {
        image::Rgb([(x % 251) as u8, (y % 241) as u8, ((x + y) % 199) as u8])
    });
    let mut out = Vec::new();
    image::ImageBuffer::write_to(&img, &mut Cursor::new(&mut out), image::ImageFormat::Jpeg)
        .unwrap_or_else(|_| panic!("test jpeg encode failed"));
    out
}
