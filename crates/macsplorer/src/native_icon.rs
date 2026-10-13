//! The icon macOS shows for an item, for things that carry their own icon:
//! app bundles, other packages and anything with a Finder custom icon.

use std::path::Path;

use eframe::egui::ColorImage;

/// Renders the Finder icon for `path` as a `size`×`size` image.
#[cfg(target_os = "macos")]
pub fn render(path: &Path, size: usize) -> Option<ColorImage> {
    use objc2::AnyThread;
    use objc2_app_kit::{NSBitmapImageRep, NSCompositingOperation, NSDeviceRGBColorSpace, NSGraphicsContext, NSWorkspace};
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

    let path = NSString::from_str(path.to_str()?);
    let image = NSWorkspace::sharedWorkspace().iconForFile(&path);
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
    let whole = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(0.0, 0.0));
    image.drawInRect_fromRect_operation_fraction(
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(s, s)),
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
