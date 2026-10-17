//! The icon macOS shows for an item, for things that carry their own icon:
//! app bundles, other packages and anything with a Finder custom icon; and
//! SF Symbols, for the glyphs Finder puts on its special folders.

use std::path::Path;

use eframe::egui::ColorImage;

/// Renders the Finder icon for `path` as a `size`×`size` image.
#[cfg(target_os = "macos")]
pub fn render(path: &Path, size: usize) -> Option<ColorImage> {
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::NSString;

    let path = NSString::from_str(path.to_str()?);
    draw(&NSWorkspace::sharedWorkspace().iconForFile(&path), size)
}

/// Renders the icon Finder shows for a folder customized with `style`: an
/// empty folder in the temporary directory gets that style and macOS draws
/// it, so the preview is Finder's own drawing.
#[cfg(target_os = "macos")]
pub fn render_folder_preview(style: &macsplorer_core::folder_style::FolderStyle, size: usize) -> Option<ColorImage> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    // A new path each time, so nothing cached for an earlier style comes back.
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("macsplorer-folder-preview-{}-{n}", std::process::id()));
    std::fs::create_dir(&dir).ok()?;
    let image = macsplorer_core::folder_style::write(&dir, style).ok().and_then(|()| render(&dir, size));
    let _ = std::fs::remove_dir(&dir);
    image
}

/// Renders Finder's folder in `style`'s color, with its emblem when
/// `show_emblem`. Finder only colors a folder that has an emblem, so a blank
/// emoji stands in when there is none to show. A color of Macsplorer's own
/// is Finder's Gray folder recolored: with an emblem, the pixels that differ
/// between a Gray and a Blue drawing are the folder's, and the rest (an
/// emoji) keeps its colors.
#[cfg(target_os = "macos")]
pub fn render_folder(style: &macsplorer_core::folder_style::FolderStyle, show_emblem: bool, size: usize) -> Option<ColorImage> {
    use eframe::egui::Color32;
    use macsplorer_core::folder_style::{Emblem, FolderColor, FolderStyle, TagColor, tint};

    let emblem = style.emblem.clone().filter(|_| show_emblem);
    let shown = emblem.clone().unwrap_or_else(|| Emblem::Emoji(" ".into()));
    let draw = |color: Option<TagColor>| render_folder_preview(&FolderStyle { color: color.map(FolderColor::Tag), emblem: Some(shown.clone()) }, size);
    let Some(FolderColor::Custom(rgb)) = style.color else {
        return draw(style.color.map(FolderColor::tag));
    };
    let gray = draw(Some(TagColor::Gray))?;
    let other = if emblem.is_some() { Some(draw(Some(TagColor::Blue))?) } else { None };
    let [w, h] = gray.size;
    // The folder's front, below where an emblem goes.
    let [r, g, b, _] = gray.pixels[(h * 4 / 5) * w + w / 2].to_srgba_unmultiplied();
    let body = ((r as u16 + g as u16 + b as u16) / 3) as u8;
    let differs = |a: Color32, b: Color32| (0..4).map(|i| (a[i] as i32 - b[i] as i32).abs()).sum::<i32>() > 24;
    let pixels = gray
        .pixels
        .iter()
        .enumerate()
        .map(|(i, &p)| {
            if other.as_ref().is_some_and(|o| !differs(o.pixels[i], p)) {
                return p;
            }
            let [r, g, b, a] = tint(p.to_srgba_unmultiplied(), body, rgb);
            Color32::from_rgba_unmultiplied(r, g, b, a)
        })
        .collect();
    Some(ColorImage { pixels, ..gray })
}

/// True if this macOS has an SF Symbol called `name`.
#[cfg(target_os = "macos")]
pub fn has_symbol(name: &str) -> bool {
    use objc2_app_kit::NSImage;
    use objc2_foundation::NSString;
    NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(name), None).is_some()
}

/// Renders an SF Symbol as white-on-transparent, fitted and centered in a
/// `size`×`size` image, so the caller can tint it. `None` if this macOS
/// doesn't have the symbol.
#[cfg(target_os = "macos")]
pub fn render_symbol(name: &str, size: usize) -> Option<ColorImage> {
    use objc2_app_kit::NSImage;
    use objc2_foundation::NSString;

    let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(name), None)?;
    let image = draw(&image, size)?;
    // Keep only the shape: symbols draw black, the tint comes from the caller.
    let pixels = image.pixels.iter().map(|p| eframe::egui::Color32::from_white_alpha(p.a())).collect();
    Some(ColorImage { source_size: image.source_size, size: image.size, pixels })
}

/// Draws `image` scaled to fit, centered, in a `size`×`size` bitmap.
#[cfg(target_os = "macos")]
fn draw(image: &objc2_app_kit::NSImage, size: usize) -> Option<ColorImage> {
    use objc2::AnyThread;
    use objc2_app_kit::{NSBitmapImageRep, NSCompositingOperation, NSDeviceRGBColorSpace, NSGraphicsContext};
    use objc2_foundation::{NSPoint, NSRect, NSSize};

    let px = size as isize;
    // SAFETY: a null planes pointer makes AppKit allocate the buffer; the
    // format is 8-bit RGBA, non-planar, with alpha.
    let rep = unsafe {
        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(),
            std::ptr::null_mut(),
            px,
            px,
            8,
            4,
            true,
            false,
            NSDeviceRGBColorSpace,
            0,
            32,
        )
    }?;
    let ctx = NSGraphicsContext::graphicsContextWithBitmapImageRep(&rep)?;
    NSGraphicsContext::saveGraphicsState_class();
    NSGraphicsContext::setCurrentContext(Some(&ctx));
    let s = size as f64;
    let natural = image.size();
    let fit = (s / natural.width).min(s / natural.height);
    let (w, h) = (natural.width * fit, natural.height * fit);
    let whole = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(0.0, 0.0));
    image.drawInRect_fromRect_operation_fraction(
        NSRect::new(NSPoint::new((s - w) / 2.0, (s - h) / 2.0), NSSize::new(w, h)),
        whole,
        NSCompositingOperation::Copy,
        1.0,
    );
    ctx.flushGraphics();
    NSGraphicsContext::restoreGraphicsState_class();

    let stride = rep.bytesPerRow() as usize;
    let data = rep.bitmapData();
    if data.is_null() {
        return None;
    }
    let mut rgba = Vec::with_capacity(size * size * 4);
    for row in 0..size {
        // SAFETY: the rep owns `stride * size` bytes and outlives this read.
        let line = unsafe { std::slice::from_raw_parts(data.add(row * stride), size * 4) };
        rgba.extend_from_slice(line);
    }
    // NSBitmapImageRep stores premultiplied alpha by default.
    Some(ColorImage::from_rgba_premultiplied([size, size], &rgba))
}

#[cfg(not(target_os = "macos"))]
pub fn render(_path: &Path, _size: usize) -> Option<ColorImage> {
    None
}

#[cfg(not(target_os = "macos"))]
pub fn render_folder(_style: &macsplorer_core::folder_style::FolderStyle, _show_emblem: bool, _size: usize) -> Option<ColorImage> {
    None
}

#[cfg(not(target_os = "macos"))]
pub fn render_symbol(_name: &str, _size: usize) -> Option<ColorImage> {
    None
}
