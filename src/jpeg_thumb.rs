//! Fast JPEG thumbnail helpers: prefer the embedded Exif thumbnail, else a cheap downscale.

use std::{fs::File, io::Read, path::Path};

use anyhow::{Context, Result};
use image::DynamicImage;

/// Max bytes to scan for an Exif-embedded JPEG thumbnail (never the full photo).
const HEADER_CAP: usize = 256 * 1024;

/// Load a small preview for `path`.
///
/// For JPEGs this prefers the camera's embedded Exif thumbnail (usually ≤160px, milliseconds).
/// Falls back to a full decode + fast `thumbnail()` resize.
pub fn load_preview(path: &Path, max_side: u32, quarters: u8) -> Result<DynamicImage> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if matches!(ext.as_str(), "jpg" | "jpeg") {
        if let Some(thumb) = exif_jpeg_thumbnail(path) {
            return Ok(apply_quarters(thumb, quarters));
        }
    }
    let img = image::open(path).with_context(|| format!("opening {}", path.display()))?;
    let img = apply_quarters(img, quarters);
    Ok(img.thumbnail(max_side, max_side))
}

fn apply_quarters(img: DynamicImage, quarters: u8) -> DynamicImage {
    use image::imageops;
    match quarters % 4 {
        1 => DynamicImage::ImageRgba8(imageops::rotate90(&img)),
        2 => DynamicImage::ImageRgba8(imageops::rotate180(&img)),
        3 => DynamicImage::ImageRgba8(imageops::rotate270(&img)),
        _ => img,
    }
}

/// Extract the JPEG bitstream stored in Exif IFD1 (`JPEGInterchangeFormat`), if any.
fn exif_jpeg_thumbnail(path: &Path) -> Option<DynamicImage> {
    let mut file = File::open(path).ok()?;
    let mut buf = vec![0u8; HEADER_CAP];
    let n = file.read(&mut buf).ok()?;
    buf.truncate(n);
    let jpeg = find_exif_thumbnail_jpeg(&buf)?;
    image::load_from_memory(jpeg).ok()
}

/// Locate Exif APP1 → TIFF → IFD1 JPEGInterchangeFormat payload (starts with SOI).
fn find_exif_thumbnail_jpeg(jpeg: &[u8]) -> Option<&[u8]> {
    if jpeg.len() < 4 || jpeg[0] != 0xFF || jpeg[1] != 0xD8 {
        return None;
    }
    let mut i = 2usize;
    while i + 4 < jpeg.len() {
        if jpeg[i] != 0xFF {
            i += 1;
            continue;
        }
        while i < jpeg.len() && jpeg[i] == 0xFF {
            i += 1;
        }
        if i >= jpeg.len() {
            break;
        }
        let marker = jpeg[i];
        i += 1;
        if matches!(marker, 0xD0..=0xD9 | 0x01) {
            continue;
        }
        if marker == 0xDA {
            break;
        }
        if i + 2 > jpeg.len() {
            break;
        }
        let len = u16::from_be_bytes([jpeg[i], jpeg[i + 1]]) as usize;
        if len < 2 || i + len > jpeg.len() {
            break;
        }
        let payload = &jpeg[i + 2..i + len];
        i += len;
        // Exif APP1: "Exif\0\0" + TIFF
        if marker == 0xE1 && payload.starts_with(b"Exif\0\0") {
            if let Some(thumb) = tiff_ifd1_jpeg_thumb(&payload[6..]) {
                return Some(thumb);
            }
        }
    }
    None
}

fn tiff_ifd1_jpeg_thumb(tiff: &[u8]) -> Option<&[u8]> {
    if tiff.len() < 8 {
        return None;
    }
    let le = match &tiff[0..2] {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let read_u16 = |o: usize| -> Option<u16> {
        let b = tiff.get(o..o + 2)?;
        Some(if le {
            u16::from_le_bytes([b[0], b[1]])
        } else {
            u16::from_be_bytes([b[0], b[1]])
        })
    };
    let read_u32 = |o: usize| -> Option<u32> {
        let b = tiff.get(o..o + 4)?;
        Some(if le {
            u32::from_le_bytes([b[0], b[1], b[2], b[3]])
        } else {
            u32::from_be_bytes([b[0], b[1], b[2], b[3]])
        })
    };
    // IFD0 offset
    let mut ifd = read_u32(4)? as usize;
    // Walk IFD0 to get pointer to IFD1 (next IFD after IFD0 entries).
    let count0 = read_u16(ifd)? as usize;
    let next_ifd_off = ifd + 2 + count0 * 12;
    let ifd1 = read_u32(next_ifd_off)? as usize;
    if ifd1 == 0 || ifd1 + 2 > tiff.len() {
        return None;
    }
    ifd = ifd1;
    let count = read_u16(ifd)? as usize;
    let mut jpeg_off: Option<u32> = None;
    let mut jpeg_len: Option<u32> = None;
    for e in 0..count {
        let base = ifd + 2 + e * 12;
        let tag = read_u16(base)?;
        // let typ = read_u16(base + 2)?;
        // let _cnt = read_u32(base + 4)?;
        let val = read_u32(base + 8)?;
        match tag {
            0x0201 => jpeg_off = Some(val), // JPEGInterchangeFormat
            0x0202 => jpeg_len = Some(val), // JPEGInterchangeFormatLength
            _ => {}
        }
    }
    let off = jpeg_off? as usize;
    let len = jpeg_len? as usize;
    if off == 0 || len < 4 || off.checked_add(len)? > tiff.len() {
        return None;
    }
    let slice = &tiff[off..off + len];
    if slice[0] == 0xFF && slice[1] == 0xD8 {
        Some(slice)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_canon_exif_thumb_when_present() {
        let path = Path::new(r"F:\sync\Pictures\test\IMG_3383.JPG");
        if !path.is_file() {
            return;
        }
        let thumb = exif_jpeg_thumbnail(path).expect("Canon JPG should embed an Exif thumbnail");
        assert!(thumb.width() <= 640 && thumb.height() <= 640, "{}x{}", thumb.width(), thumb.height());
    }
}
