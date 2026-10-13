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
pub fn render_symbol(_name: &str, _size: usize) -> Option<ColorImage> {
    None
}
