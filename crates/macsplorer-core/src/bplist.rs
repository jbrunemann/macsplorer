//! The little of Apple's binary property list format (`bplist00`) that
//! Finder's attributes need: a top object that is a string or an array of
//! strings. Strings are ASCII (marker `0x5n`) or UTF-16 big endian (`0x6n`);
//! lengths of 15 and more follow the marker as an integer object.

/// The strings of a binary plist whose top object is a string (one) or an
/// array of strings. `None` if it isn't one of those.
pub fn strings(data: &[u8]) -> Option<Vec<String>> {
    if data.len() < 8 + 32 || &data[..8] != b"bplist00" {
        return None;
    }
    let trailer = &data[data.len() - 32..];
    let offset_size = trailer[6] as usize;
    let ref_size = trailer[7] as usize;
    let top = u64::from_be_bytes(trailer[16..24].try_into().ok()?) as usize;
    let table = u64::from_be_bytes(trailer[24..32].try_into().ok()?) as usize;
    let offset = |object: usize| data.get(table + object * offset_size..table + (object + 1) * offset_size).map(be);
    let at = offset(top)?;
    match data.get(at)? >> 4 {
        0xa => {
            let (count, start) = length(data, at)?;
            let refs = data.get(start..start + count * ref_size)?;
            refs.chunks(ref_size.max(1)).map(|r| string(data, offset(be(r))?)).collect()
        }
        _ => Some(vec![string(data, at)?]),
    }
}

/// A binary plist whose top object is an array of `items`.
pub fn encode_strings(items: &[String]) -> Vec<u8> {
    let ref_size = if items.len() < 0xff { 1 } else { 2 };
    let mut out = b"bplist00".to_vec();
    let mut offsets = vec![out.len()];
    marker(&mut out, 0xa, items.len());
    for n in 1..=items.len() {
        out.extend_from_slice(&n.to_be_bytes()[8 - ref_size..]);
    }
    for item in items {
        offsets.push(out.len());
        if item.is_ascii() {
            marker(&mut out, 0x5, item.len());
            out.extend_from_slice(item.as_bytes());
        } else {
            let units: Vec<u16> = item.encode_utf16().collect();
            marker(&mut out, 0x6, units.len());
            out.extend(units.iter().flat_map(|u| u.to_be_bytes()));
        }
    }
    let table = out.len();
    let offset_size = match table {
        0..0x100 => 1,
        0x100..0x1_0000 => 2,
        _ => 4,
    };
    for o in &offsets {
        out.extend_from_slice(&o.to_be_bytes()[8 - offset_size..]);
    }
    // Trailer: 6 unused bytes, the two sizes, then the object count, the top
    // object and where the offset table starts, 8 bytes each.
    out.extend_from_slice(&[0; 6]);
    out.extend_from_slice(&[offset_size as u8, ref_size as u8]);
    out.extend_from_slice(&(offsets.len() as u64).to_be_bytes());
    out.extend_from_slice(&0u64.to_be_bytes());
    out.extend_from_slice(&(table as u64).to_be_bytes());
    out
}

/// The string object at `at`.
fn string(data: &[u8], at: usize) -> Option<String> {
    let (len, start) = length(data, at)?;
    match data.get(at)? >> 4 {
        0x5 => data.get(start..start + len).map(|b| String::from_utf8_lossy(b).into_owned()),
        0x6 => {
            let units: Vec<u16> = data.get(start..start + 2 * len)?.as_chunks::<2>().0.iter().map(|&c| u16::from_be_bytes(c)).collect();
            Some(String::from_utf16_lossy(&units))
        }
        _ => None,
    }
}

/// The length of the object at `at` and where its contents start.
fn length(data: &[u8], at: usize) -> Option<(usize, usize)> {
    let (len, start) = ((*data.get(at)? & 0x0f) as usize, at + 1);
    if len < 0x0f {
        return Some((len, start));
    }
    // The length follows as an integer object: 0x1n, then 2^n bytes.
    let bytes = 1usize << (*data.get(start)? & 0x0f);
    Some((be(data.get(start + 1..start + 1 + bytes)?), start + 1 + bytes))
}

/// Writes an object marker with its length, adding the integer object for 15 and more.
fn marker(out: &mut Vec<u8>, kind: u8, len: usize) {
    if len < 0x0f {
        out.push(kind << 4 | len as u8);
        return;
    }
    out.push(kind << 4 | 0x0f);
    let size = if len < 0x100 { 0 } else if len < 0x1_0000 { 1 } else { 2 };
    out.push(0x10 | size);
    out.extend_from_slice(&(len as u64).to_be_bytes()[8 - (1 << size)..]);
}

fn be(bytes: &[u8]) -> usize {
    bytes.iter().fold(0usize, |n, &b| n << 8 | b as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tags attribute of a folder Finder tagged Red.
    const FINDER_RED: &str = "62706c6973743030a101555265640a36080a0000000000000101000000000000000200000000000000000000000000000010";

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    #[test]
    fn reads_and_writes_finders_bytes() {
        let data = hex(FINDER_RED);
        assert_eq!(strings(&data), Some(vec!["Red\n6".to_owned()]));
        assert_eq!(encode_strings(&["Red\n6".to_owned()]), data);
    }

    #[test]
    fn round_trips_long_and_unicode_strings() {
        let items: Vec<String> = ["", "Work", "a tag longer than fifteen\n4", "Rötlich\n6", "🚀"].map(str::to_owned).into();
        assert_eq!(strings(&encode_strings(&items)), Some(items));
        let many: Vec<String> = (0..300).map(|n| format!("tag {n} {}", "x".repeat(n))).collect();
        assert_eq!(strings(&encode_strings(&many)), Some(many));
        assert_eq!(strings(&encode_strings(&[])), Some(vec![]));
    }

    #[test]
    fn rejects_other_data() {
        assert_eq!(strings(b"not a plist"), None);
        assert_eq!(strings(&hex(FINDER_RED)[..30]), None);
    }

    /// `plutil` reads what we write.
    #[cfg(target_os = "macos")]
    #[test]
    fn plutil_accepts_it() {
        let path = std::env::temp_dir().join(format!("macsplorer-bplist-{}.plist", std::process::id()));
        std::fs::write(&path, encode_strings(&["Red\n6".into(), "Grün\n2".into(), "a tag longer than fifteen".into()])).unwrap();
        let out = std::process::Command::new("plutil").args(["-convert", "json", "-o", "-"]).arg(&path).output().unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(String::from_utf8(out.stdout).unwrap().trim(), r#"["Red\n6","Grün\n2","a tag longer than fifteen"]"#);
    }
}
