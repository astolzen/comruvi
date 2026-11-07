//! Extract and flatten embedded JPEG XMP (APP1 packet + optional `.jpg.xmp` sidecar)
//! for Full meta, translators (`xmp = "…"`), and volltext search.

use std::{
    collections::BTreeMap,
    fs::File,
    io::{BufReader, Read, Seek, SeekFrom},
    path::Path,
};

use anyhow::Result;
use quick_xml::escape::unescape;
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use quick_xml::XmlVersion;

const XMP_APP1_HEADER: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";
/// Safety cap while scanning markers before SOS (XMP never needs the pixel stream).
const MAX_HEADER_SCAN: u64 = 4 * 1024 * 1024;

/// Load flattened XMP properties from the JPEG APP1 packet and/or a `.jpg.xmp` sidecar.
pub fn load_xmp_map(path: &Path) -> Result<BTreeMap<String, String>> {
    let mut merged = BTreeMap::new();
    if let Ok(file) = File::open(path) {
        let mut reader = BufReader::with_capacity(64 * 1024, file);
        if let Some(xml) = extract_xmp_packet_reader(&mut reader) {
            for (k, v) in flatten_xmp_rdf(&xml) {
                merged.insert(k, v);
            }
        }
    }
    // Many tools (Digikam, Lightroom, …) also write `photo.jpg.xmp` beside the image.
    let sidecar = Path::new(&format!("{}.xmp", path.display())).to_path_buf();
    if sidecar.is_file() {
        if let Ok(xml) = std::fs::read_to_string(&sidecar) {
            for (k, v) in flatten_xmp_rdf(&xml) {
                merged.entry(k).or_insert(v);
            }
        }
    }
    Ok(merged)
}

/// Find the Adobe XMP packet inside a JPEG by streaming markers (no full-file read).
pub fn extract_xmp_packet_reader<R: Read + Seek>(reader: &mut R) -> Option<String> {
    let mut soi = [0u8; 2];
    reader.read_exact(&mut soi).ok()?;
    if soi != [0xFF, 0xD8] {
        return None;
    }
    let start = reader.stream_position().ok()?;
    loop {
        let pos = reader.stream_position().ok()?;
        if pos.saturating_sub(start) > MAX_HEADER_SCAN {
            return None;
        }
        // Skip fill bytes (0xFF padding).
        let mut b = [0u8; 1];
        loop {
            reader.read_exact(&mut b).ok()?;
            if b[0] != 0xFF {
                continue;
            }
            // Collapse runs of 0xFF, then read marker.
            loop {
                reader.read_exact(&mut b).ok()?;
                if b[0] != 0xFF {
                    break;
                }
            }
            break;
        }
        let marker = b[0];
        if matches!(marker, 0xD0..=0xD9 | 0x01) {
            continue;
        }
        // Standalone markers without length — rare before SOS; stop at SOS/EOI.
        if marker == 0xDA || marker == 0xD9 {
            return None;
        }
        let mut len_buf = [0u8; 2];
        reader.read_exact(&mut len_buf).ok()?;
        let len = u16::from_be_bytes(len_buf) as u64;
        if len < 2 {
            return None;
        }
        let payload_len = (len - 2) as usize;
        if marker == 0xE1 {
            // Only buffer APP1 payloads (XMP lives here); skip others cheaply.
            if payload_len >= XMP_APP1_HEADER.len() && payload_len <= 65533 {
                let mut payload = vec![0u8; payload_len];
                reader.read_exact(&mut payload).ok()?;
                if payload.starts_with(XMP_APP1_HEADER) {
                    let xml = &payload[XMP_APP1_HEADER.len()..];
                    return std::str::from_utf8(xml).ok().map(|s| s.to_string());
                }
                continue;
            }
        }
        // Skip segment body.
        reader.seek(SeekFrom::Current((len - 2) as i64)).ok()?;
    }
}

/// Find the Adobe XMP packet inside a JPEG buffer (tests / callers with bytes).
#[cfg_attr(not(test), allow(dead_code))]
pub fn extract_xmp_packet(jpeg: &[u8]) -> Option<String> {
    use std::io::Cursor;
    extract_xmp_packet_reader(&mut Cursor::new(jpeg))
}

/// Flatten RDF/XML XMP into `prefix:LocalName` → human string (arrays joined with ", ").
pub fn flatten_xmp_rdf(xml: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    // Stack of open element display keys (prefix:local).
    let mut stack: Vec<String> = Vec::new();
    let mut text_buf = String::new();
    let mut list_items: Option<Vec<String>> = None;
    let mut list_owner: Option<String> = None;

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                // Compact XMP often stores simple props as attributes on rdf:Description.
                for attr in e.attributes().flatten() {
                    let key = attr.key.as_ref().to_string();
                    if key.starts_with("xmlns") || key == "about" || key.ends_with(":about") {
                        continue;
                    }
                    if let Ok(value) = attr.normalized_value(XmlVersion::Implicit1_0) {
                        let value = value.trim();
                        if !value.is_empty() {
                            out.entry(key).or_insert_with(|| value.to_string());
                        }
                    }
                }
                let key = e.name().as_ref().to_string();
                let local = local_of(&key);
                if matches!(local, "Bag" | "Seq" | "Alt") {
                    list_items = Some(Vec::new());
                    list_owner = stack.last().cloned();
                }
                stack.push(key);
                text_buf.clear();
            }
            Ok(Event::Empty(e)) => {
                for attr in e.attributes().flatten() {
                    let key = attr.key.as_ref().to_string();
                    if key.starts_with("xmlns") || key == "about" || key.ends_with(":about") {
                        continue;
                    }
                    if let Ok(value) = attr.normalized_value(XmlVersion::Implicit1_0) {
                        let value = value.trim();
                        if !value.is_empty() {
                            out.entry(key).or_insert_with(|| value.to_string());
                        }
                    }
                }
            }
            Ok(Event::Text(t)) => {
                let raw = t.as_ref();
                if let Ok(text) = unescape(raw) {
                    text_buf.push_str(&text);
                } else {
                    text_buf.push_str(raw);
                }
            }
            Ok(Event::CData(t)) => {
                text_buf.push_str(t.as_ref());
            }
            Ok(Event::End(_)) => {
                let Some(key) = stack.pop() else {
                    continue;
                };
                let local = local_of(&key);
                let text = text_buf.trim().to_string();
                text_buf.clear();

                if local == "li" {
                    if let Some(items) = list_items.as_mut() {
                        if !text.is_empty() {
                            items.push(text);
                        }
                    }
                } else if matches!(local, "Bag" | "Seq" | "Alt") {
                    if let (Some(owner), Some(items)) = (list_owner.take(), list_items.take()) {
                        if !items.is_empty() {
                            out.insert(owner, items.join(", "));
                        }
                    }
                } else if !text.is_empty()
                    && !matches!(
                        local,
                        "RDF" | "Description" | "xmpmeta" | "Bag" | "Seq" | "Alt" | "li"
                    )
                {
                    out.entry(key).or_insert(text);
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }

    out.retain(|k, v| {
        let local = local_of(k);
        !matches!(local, "RDF" | "Description" | "xmpmeta") && !v.trim().is_empty()
    });
    out
}

fn local_of(key: &str) -> &str {
    key.rsplit_once(':').map(|(_, l)| l).unwrap_or(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flattens_digikam_tagslist() {
        let xml = r#"<?xml version="1.0"?>
        <x:xmpmeta xmlns:x="adobe:ns:meta/">
          <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
            <rdf:Description xmlns:digiKam="http://www.digikam.org/ns/1.0/"
                             xmlns:mediapro="http://ns.iview-multimedia.com/mediapro/1.0/"
                             xmlns:dc="http://purl.org/dc/elements/1.1/">
              <digiKam:TagsList>
                <rdf:Seq>
                  <rdf:li>Boots</rdf:li>
                  <rdf:li>Bondage/Rope</rdf:li>
                </rdf:Seq>
              </digiKam:TagsList>
              <mediapro:CatalogSets>
                <rdf:Bag>
                  <rdf:li>Boots</rdf:li>
                  <rdf:li>Bondage|Rope</rdf:li>
                </rdf:Bag>
              </mediapro:CatalogSets>
              <dc:subject>
                <rdf:Bag>
                  <rdf:li>Boots</rdf:li>
                  <rdf:li>Rope</rdf:li>
                </rdf:Bag>
              </dc:subject>
              <xmp:Rating xmlns:xmp="http://ns.adobe.com/xap/1.0/">3</xmp:Rating>
            </rdf:Description>
          </rdf:RDF>
        </x:xmpmeta>"#;
        let map = flatten_xmp_rdf(xml);
        assert_eq!(
            map.get("digiKam:TagsList").map(String::as_str),
            Some("Boots, Bondage/Rope")
        );
        assert_eq!(
            map.get("mediapro:CatalogSets").map(String::as_str),
            Some("Boots, Bondage|Rope")
        );
        assert_eq!(map.get("dc:subject").map(String::as_str), Some("Boots, Rope"));
        assert_eq!(map.get("xmp:Rating").map(String::as_str), Some("3"));
    }

    #[test]
    fn flattens_description_attributes() {
        let xml = r#"<?xml version="1.0"?>
        <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"
                 xmlns:xmp="http://ns.adobe.com/xap/1.0/">
          <rdf:Description rdf:about="" xmp:Rating="4" xmp:Label="Red"/>
        </rdf:RDF>"#;
        let map = flatten_xmp_rdf(xml);
        assert_eq!(map.get("xmp:Rating").map(String::as_str), Some("4"));
        assert_eq!(map.get("xmp:Label").map(String::as_str), Some("Red"));
    }

    #[test]
    fn reads_sample_when_present() {
        let path = Path::new(r"F:\sync\Pictures\test\DSCF0221.JPG");
        if !path.is_file() {
            return;
        }
        let map = load_xmp_map(path).unwrap();
        assert!(
            map.keys()
                .any(|k| k.contains("TagsList") || k.contains("CatalogSets") || k.contains("subject")),
            "expected XMP keyword properties, got keys: {:?}",
            map.keys().collect::<Vec<_>>()
        );
    }
}
