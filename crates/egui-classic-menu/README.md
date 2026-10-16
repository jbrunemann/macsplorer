# egui-classic-menu

Menu rows in the style of Windows 2000 for egui 0.36.

- A 7 pt gutter on the left holds the check mark or radio bullet, one on the
  right holds the submenu arrow, and a keyboard shortcut sits left of the
  arrow. Every row of a menu reserves both, so the labels line up.
- The whole row highlights on hover, mark and arrow included.
- Marks and arrows are drawn pixel by pixel, so they stay crisp at 1x.

It takes its colors from the `Visuals` of the `Ui`: the label color of
`widgets.inactive` and the highlighted one of `widgets.hovered`. Style the
popup with the usual egui calls and give it [`style`]:

```rust
egui::MenuBar::new()
    .config(egui::containers::menu::MenuConfig::new().style(egui_classic_menu::style))
    .ui(ui, |ui| {
        ui.menu_button("View", |ui| {
            egui_classic_menu::radio(ui, true, "Details");
            egui_classic_menu::checkbox(ui, &mut checked, "Status Bar");
            ui.add(egui_classic_menu::item(ui, "Refresh", Some("F5".into())));
        });
    });
```

Submenus: `submenu_row` draws the row with its arrow and returns the response;
open the submenu from it with `egui::containers::menu::SubMenu`.

The sizes (`GUTTER`, `EDGE`, the 5 pt button padding in `style`) were measured
against a 12 px Tahoma menu with a 1 px frame. Change the constants for a
different font or frame.
