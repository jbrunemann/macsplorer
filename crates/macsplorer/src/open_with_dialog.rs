//! The "Open With" dialog "Choose Program..." opens: Windows 2000's
//! "Choose the program you want to use to open this file" with its list of
//! programs, an "Always use this program to open these files" check box and
//! an "Other..." button. The layout and wording are from memory (the strings
//! are in shell32's MUI file, which isn't analyzed). Which app a file opens
//! with is then kept per file type (`macsplorer_core::associations`).

use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc::{Receiver, channel};
use std::thread;

use eframe::egui;
use egui::{Align, Key, Layout, Rect, Sense, Ui, pos2, vec2};

use crate::icons::FileIcons;
use crate::open_with;
use crate::scrollbar::{self, Bars};
use crate::theme::{self, Bevel};

pub const SIZE: egui::Vec2 = vec2(372.0, 392.0);
const FRAME: f32 = 4.0;
const BUTTON: egui::Vec2 = vec2(75.0, 23.0);
const ROW_HEIGHT: f32 = 22.0;
const TITLE: &str = "Open With";

/// What the dialog asks the window to do after this frame.
pub enum Outcome {
    Keep,
    Cancel,
    /// OK: open the file with this app, and make it the choice for the
    /// file's type when `always`.
    Open { app: PathBuf, always: bool },
}

pub struct OpenWithDialog {
    viewport: egui::ViewportId,
    file: PathBuf,
    apps: Vec<PathBuf>,
    /// The first of `apps` is the file's default.
    has_default: bool,
    selected: Option<usize>,
    always: bool,
    /// Scroll the selected row into view on the next frame. Only a keyboard
    /// move or a new app asks for it, or the list would jump back to the
    /// selection whenever the user scrolls away from it.
    reveal: bool,
    /// The app "Other..." picks, sent from the thread that waits for the picker.
    picked: Option<Receiver<PathBuf>>,
}

impl OpenWithDialog {
    /// `apps` are the ones that can open `file`, the file's default first;
    /// that one starts out selected and is marked "(default)".
    pub fn new(file: PathBuf, apps: open_with::Apps) -> Self {
        let selected = (!apps.list.is_empty()).then_some(0);
        Self { viewport: egui::ViewportId::from_hash_of(("open-with", &file)), file, apps: apps.list, has_default: apps.has_default, selected, always: false, reveal: false, picked: None }
    }

    pub fn file(&self) -> &std::path::Path {
        &self.file
    }

    /// Adds an app that isn't on the list and selects it; one that is
    /// already there is just selected.
    fn add(&mut self, app: PathBuf) {
        let index = self.apps.iter().position(|a| open_with::app_name(a) == open_with::app_name(&app)).unwrap_or_else(|| {
            self.apps.push(app);
            self.apps.len() - 1
        });
        self.selected = Some(index);
        self.reveal = true;
    }

    /// "Other...": the system's app picker, off the UI thread since it
    /// blocks until the user answers.
    fn pick_other(&mut self, ctx: &egui::Context) {
        if self.picked.is_some() {
            return;
        }
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        thread::spawn(move || {
            let script = "POSIX path of (choose application with prompt \"Choose the program you want to use to open this file:\" as alias)";
            let Ok(out) = Command::new("osascript").arg("-e").arg(script).output() else { return };
            let app = String::from_utf8_lossy(&out.stdout).trim().to_owned();
            if out.status.success() && !app.is_empty() {
                let _ = tx.send(PathBuf::from(app));
                ctx.request_repaint();
            }
        });
        self.picked = Some(rx);
    }

    /// Shows the dialog in its own window for this frame.
    pub fn show(&mut self, ctx: &egui::Context, icons: &mut FileIcons) -> Outcome {
        let builder = egui::ViewportBuilder::default()
            .with_title(TITLE)
            .with_inner_size(SIZE)
            .with_resizable(false)
            .with_minimize_button(false)
            .with_maximize_button(false)
            .with_decorations(false);
        ctx.show_viewport_immediate(self.viewport, builder, |ui, _| {
            if ui.input(|i| i.viewport().close_requested()) {
                return Outcome::Cancel;
            }
            self.ui(ui, icons)
        })
    }

    /// Draws the whole window: frame, caption, the list, buttons.
    pub fn ui(&mut self, ui: &mut Ui, icons: &mut FileIcons) -> Outcome {
        if let Some(rx) = &self.picked {
            match rx.try_recv() {
                Ok(app) => {
                    self.picked = None;
                    self.add(app);
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.picked = None,
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        let (outer, _) = ui.allocate_exact_size(SIZE, Sense::hover());
        let p = ui.painter();
        p.rect_filled(outer, 0.0, theme::color::face());
        theme::bevel(p, outer, Bevel::Raised);
        let inner = outer.shrink(FRAME);
        let mut caption = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(Layout::top_down(Align::Min)));
        theme::dialog_title_bar(&mut caption, TITLE);

        let font = egui::TextStyle::Body.resolve(ui.style());
        let left = inner.left() + 11.0;
        let right = inner.right() - 11.0;
        let mut y = inner.top() + 18.0 + 11.0;
        let name = self.file.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let intro = ui.fonts_mut(|f| f.layout(format!("Click the program you want to use to open the file '{name}'."), font.clone(), theme::color::text(), right - left));
        let intro_height = intro.size().y;
        ui.painter().galley(pos2(left, y), intro, theme::color::text());
        y += intro_height + 10.0;

        let ok_top = inner.bottom() - 11.0 - BUTTON.y;
        let always_top = ok_top - 14.0 - 15.0;
        let list = Rect::from_min_max(pos2(left, y), pos2(right, always_top - 10.0));
        let (escape, enter, up, down) = ui.input(|i| (i.key_pressed(Key::Escape), i.key_pressed(Key::Enter), i.key_pressed(Key::ArrowUp), i.key_pressed(Key::ArrowDown)));
        let double_clicked = self.list(ui, icons, list, (up, down));

        self.always_box(ui, left, always_top, &font);

        let cancel = Rect::from_min_size(pos2(right - BUTTON.x, ok_top), BUTTON);
        let ok = cancel.translate(vec2(-BUTTON.x - 6.0, 0.0));
        let other = Rect::from_min_size(pos2(left, ok_top), BUTTON);
        if theme::push_button(ui, other, "Other...", true, false).clicked() {
            self.pick_other(ui.ctx());
        }
        let app = self.selected.map(|i| self.apps[i].clone());
        let confirmed = theme::push_button(ui, ok, "OK", app.is_some(), true).clicked() || enter || double_clicked;
        if let (true, Some(app)) = (confirmed, app) {
            return Outcome::Open { app, always: self.always };
        }
        if theme::push_button(ui, cancel, "Cancel", true, false).clicked() || escape {
            return Outcome::Cancel;
        }
        Outcome::Keep
    }

    /// The program list: icon and name per row, one selected. Returns true
    /// on a double-click, which is OK. Up and Down move the selection.
    fn list(&mut self, ui: &mut Ui, icons: &mut FileIcons, rect: Rect, (up, down): (bool, bool)) -> bool {
        let inner = theme::sunken_field(ui.painter(), rect);
        if let Some(selected) = &mut self.selected {
            let before = *selected;
            if up {
                *selected = selected.saturating_sub(1);
            }
            if down {
                *selected = (*selected + 1).min(self.apps.len() - 1);
            }
            self.reveal |= *selected != before;
        }
        let reveal = std::mem::take(&mut self.reveal);
        let font = egui::TextStyle::Body.resolve(ui.style());
        let id = ui.id().with("open-with-bars");
        let content = ROW_HEIGHT * self.apps.len() as f32;
        ui.data_mut(|d| d.insert_temp(id, egui::Vec2b::new(false, content > inner.height())));
        let (bars, viewport) = Bars::begin(ui, id, inner, [false, true], ROW_HEIGHT);
        let mut body = ui.new_child(egui::UiBuilder::new().max_rect(viewport).layout(Layout::top_down(Align::Min)));
        let mut double_clicked = false;
        let mut clicked = None;
        let selected = self.selected;
        let ctx = ui.ctx().clone();
        let out = egui::ScrollArea::vertical().auto_shrink(false).scroll_bar_visibility(scrollbar::hidden(&mut body)).show(&mut body, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (i, app) in self.apps.iter().enumerate() {
                let (row, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::click());
                let icon = Rect::from_min_size(pos2(row.left() + 4.0, row.center().y - 8.0), vec2(16.0, 16.0));
                if let Some(texture) = icons.menu_icon(&ctx, app) {
                    egui::Image::new((texture, icon.size())).paint_at(ui, icon);
                }
                let name = open_with::app_name(app);
                let name = if self.has_default && i == 0 { format!("{name} (default)") } else { name };
                let galley = ui.fonts_mut(|f| f.layout_no_wrap(name, font.clone(), theme::color::text()));
                let label = Rect::from_min_size(pos2(icon.right() + 5.0, row.center().y - 8.0), vec2(galley.size().x + 6.0, 16.0));
                let on = selected == Some(i);
                if on {
                    ui.painter().rect_filled(label, 0.0, theme::color::selection());
                }
                let color = if on { theme::color::selection_text() } else { theme::color::text() };
                ui.painter().galley_with_override_text_color(pos2(label.left() + 3.0, label.center().y - galley.size().y / 2.0), galley, color);
                if on {
                    theme::focus_rect(ui.painter(), label);
                    if reveal {
                        ui.scroll_to_rect(row, None);
                    }
                }
                if resp.clicked() || resp.double_clicked() {
                    clicked = Some(i);
                }
                double_clicked |= resp.double_clicked();
            }
        });
        bars.end(&mut body, &out);
        if clicked.is_some() {
            self.selected = clicked;
        }
        double_clicked
    }

    /// "Always use this program to open these files".
    fn always_box(&mut self, ui: &mut Ui, left: f32, top: f32, font: &egui::FontId) {
        let label = "Always use this program to open these files";
        let galley = ui.fonts_mut(|f| f.layout_no_wrap(label.to_owned(), font.clone(), theme::color::text()));
        let row = Rect::from_min_size(pos2(left, top), vec2(13.0 + 6.0 + galley.size().x, 15.0));
        let resp = ui.interact(row, ui.id().with("always"), Sense::click());
        if resp.clicked() {
            self.always = !self.always;
        }
        theme::check_box(ui.painter(), Rect::from_min_size(pos2(left, top + 1.0), vec2(13.0, 13.0)), self.always, true);
        ui.painter().galley(pos2(left + 19.0, top + 7.5 - galley.size().y / 2.0), galley, theme::color::text());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dialog() -> OpenWithDialog {
        OpenWithDialog::new(PathBuf::from("/tmp/a.txt"), open_with::Apps { list: vec![PathBuf::from("/Applications/TextEdit.app"), PathBuf::from("/Applications/Zed.app")], has_default: true })
    }

    #[test]
    fn the_default_app_starts_selected() {
        assert_eq!(dialog().selected, Some(0));
        assert_eq!(OpenWithDialog::new(PathBuf::from("/tmp/a"), open_with::Apps::default()).selected, None);
    }

    #[test]
    fn another_app_is_added_once_and_selected() {
        let mut d = dialog();
        d.add(PathBuf::from("/Applications/Nova.app"));
        assert_eq!((d.apps.len(), d.selected), (3, Some(2)));
        d.add(PathBuf::from("/Users/me/Applications/Zed.app"));
        assert_eq!((d.apps.len(), d.selected), (3, Some(1)));
    }
}
