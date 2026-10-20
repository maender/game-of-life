//! Floating control panel (top left) and its collapsed bar, drawn with egui.

use crate::sim::{Snapshot, Speed, SPEEDS};
use crate::ui::{Action, CellStyle, Settings};
use crate::view::Camera;
use egui::{Align, Align2, Color32, Layout, RichText, Stroke};

const PANEL_FILL: Color32 = Color32::from_rgba_premultiplied(19, 24, 30, 224); // rgba(22,27,34,.88)
const BORDER: Color32 = Color32::from_rgb(0x30, 0x36, 0x3d);
const TEXT: Color32 = Color32::from_rgb(0xc9, 0xd1, 0xd9);
const MUTED: Color32 = Color32::from_rgb(0x8b, 0x94, 0x9e);
const ACCENT: Color32 = Color32::from_rgb(0x3f, 0xb8, 0xa8);
const DARK: Color32 = Color32::from_rgb(0x0d, 0x11, 0x17);
const PANEL_WIDTH: f32 = 232.0;
const MARGIN: f32 = 14.0;

/// Draws the panel (or its collapsed bar) and the optional notice; returns the
/// actions triggered by its buttons. Continuous settings (density, color,
/// variation, style) are edited in place.
pub fn draw(
    ctx: &egui::Context,
    s: &mut Settings,
    snap: &Snapshot,
    speed: Speed,
    fps: f32,
    cam: &Camera,
    notice: Option<&str>,
) -> Vec<Action> {
    let mut actions = Vec::new();
    let frame = egui::Frame::new()
        .fill(PANEL_FILL)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(12.0)
        .inner_margin(egui::Margin::symmetric(14, 12))
        .shadow(egui::Shadow {
            offset: [0, 8],
            blur: 24,
            spread: 0,
            color: Color32::from_black_alpha(136),
        });

    egui::Area::new(egui::Id::new("panel"))
        .anchor(Align2::LEFT_TOP, [MARGIN, MARGIN])
        .show(ctx, |ui| {
            style_ui(ui);
            frame.show(ui, |ui| {
                if s.panel_open {
                    full_panel(ui, s, snap, speed, fps, cam, &mut actions);
                } else {
                    collapsed_bar(ui, snap, fps, &mut actions);
                }
            });
        });

    if let Some(text) = notice {
        egui::Area::new(egui::Id::new("notice"))
            .anchor(Align2::CENTER_TOP, [0.0, MARGIN])
            .interactable(false)
            .show(ctx, |ui| {
                style_ui(ui);
                frame.show(ui, |ui| {
                    ui.label(RichText::new(text).color(TEXT));
                });
            });
    }
    actions
}

fn style_ui(ui: &mut egui::Ui) {
    ui.spacing_mut().slider_width = 70.0;
    let v = ui.visuals_mut();
    v.override_text_color = Some(TEXT);
    v.selection.bg_fill = ACCENT;
    v.selection.stroke = Stroke::new(1.0, DARK);
    v.widgets.inactive.weak_bg_fill = Color32::from_rgb(0x21, 0x26, 0x2d);
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER);
}

fn full_panel(
    ui: &mut egui::Ui,
    s: &mut Settings,
    snap: &Snapshot,
    speed: Speed,
    fps: f32,
    cam: &Camera,
    actions: &mut Vec<Action>,
) {
    ui.set_width(PANEL_WIDTH);
    ui.horizontal(|ui| {
        ui.label(RichText::new("Game of Life").strong());
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.button("Réduire ⏶").clicked() {
                actions.push(Action::TogglePanel);
            }
        });
    });
    ui.label(stats(format!(
        "Génération {} · Population {}\n{} FPS · {} gén/s · Zoom ×{}",
        fmt_int(snap.generation),
        fmt_count(snap.population),
        fps.round() as u32,
        snap.gens_per_sec.round() as u32,
        fmt_zoom(cam.px_per_cell),
    )));

    section(ui, "Simulation");
    row(ui, "Espace / N", |ui| {
        let play = if snap.running {
            "⏸ Pause"
        } else {
            "▶ Lecture"
        };
        if ui.selectable_label(snap.running, play).clicked() {
            actions.push(Action::TogglePlay);
        }
        if ui
            .add_enabled(!snap.running, egui::Button::new("⏭ 1 pas"))
            .clicked()
        {
            actions.push(Action::Step);
        }
    });
    row(ui, "+ / −", |ui| {
        ui.label("Vitesse");
        let current = SPEEDS.iter().position(|&x| x == speed).unwrap_or(0);
        let mut idx = current;
        ui.add(egui::Slider::new(&mut idx, 0..=SPEEDS.len() - 1).show_value(false));
        ui.label(speed_label(speed));
        let step = if idx > current {
            Action::Faster
        } else {
            Action::Slower
        };
        actions.extend(std::iter::repeat_n(step, idx.abs_diff(current)));
    });

    section(ui, "Grille");
    row(ui, "R", |ui| {
        if ui.button("🎲 Randomiser la vue").clicked() {
            actions.push(Action::Randomize);
        }
    });
    ui.horizontal(|ui| {
        ui.label("Densité");
        ui.add(egui::Slider::new(&mut s.density, 0.01..=1.0).fixed_decimals(2));
    });
    row(ui, "C", |ui| {
        if ui.button("Tout effacer").clicked() {
            actions.push(Action::Clear);
        }
    });

    section(ui, "Dessin");
    row(ui, "D / E", |ui| {
        if ui.selectable_label(!s.tool_erase, "✏ Dessiner").clicked() {
            actions.push(Action::ToolDraw);
        }
        if ui.selectable_label(s.tool_erase, "✖ Gommer").clicked() {
            actions.push(Action::ToolErase);
        }
    });

    section(ui, "Apparence");
    row(ui, "S", |ui| {
        ui.label("Style");
        egui::ComboBox::from_id_salt("style")
            .selected_text(s.style.label())
            .show_ui(ui, |ui| {
                for style in CellStyle::ALL {
                    ui.selectable_value(&mut s.style, style, style.label());
                }
            });
    });
    ui.horizontal(|ui| {
        ui.label("Couleur");
        let mut rgb = s.color.map(|c| (c * 255.0).round() as u8);
        if egui::color_picker::color_edit_button_srgb(ui, &mut rgb).changed() {
            s.color = rgb.map(|c| c as f32 / 255.0);
        }
        ui.label("Variation");
        ui.add(egui::Slider::new(&mut s.variation, 0.0..=1.0).show_value(false));
    });
    row(ui, "1 2 3", |ui| {
        for (on, label, action) in [
            (s.effect_age, "Âge", Action::ToggleAge),
            (s.effect_trails, "Traînées", Action::ToggleTrails),
            (s.effect_anim, "Animation", Action::ToggleAnim),
        ] {
            if ui.selectable_label(on, label).clicked() {
                actions.push(action);
            }
        }
    });
    row(ui, "L", |ui| {
        if ui.selectable_label(s.degraded, "⚡ Mode dégradé").clicked() {
            actions.push(Action::ToggleDegraded);
        }
    });
}

fn collapsed_bar(ui: &mut egui::Ui, snap: &Snapshot, fps: f32, actions: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        ui.label(if snap.running { "⏸" } else { "▶" });
        ui.label(stats(format!(
            "Gén {} · {} · {} FPS",
            fmt_int(snap.generation),
            fmt_count(snap.population),
            fps.round() as u32,
        )));
        if ui
            .button("⏷")
            .on_hover_text("Ouvrir le panneau (Tab)")
            .clicked()
        {
            actions.push(Action::TogglePanel);
        }
    });
}

fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(6.0);
    ui.label(RichText::new(title.to_uppercase()).small().color(MUTED));
}

/// A row of widgets with its shortcut hint aligned to the right.
fn row(ui: &mut egui::Ui, keys: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        add(ui);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(keys).monospace().small().color(MUTED));
        });
    });
}

fn stats(text: String) -> RichText {
    RichText::new(text).monospace().small().color(MUTED)
}

fn speed_label(speed: Speed) -> String {
    match speed {
        Speed::Gps(n) => format!("{n} gén/s"),
        Speed::Max => "max".to_owned(),
    }
}

/// `48213` -> `"48 213"`.
fn fmt_int(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(' ');
        }
        out.push(c);
    }
    out
}

/// Exact below a million, then `"2,4 M"` / `"1,3 Md"`.
fn fmt_count(n: u64) -> String {
    match n {
        0..=999_999 => fmt_int(n),
        1_000_000..=999_999_999 => format!("{} M", fmt_dec(n as f64 / 1e6, 1)),
        _ => format!("{} Md", fmt_dec(n as f64 / 1e9, 1)),
    }
}

/// Pixels per cell with about three significant digits.
fn fmt_zoom(px_per_cell: f64) -> String {
    let decimals = if px_per_cell >= 10.0 {
        0
    } else {
        (2 - px_per_cell.log10().floor() as i32).max(1) as usize
    };
    fmt_dec(px_per_cell, decimals)
}

/// Fixed decimals, French decimal comma, trailing zeros trimmed.
fn fmt_dec(x: f64, decimals: usize) -> String {
    let s = format!("{x:.decimals$}");
    let s = if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.')
    } else {
        &s
    };
    s.replace('.', ",")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::TileMap;
    use std::sync::Arc;
    use std::time::Instant;

    #[test]
    fn number_formats() {
        assert_eq!(fmt_int(0), "0");
        assert_eq!(fmt_int(48_213), "48 213");
        assert_eq!(fmt_int(1_234_567), "1 234 567");
        assert_eq!(fmt_count(999_999), "999 999");
        assert_eq!(fmt_count(2_400_000), "2,4 M");
        assert_eq!(fmt_count(3_000_000_000), "3 Md");
        assert_eq!(fmt_zoom(8.0), "8");
        assert_eq!(fmt_zoom(0.25), "0,25");
        assert_eq!(fmt_zoom(128.0), "128");
        assert_eq!(fmt_zoom(1.0 / 4096.0), "0,000244");
    }

    /// Every label renders with egui's default fonts (no missing glyphs), in
    /// both the open and the collapsed state, and the notice is shown.
    #[test]
    fn panel_glyphs_exist() {
        let ctx = egui::Context::default();
        let tiles = Arc::new(TileMap::default());
        let snap = Snapshot {
            tiles: tiles.clone(),
            prev: tiles,
            meta: None,
            generation: 48_213,
            population: 2_400_000,
            running: false,
            gens_per_sec: 0.0,
            published_at: Instant::now(),
        };
        let cam = Camera {
            center: (0.0, 0.0),
            px_per_cell: 8.0,
            screen: (800.0, 600.0),
        };
        let mut texts = Vec::new();
        for open in [true, false] {
            let mut s = Settings {
                panel_open: open,
                ..Settings::default()
            };
            for _ in 0..2 {
                let out = ctx.run(egui::RawInput::default(), |ctx| {
                    draw(ctx, &mut s, &snap, Speed::Gps(30), 60.0, &cam, Some("Zone"));
                });
                texts.extend(collect_text(&out.shapes));
            }
        }
        let all: String = texts.concat();
        for needle in ["Génération 48 213", "Randomiser", "Zone", "Gén 48 213", "⏷"] {
            assert!(all.contains(needle), "missing {needle:?}");
        }
        let font = egui::FontId::proportional(14.0);
        let missing: Vec<char> = ctx.fonts_mut(|f| {
            all.chars()
                .filter(|&c| !c.is_whitespace() && !f.has_glyph(&font, c))
                .collect()
        });
        assert!(missing.is_empty(), "no glyph for {missing:?}");
    }

    /// The collapsed bar shows the same icon as the full panel's play button:
    /// pause while running, play while paused.
    #[test]
    fn collapsed_bar_icon_matches_play_button() {
        let tiles = Arc::new(TileMap::default());
        let cam = Camera {
            center: (0.0, 0.0),
            px_per_cell: 8.0,
            screen: (800.0, 600.0),
        };
        for (running, shown, hidden) in [(true, "⏸", "▶"), (false, "▶", "⏸")] {
            let ctx = egui::Context::default();
            let snap = Snapshot {
                tiles: tiles.clone(),
                prev: tiles.clone(),
                meta: None,
                generation: 0,
                population: 0,
                running,
                gens_per_sec: 0.0,
                published_at: Instant::now(),
            };
            let mut s = Settings {
                panel_open: false,
                ..Settings::default()
            };
            let mut texts = Vec::new();
            for _ in 0..2 {
                let out = ctx.run(egui::RawInput::default(), |ctx| {
                    draw(ctx, &mut s, &snap, Speed::Gps(30), 60.0, &cam, None);
                });
                texts = collect_text(&out.shapes);
            }
            let all = texts.concat();
            assert!(all.contains(shown), "running={running}: missing {shown}");
            assert!(
                !all.contains(hidden),
                "running={running}: unexpected {hidden}"
            );
        }
    }

    /// The app lets egui have Escape while `Popup::is_any_open`: opening the
    /// style combo box must register as an open popup, and Escape closes it.
    #[test]
    fn style_combo_is_an_open_popup_until_escape() {
        let ctx = egui::Context::default();
        let tiles = Arc::new(TileMap::default());
        let snap = Snapshot {
            tiles: tiles.clone(),
            prev: tiles,
            meta: None,
            generation: 0,
            population: 0,
            running: false,
            gens_per_sec: 0.0,
            published_at: Instant::now(),
        };
        let cam = Camera {
            center: (0.0, 0.0),
            px_per_cell: 8.0,
            screen: (800.0, 600.0),
        };
        let mut s = Settings::default();
        let label = s.style.label();
        let frame = |events: Vec<egui::Event>, s: &mut Settings| {
            let input = egui::RawInput {
                events,
                ..Default::default()
            };
            ctx.run(input, |ctx| {
                draw(ctx, s, &snap, Speed::Gps(30), 60.0, &cam, None);
            })
        };
        frame(vec![], &mut s); // the first pass only measures the layout
        let out = frame(vec![], &mut s);
        let combo = find_text(&out.shapes, label).expect("style combo drawn");
        assert!(!egui::Popup::is_any_open(&ctx));

        let pos = combo.center();
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(vec![egui::Event::PointerMoved(pos), button(true)], &mut s);
        frame(vec![button(false)], &mut s);
        frame(vec![], &mut s);
        assert!(egui::Popup::is_any_open(&ctx), "combo click opens a popup");

        let esc = egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        frame(vec![esc], &mut s);
        frame(vec![], &mut s);
        assert!(!egui::Popup::is_any_open(&ctx), "Escape closes the popup");
    }

    fn find_text(shapes: &[egui::epaint::ClippedShape], needle: &str) -> Option<egui::Rect> {
        fn walk(shape: &egui::Shape, needle: &str) -> Option<egui::Rect> {
            match shape {
                egui::Shape::Text(t) if t.galley.text() == needle => {
                    Some(t.galley.rect.translate(t.pos.to_vec2()))
                }
                egui::Shape::Vec(v) => v.iter().find_map(|s| walk(s, needle)),
                _ => None,
            }
        }
        shapes.iter().find_map(|s| walk(&s.shape, needle))
    }

    fn collect_text(shapes: &[egui::epaint::ClippedShape]) -> Vec<String> {
        fn walk(shape: &egui::Shape, out: &mut Vec<String>) {
            match shape {
                egui::Shape::Text(t) => out.push(t.galley.text().to_owned()),
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                _ => {}
            }
        }
        let mut out = Vec::new();
        shapes.iter().for_each(|s| walk(&s.shape, &mut out));
        out
    }
}
