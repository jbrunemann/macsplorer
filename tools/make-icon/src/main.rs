//! Renders the macsplorer app icon: a macOS (Big Sur style) squircle holding
//! a Windows 2000 window and a pixel-art Windows 2000 folder.
//!
//! `cargo run -p make-icon` writes `assets/icon.png` (1024²), a full
//! `.iconset`, and `assets/macsplorer.icns` (via `iconutil`).

use std::path::Path;
use std::process::Command;

use tiny_skia::{
    Color, FillRule, GradientStop, LinearGradient, Mask, Paint, Path as SkPath, PathBuilder, Pixmap, Point, Rect,
    SpreadMode, Stroke, Transform,
};

/// Every coordinate below is on Apple's 1024×1024 icon grid.
const GRID: f32 = 1024.0;
/// The squircle body: 824 px square, centered (Apple's macOS 11+ template).
const BODY_MIN: f32 = 100.0;
const BODY_SIZE: f32 = 824.0;

fn rgb(hex: u32) -> Color {
    rgba(hex, 255)
}

fn rgba(hex: u32, a: u8) -> Color {
    Color::from_rgba8((hex >> 16) as u8, (hex >> 8) as u8, hex as u8, a)
}

fn solid(c: Color) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color(c);
    p.anti_alias = true;
    p
}

fn vgradient(y0: f32, y1: f32, stops: &[(f32, Color)]) -> Paint<'static> {
    hgradient_points(Point::from_xy(0.0, y0), Point::from_xy(0.0, y1), stops)
}

fn hgradient_points(a: Point, b: Point, stops: &[(f32, Color)]) -> Paint<'static> {
    let stops = stops.iter().map(|&(p, c)| GradientStop::new(p, c)).collect();
    Paint {
        shader: LinearGradient::new(a, b, stops, SpreadMode::Pad, Transform::identity()).unwrap(),
        anti_alias: true,
        ..Default::default()
    }
}

/// Apple's icon shape: straight sides with continuous-curvature corners.
/// Each corner is a superellipse quadrant (|x|^n + |y|^n = 1), which blends
/// into the straight edge without the visible kink of a circular arc.
fn squircle(x: f32, y: f32, size: f32) -> SkPath {
    let n = 4.0f32;
    // Corner extent ~29% of the side: matches the macOS 11+ template, whose
    // smoothed corners start well before a 22.5% circular radius would.
    let r = size * 0.29;
    let corners = [
        (x + size - r, y + size - r, 0.0f32), // bottom-right, sweep 0..90°
        (x + r, y + size - r, 90.0),
        (x + r, y + r, 180.0),
        (x + size - r, y + r, 270.0),
    ];
    let mut pb = PathBuilder::new();
    let steps = 90;
    for (k, &(cx, cy, start)) in corners.iter().enumerate() {
        for i in 0..=steps {
            let t = (start + 90.0 * i as f32 / steps as f32).to_radians();
            let (c, s) = (t.cos(), t.sin());
            let px = cx + r * c.signum() * c.abs().powf(2.0 / n);
            let py = cy + r * s.signum() * s.abs().powf(2.0 / n);
            if k == 0 && i == 0 { pb.move_to(px, py) } else { pb.line_to(px, py) }
        }
    }
    pb.close();
    pb.finish().unwrap()
}

fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
    Rect::from_ltrb(x0, y0, x1, y1).unwrap()
}

struct Canvas {
    pixmap: Pixmap,
    t: Transform,
}

impl Canvas {
    fn new(size: u32) -> Self {
        let s = size as f32 / GRID;
        Self { pixmap: Pixmap::new(size, size).unwrap(), t: Transform::from_scale(s, s) }
    }

    fn rect(&mut self, r: Rect, paint: &Paint, mask: Option<&Mask>) {
        self.pixmap.fill_rect(r, paint, self.t, mask);
    }

    fn path(&mut self, p: &SkPath, paint: &Paint, mask: Option<&Mask>) {
        self.pixmap.fill_path(p, paint, FillRule::Winding, self.t, mask);
    }

    fn stroke(&mut self, p: &SkPath, paint: &Paint, width: f32) {
        let stroke = Stroke { width, ..Stroke::default() };
        self.pixmap.stroke_path(p, paint, &stroke, self.t, None);
    }

    fn mask(&self, p: &SkPath) -> Mask {
        let mut m = Mask::new(self.pixmap.width(), self.pixmap.height()).unwrap();
        m.fill_path(p, FillRule::Winding, true, self.t);
        m
    }

    /// A soft shadow faked with stacked translucent copies (tiny-skia has
    /// no blur). `spread` is how far it fades out, `dy` how far it drops.
    fn soft_shadow(&mut self, shape: impl Fn(f32) -> SkPath, spread: f32, dy: f32, alpha: f32) {
        let layers = 16;
        for i in 0..layers {
            let grow = spread * (1.0 - i as f32 / layers as f32);
            let a = (alpha / layers as f32 * 255.0) as u8;
            let p = shape(grow).transform(Transform::from_translate(0.0, dy)).unwrap();
            self.path(&p, &solid(rgba(0x000000, a)), None);
        }
    }

    /// The Windows Classic 3D edge, scaled up: `w` is the width of one of
    /// the original 1-pixel lines.
    fn bevel(&mut self, r: Rect, w: f32, sunken: bool) {
        let (otl, obr, itl, ibr) = if sunken {
            (0x808080, 0xFFFFFF, 0x404040, 0xD4D0C8)
        } else {
            (0xFFFFFF, 0x404040, 0xD4D0C8, 0x808080)
        };
        self.edge(r, w, otl, obr);
        self.edge(rect(r.left() + w, r.top() + w, r.right() - w, r.bottom() - w), w, itl, ibr);
    }

    fn edge(&mut self, r: Rect, w: f32, tl: u32, br: u32) {
        let (l, t, rr, b) = (r.left(), r.top(), r.right(), r.bottom());
        self.rect(rect(l, t, rr, t + w), &solid(rgb(tl)), None);
        self.rect(rect(l, t, l + w, b), &solid(rgb(tl)), None);
        self.rect(rect(l, b - w, rr, b), &solid(rgb(br)), None);
        self.rect(rect(rr - w, t, rr, b), &solid(rgb(br)), None);
    }
}

/// 16×16 pixel-art folder in the Windows 2000 palette.
/// K outline, Y yellow, L highlight, S shade, F front flap, D drop shadow.
const FOLDER: [&str; 16] = [
    "................",
    "................",
    ".KKKKK..........",
    "KLLLLLK.........",
    "KLYYYYYKKKKKKK..",
    "KLYYYYYYYYYYYYK.",
    "KLYKKKKKKKKKKKKK",
    "KLKFFFFFFFFFFFFK",
    "KLKFLLLLLLLLLLFK",
    "KKFLYYYYYYYYYSKD",
    "KKFLYYYYYYYYYSKD",
    "KFLYYYYYYYYYYSKD",
    "KFLYYYYYYYYYSSKD",
    "KFSSSSSSSSSSSSKD",
    "KKKKKKKKKKKKKKKD",
    ".DDDDDDDDDDDDDDD",
];

fn folder_color(c: char) -> Option<Color> {
    Some(match c {
        'K' => rgb(0x6B5410),
        'Y' => rgb(0xF6D35E),
        'L' => rgb(0xFFF3B8),
        'S' => rgb(0xD9A82E),
        'F' => rgb(0xFFE38A),
        'D' => rgba(0x000000, 70),
        _ => return None,
    })
}

fn draw(size: u32) -> Pixmap {
    let mut c = Canvas::new(size);
    let body = |grow: f32| squircle(BODY_MIN - grow, BODY_MIN - grow, BODY_SIZE + 2.0 * grow);

    // macOS: drop shadow under the body.
    // Mostly downward, like the system icon shadow.
    c.soft_shadow(body, 10.0, 12.0, 0.30);
    c.soft_shadow(body, 4.0, 4.0, 0.20);

    // macOS: the body, Windows 2000 desktop blue with a soft vertical gradient.
    let body_path = body(0.0);
    let clip = c.mask(&body_path);
    c.path(&body_path, &vgradient(BODY_MIN, BODY_MIN + BODY_SIZE, &[(0.0, rgb(0x5B8FCB)), (1.0, rgb(0x2A5A8E))]), None);

    // Windows 2000: a window, slightly tilted toward the viewer's left.
    let (wx0, wy0, wx1, wy1) = (176.0, 214.0, 772.0, 704.0);
    c.soft_shadow(
        |g| {
            let mut pb = PathBuilder::new();
            pb.push_rect(rect(wx0 - g, wy0 - g, wx1 + g, wy1 + g));
            pb.finish().unwrap()
        },
        18.0,
        16.0,
        0.45,
    );
    c.rect(rect(wx0, wy0, wx1, wy1), &solid(rgb(0xD4D0C8)), None);
    let px = 7.0; // one original pixel
    c.bevel(rect(wx0, wy0, wx1, wy1), px, false);

    // Caption: navy → light blue gradient with caption buttons.
    let (cx0, cy0, cx1, cy1) = (wx0 + 3.0 * px, wy0 + 3.0 * px, wx1 - 3.0 * px, wy0 + 3.0 * px + 76.0);
    c.rect(rect(cx0, cy0, cx1, cy1), &hgradient_points(Point::from_xy(cx0, 0.0), Point::from_xy(cx1, 0.0), &[(0.0, rgb(0x0A246A)), (1.0, rgb(0xA6CAF0))]), None);
    let bw = 54.0;
    let bh = 48.0;
    let by = cy0 + (cy1 - cy0 - bh) / 2.0;
    let close_x = cx1 - 10.0 - bw;
    for (i, x) in [close_x - 2.0 * bw - 10.0, close_x - bw - 10.0, close_x].into_iter().enumerate() {
        let r = rect(x, by, x + bw, by + bh);
        c.rect(r, &solid(rgb(0xD4D0C8)), None);
        c.bevel(r, 4.0, false);
        let ink = solid(rgb(0x000000));
        let g = rect(x + 14.0, by + 11.0, x + bw - 14.0, by + bh - 13.0);
        match i {
            0 => c.rect(rect(g.left(), g.bottom() - 7.0, g.left() + 18.0, g.bottom()), &ink, None),
            1 => {
                let mut pb = PathBuilder::new();
                pb.push_rect(g);
                c.stroke(&pb.finish().unwrap(), &ink, 4.0);
                c.rect(rect(g.left(), g.top(), g.right(), g.top() + 6.0), &ink, None);
            }
            _ => {
                let mut pb = PathBuilder::new();
                pb.move_to(g.left() + 2.0, g.top());
                pb.line_to(g.right() - 2.0, g.bottom());
                pb.move_to(g.right() - 2.0, g.top());
                pb.line_to(g.left() + 2.0, g.bottom());
                c.stroke(&pb.finish().unwrap(), &ink, 7.0);
            }
        }
    }
    // A tiny folder in the caption, like Explorer's window icon.
    pixel_art(&mut c, cx0 + 12.0, cy0 + 10.0, 3.6);

    // Client area: a sunken white details list.
    let (lx0, ly0, lx1, ly1) = (cx0, cy1 + 14.0, cx1, wy1 - 3.0 * px);
    c.rect(rect(lx0, ly0, lx1, ly1), &solid(rgb(0xFFFFFF)), None);
    c.bevel(rect(lx0, ly0, lx1, ly1), 5.0, true);
    // Column header.
    let hy1 = ly0 + 10.0 + 34.0;
    for (x0, x1) in [(lx0 + 10.0, lx0 + 260.0), (lx0 + 260.0, lx0 + 400.0), (lx0 + 400.0, lx1 - 10.0)] {
        let r = rect(x0, ly0 + 10.0, x1, hy1);
        c.rect(r, &solid(rgb(0xD4D0C8)), None);
        c.bevel(r, 3.0, false);
    }
    // Rows: gray "text" bars, one row selected in navy.
    for i in 0..6 {
        let y = hy1 + 22.0 + i as f32 * 46.0;
        if y + 24.0 > ly1 - 10.0 {
            break;
        }
        let selected = i == 2;
        if selected {
            c.rect(rect(lx0 + 58.0, y - 6.0, lx0 + 236.0, y + 30.0), &solid(rgb(0x0A246A)), None);
        }
        // Row icon: a small yellow square for folders, white page for files.
        let icon = if i < 2 { rgb(0xF6D35E) } else { rgb(0xFFFFFF) };
        c.rect(rect(lx0 + 20.0, y - 2.0, lx0 + 46.0, y + 26.0), &solid(icon), None);
        let mut pb = PathBuilder::new();
        pb.push_rect(rect(lx0 + 20.0, y - 2.0, lx0 + 46.0, y + 26.0));
        c.stroke(&pb.finish().unwrap(), &solid(rgb(if i < 2 { 0x9C7A1E } else { 0x808080 })), 3.0);
        let text = if selected { rgb(0xFFFFFF) } else { rgb(0x9A9A9A) };
        c.rect(rect(lx0 + 66.0, y + 6.0, lx0 + 66.0 + 140.0 - (i % 3) as f32 * 26.0, y + 18.0), &solid(text), None);
        c.rect(rect(lx0 + 282.0, y + 6.0, lx0 + 360.0, y + 18.0), &solid(rgb(0xC4C4C4)), None);
    }

    // The hero: a big pixel-art Windows 2000 folder in front, with a
    // macOS-style soft shadow so it floats above the window.
    let cell = 27.0;
    let (fx, fy) = (418.0, 404.0);
    c.soft_shadow(
        |g| {
            let mut pb = PathBuilder::new();
            pb.push_rect(rect(fx + 0.0 * cell - g, fy + 4.0 * cell - g, fx + 16.0 * cell + g, fy + 15.0 * cell + g));
            pb.finish().unwrap()
        },
        26.0,
        18.0,
        0.30,
    );
    pixel_art(&mut c, fx, fy, cell);

    // macOS: glossy top highlight and a hairline inner edge, clipped to the body.
    c.rect(
        rect(BODY_MIN, BODY_MIN, BODY_MIN + BODY_SIZE, BODY_MIN + BODY_SIZE * 0.55),
        &vgradient(BODY_MIN, BODY_MIN + BODY_SIZE * 0.55, &[(0.0, rgba(0xFFFFFF, 70)), (1.0, rgba(0xFFFFFF, 0))]),
        Some(&clip),
    );
    c.stroke(&squircle(BODY_MIN + 2.0, BODY_MIN + 2.0, BODY_SIZE - 4.0), &solid(rgba(0xFFFFFF, 60)), 4.0);

    c.pixmap
}

fn pixel_art(c: &mut Canvas, x: f32, y: f32, cell: f32) {
    for (row, line) in FOLDER.iter().enumerate() {
        for (col, ch) in line.chars().enumerate() {
            if let Some(color) = folder_color(ch) {
                let (x0, y0) = (x + col as f32 * cell, y + row as f32 * cell);
                // Slight overlap avoids hairline seams between cells.
                let mut p = solid(color);
                p.anti_alias = false;
                c.rect(rect(x0, y0, x0 + cell + 0.5, y0 + cell + 0.5), &p, None);
            }
        }
    }
}

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let iconset = root.join("macsplorer.iconset");
    std::fs::create_dir_all(&iconset).expect("create assets dir");

    draw(1024).save_png(root.join("icon.png")).expect("write icon.png");
    for base in [16u32, 32, 128, 256, 512] {
        draw(base).save_png(iconset.join(format!("icon_{base}x{base}.png"))).unwrap();
        draw(base * 2).save_png(iconset.join(format!("icon_{base}x{base}@2x.png"))).unwrap();
    }

    let status = Command::new("iconutil")
        .args(["-c", "icns", "-o"])
        .arg(root.join("macsplorer.icns"))
        .arg(&iconset)
        .status()
        .expect("run iconutil");
    assert!(status.success(), "iconutil failed");
    println!("wrote {}", root.display());
}
