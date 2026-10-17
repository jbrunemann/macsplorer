//! MIME types for `prepare-upload`'s `fileType`, from the file name. The
//! receiving app uses them to show a preview and to put images and videos
//! in the gallery, so the common media types matter most; anything else is
//! `application/octet-stream`.

pub const UNKNOWN: &str = "application/octet-stream";

const TYPES: &[(&str, &str)] = &[
    ("7z", "application/x-7z-compressed"),
    ("aac", "audio/aac"),
    ("apk", "application/vnd.android.package-archive"),
    ("avi", "video/x-msvideo"),
    ("bmp", "image/bmp"),
    ("csv", "text/csv"),
    ("doc", "application/msword"),
    ("docx", "application/vnd.openxmlformats-officedocument.wordprocessingml.document"),
    ("epub", "application/epub+zip"),
    ("flac", "audio/flac"),
    ("gif", "image/gif"),
    ("gz", "application/gzip"),
    ("heic", "image/heic"),
    ("heif", "image/heif"),
    ("htm", "text/html"),
    ("html", "text/html"),
    ("jpeg", "image/jpeg"),
    ("jpg", "image/jpeg"),
    ("js", "text/javascript"),
    ("json", "application/json"),
    ("m4a", "audio/mp4"),
    ("m4v", "video/mp4"),
    ("md", "text/markdown"),
    ("mkv", "video/x-matroska"),
    ("mov", "video/quicktime"),
    ("mp3", "audio/mpeg"),
    ("mp4", "video/mp4"),
    ("ogg", "audio/ogg"),
    ("pdf", "application/pdf"),
    ("png", "image/png"),
    ("ppt", "application/vnd.ms-powerpoint"),
    ("pptx", "application/vnd.openxmlformats-officedocument.presentationml.presentation"),
    ("rar", "application/vnd.rar"),
    ("rtf", "application/rtf"),
    ("svg", "image/svg+xml"),
    ("tar", "application/x-tar"),
    ("tif", "image/tiff"),
    ("tiff", "image/tiff"),
    ("txt", "text/plain"),
    ("wav", "audio/wav"),
    ("webm", "video/webm"),
    ("webp", "image/webp"),
    ("xls", "application/vnd.ms-excel"),
    ("xlsx", "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"),
    ("xml", "application/xml"),
    ("zip", "application/zip"),
];

/// The MIME type of a file called `name`, by its extension.
pub fn of(name: &str) -> &'static str {
    let Some((_, ext)) = name.rsplit_once('.') else { return UNKNOWN };
    let ext = ext.to_ascii_lowercase();
    TYPES.binary_search_by(|(e, _)| e.cmp(&ext.as_str())).map_or(UNKNOWN, |i| TYPES[i].1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_sorted() {
        assert!(TYPES.windows(2).all(|w| w[0].0 < w[1].0));
    }

    #[test]
    fn types_by_extension() {
        assert_eq!(of("IMG_0001.JPG"), "image/jpeg");
        assert_eq!(of("notes.txt"), "text/plain");
        assert_eq!(of("Makefile"), UNKNOWN);
        assert_eq!(of("archive.unknown"), UNKNOWN);
    }
}
