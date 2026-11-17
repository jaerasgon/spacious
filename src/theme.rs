//! Aurora UI design language: tokens for the Dark, Darker and Light themes, fonts, egui
//! visuals and the shared button helpers. Widgets read colours from `tk()`, never literals.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use egui::{
    Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Id, Rect, Response, Sense, Stroke, StrokeKind,
    TextStyle, Ui, Visuals, vec2,
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ThemeKind {
    Dark,
    Darker,
    Light,
}

impl ThemeKind {
    pub const ALL: [ThemeKind; 3] = [ThemeKind::Dark, ThemeKind::Darker, ThemeKind::Light];
    pub fn name(self) -> &'static str {
        match self {
            ThemeKind::Dark => "Dark",
            ThemeKind::Darker => "Darker",
            ThemeKind::Light => "Light",
        }
    }
    pub fn from_name(s: &str) -> Option<ThemeKind> {
        ThemeKind::ALL.into_iter().find(|k| k.name().eq_ignore_ascii_case(s.trim()))
    }
}

pub struct Tokens {
    pub kind: ThemeKind,
    pub app_bg: Color32,
    pub header_bg: Color32,
    pub panel_header: Color32,
    pub panel_bg: Color32,
    pub inset: Color32,
    pub pasteboard: Color32,
    pub field_bg: Color32,
    pub field_border: Color32,
    pub btn_bg: Color32,
    pub btn_hover: Color32,
    pub btn_pressed: Color32,
    pub btn_fg: Color32,
    pub btn_border: Color32,
    pub btn_icon: Color32,
    pub btn_primary: Color32,
    pub btn_primary_hover: Color32,
    pub btn_primary_pressed: Color32,
    pub btn_shadow: Color32,
    pub separator: Color32,
    pub hover: Color32,
    pub pressed: Color32,
    pub row: Color32,
    pub row_alt: Color32,
    pub row_line: Color32,
    pub row_selected: Color32,
    pub selection_fill: Color32,
    pub selected_name_bg: Color32,
    pub selected_name_fg: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub text_faint: Color32,
    pub disabled_fg: Color32,
    pub tab_text: Color32,
    pub tab_text_active: Color32,
    pub icon: Color32,
    pub icon_active: Color32,
    pub accent: Color32,
    pub warning: Color32,
    pub success: Color32,
    pub cache_busy: Color32,
}

const fn rgb(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

const DARK: Tokens = Tokens {
    kind: ThemeKind::Dark,
    app_bg: rgb(0x141414),
    header_bg: rgb(0x1D1D1D),
    panel_header: rgb(0x1F1F1F),
    panel_bg: rgb(0x262626),
    inset: rgb(0x1A1A1A),
    pasteboard: rgb(0x181818),
    field_bg: rgb(0x1A1A1A),
    field_border: rgb(0x383838),
    btn_bg: rgb(0x333333),
    btn_hover: rgb(0x3A3A3A),
    btn_pressed: rgb(0x2C2C2C),
    btn_fg: rgb(0xE6E6E6),
    btn_border: rgb(0x444444),
    btn_icon: rgb(0xB4B4B4),
    btn_primary: rgb(0x2D8CEB),
    btn_primary_hover: rgb(0x3A95EE),
    btn_primary_pressed: rgb(0x2580D8),
    btn_shadow: Color32::from_rgba_premultiplied(0, 0, 0, 64),
    separator: rgb(0x161616),
    hover: rgb(0x303030),
    pressed: rgb(0x3C3C3C),
    row: rgb(0x262626),
    row_alt: rgb(0x292929),
    row_line: rgb(0x222222),
    row_selected: rgb(0x343840),
    selection_fill: rgb(0x373A40),
    selected_name_bg: rgb(0x3A4455),
    selected_name_fg: rgb(0xE6E6E6),
    text: rgb(0xC8C8C8),
    text_dim: rgb(0x929292),
    text_faint: rgb(0x7A7A7A),
    disabled_fg: rgb(0x5C5C5C),
    tab_text: rgb(0x9A9A9A),
    tab_text_active: rgb(0xE3E3E3),
    icon: rgb(0xB4B4B4),
    icon_active: rgb(0xF0F0F0),
    accent: rgb(0x2D8CEB),
    warning: rgb(0xE89A2C),
    success: rgb(0x6CC06C),
    cache_busy: rgb(0xE8A62C),
};

const DARKER: Tokens = Tokens {
    kind: ThemeKind::Darker,
    app_bg: rgb(0x080808),
    header_bg: rgb(0x161616),
    panel_header: rgb(0x141414),
    panel_bg: rgb(0x191919),
    inset: rgb(0x101010),
    pasteboard: rgb(0x111111),
    field_bg: rgb(0x0E0E0E),
    btn_bg: rgb(0x262626),
    btn_hover: rgb(0x2E2E2E),
    btn_pressed: rgb(0x202020),
    btn_border: rgb(0x363636),
    row: rgb(0x191919),
    row_alt: rgb(0x1D1D1D),
    row_line: rgb(0x151515),
    ..DARK
};

const LIGHT: Tokens = Tokens {
    kind: ThemeKind::Light,
    app_bg: rgb(0xB4B4B4),
    header_bg: rgb(0xD6D6D6),
    panel_header: rgb(0xDCDCDC),
    panel_bg: rgb(0xE6E6E6),
    inset: rgb(0xD2D2D2),
    pasteboard: rgb(0xA8A8A8),
    field_bg: rgb(0xFAFAFA),
    field_border: rgb(0xAAAAAA),
    btn_bg: rgb(0xFFFFFF),
    btn_hover: rgb(0xF6F6F6),
    btn_pressed: rgb(0xEEEEEE),
    btn_fg: rgb(0x141414),
    btn_border: rgb(0xE2E2E2),
    btn_icon: rgb(0x555555),
    btn_primary: rgb(0x2B6EE6),
    btn_primary_hover: rgb(0x3577EA),
    btn_primary_pressed: rgb(0x2563D6),
    btn_shadow: Color32::from_rgba_premultiplied(0, 0, 0, 13),
    separator: rgb(0xC8C8C8),
    hover: rgb(0xD0D0D0),
    pressed: rgb(0xC0C0C0),
    row: rgb(0xE6E6E6),
    row_alt: rgb(0xDEDEDE),
    row_line: rgb(0xD4D4D4),
    row_selected: rgb(0xAAC8F0),
    selection_fill: rgb(0xC4D8F2),
    selected_name_bg: rgb(0xAAC8F0),
    selected_name_fg: rgb(0x101010),
    text: rgb(0x222222),
    text_dim: rgb(0x5A5A5A),
    text_faint: rgb(0x686868),
    disabled_fg: rgb(0x9A9A9A),
    tab_text: rgb(0x5A5A5A),
    tab_text_active: rgb(0x141414),
    icon: rgb(0x3C3C3C),
    icon_active: rgb(0x101010),
    ..DARK
};

static THEMES: [Tokens; 3] = [DARK, DARKER, LIGHT];
static CURRENT: AtomicU8 = AtomicU8::new(0);

/// The current theme's tokens.
pub fn tk() -> &'static Tokens {
    &THEMES[CURRENT.load(Ordering::Relaxed) as usize]
}

pub fn set_theme(ctx: &egui::Context, kind: ThemeKind) {
    let i = ThemeKind::ALL.iter().position(|k| *k == kind).unwrap_or(0);
    CURRENT.store(i as u8, Ordering::Relaxed);
    apply_visuals(ctx);
}

pub const FS_SM: f32 = 10.5;
pub const FS_MD: f32 = 11.0;
pub const FS_LG: f32 = 11.5;

pub const ROW_H: f32 = 19.0;
pub const TAB_H: f32 = 22.0;
pub const APP_BAR_H: f32 = 34.0;
pub const GAP: f32 = 2.0;

pub fn ui_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}
pub fn mono(size: f32) -> FontId {
    FontId::new(size, FontFamily::Monospace)
}
pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("semibold".into()))
}
pub fn medium(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("medium".into()))
}

static INTER_REGULAR: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
static INTER_MEDIUM: &[u8] = include_bytes!("../assets/fonts/Inter-Medium.ttf");
static INTER_SEMIBOLD: &[u8] = include_bytes!("../assets/fonts/Inter-SemiBold.ttf");
static JETBRAINS_MONO: &[u8] = include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf");

/// Install fonts and the visuals of `kind`.
pub fn install(ctx: &egui::Context, kind: ThemeKind) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert("inter".into(), Arc::new(FontData::from_static(INTER_REGULAR)));
    fonts.font_data.insert("inter-medium".into(), Arc::new(FontData::from_static(INTER_MEDIUM)));
    fonts.font_data.insert("inter-semibold".into(), Arc::new(FontData::from_static(INTER_SEMIBOLD)));
    fonts.font_data.insert("jbmono".into(), Arc::new(FontData::from_static(JETBRAINS_MONO)));
    fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "inter".into());
    fonts.families.entry(FontFamily::Monospace).or_default().insert(0, "jbmono".into());
    // Inter has no ▸ ⌄ ✓ and similar symbols: JetBrains Mono fills them in.
    fonts.families.entry(FontFamily::Proportional).or_default().insert(1, "jbmono".into());
    fonts.families.insert(
        FontFamily::Name("semibold".into()),
        vec!["inter-semibold".into(), "inter".into(), "jbmono".into()],
    );
    fonts.families.insert(
        FontFamily::Name("medium".into()),
        vec!["inter-medium".into(), "inter".into(), "jbmono".into()],
    );
    ctx.set_fonts(fonts);
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = vec2(6.0, 4.0);
        s.spacing.button_padding = vec2(8.0, 3.0);
        s.spacing.interact_size.y = 22.0;
        s.spacing.menu_margin = egui::Margin::same(5);
        s.text_styles.insert(TextStyle::Body, ui_font(FS_LG));
        s.text_styles.insert(TextStyle::Button, ui_font(FS_LG));
        s.text_styles.insert(TextStyle::Small, ui_font(FS_SM));
        s.text_styles.insert(TextStyle::Heading, semibold(14.0));
        s.text_styles.insert(TextStyle::Monospace, mono(FS_LG));
        s.animation_time = 0.1;
        s.spacing.scroll = egui::style::ScrollStyle::solid();
        s.spacing.scroll.bar_width = 6.0;
    });
    set_theme(ctx, kind);
}

fn apply_visuals(ctx: &egui::Context) {
    let t = tk();
    let mut v = if t.kind == ThemeKind::Light { Visuals::light() } else { Visuals::dark() };
    v.panel_fill = t.panel_bg;
    v.window_fill = t.panel_bg;
    v.extreme_bg_color = t.field_bg;
    v.faint_bg_color = t.row_alt;
    v.override_text_color = Some(t.text);
    v.selection.bg_fill = t.accent.lerp_to_gamma(t.panel_bg, 0.5);
    v.selection.stroke = Stroke::new(1.0, if t.kind == ThemeKind::Light { t.icon_active } else { Color32::WHITE });
    v.hyperlink_color = t.accent;
    v.window_stroke = Stroke::new(1.0, t.field_border);
    v.window_corner_radius = CornerRadius::same(4);
    v.menu_corner_radius = CornerRadius::same(4);
    v.popup_shadow = egui::epaint::Shadow { offset: [0, 3], blur: 12, spread: 0, color: Color32::from_black_alpha(120) };
    v.window_shadow = egui::epaint::Shadow { offset: [0, 6], blur: 20, spread: 0, color: Color32::from_black_alpha(140) };
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = CornerRadius::same(2);
    }
    v.widgets.noninteractive.bg_fill = t.panel_bg;
    v.widgets.noninteractive.weak_bg_fill = t.panel_bg;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, t.separator);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, t.text);
    v.widgets.inactive.bg_fill = t.field_bg;
    v.widgets.inactive.weak_bg_fill = t.btn_bg;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, t.field_border);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, t.text);
    v.widgets.hovered.bg_fill = t.hover;
    v.widgets.hovered.weak_bg_fill = t.selection_fill;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, t.field_border);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, t.tab_text_active);
    v.widgets.active.bg_fill = t.pressed;
    v.widgets.active.weak_bg_fill = t.btn_pressed;
    v.widgets.active.fg_stroke = Stroke::new(1.0, t.tab_text_active);
    v.widgets.open.bg_fill = t.hover;
    v.widgets.open.weak_bg_fill = t.selection_fill;
    ctx.set_visuals(v);
}

fn paint_button_frame(ui: &Ui, rect: Rect, resp: &Response, primary: bool, enabled: bool) {
    let t = tk();
    let down = resp.is_pointer_button_down_on();
    let (fill, border) = if primary {
        let f = if !enabled {
            t.btn_primary.gamma_multiply(0.5)
        } else if down {
            t.btn_primary_pressed
        } else if resp.hovered() {
            t.btn_primary_hover
        } else {
            t.btn_primary
        };
        (f, f)
    } else {
        let f = if enabled && down {
            t.btn_pressed
        } else if enabled && resp.hovered() {
            t.btn_hover
        } else {
            t.btn_bg
        };
        (f, t.btn_border)
    };
    let p = ui.painter();
    p.rect_filled(rect.translate(vec2(0.0, 1.0)), CornerRadius::same(7), t.btn_shadow);
    p.rect(rect, CornerRadius::same(7), fill, Stroke::new(1.0, border), StrokeKind::Inside);
}

/// The one matte button: 26 px high, min 64 px wide, 7 px radius, 1 px border.
/// `id` is its stable automation id.
pub fn button(ui: &mut Ui, id: &str, label: &str, primary: bool, enabled: bool) -> Response {
    let t = tk();
    let galley = ui.painter().layout_no_wrap(label.to_owned(), semibold(12.0), t.btn_fg);
    let w = (galley.size().x + 24.0).max(64.0);
    let (rect, _) = ui.allocate_exact_size(vec2(w, 26.0), Sense::hover());
    let resp = ui.interact(rect, Id::new(id), if enabled { Sense::click() } else { Sense::hover() });
    if ui.is_rect_visible(rect) {
        paint_button_frame(ui, rect, &resp, primary, enabled);
        let fg = if !enabled {
            t.disabled_fg
        } else if primary {
            Color32::WHITE
        } else {
            t.btn_fg
        };
        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, label, semibold(12.0), fg);
    }
    resp
}

/// Square icon-only button, same height and style as `button`.
pub fn icon_button(ui: &mut Ui, id: &str, glyph: &str, enabled: bool) -> Response {
    let t = tk();
    let (rect, _) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::hover());
    let resp = ui.interact(rect, Id::new(id), if enabled { Sense::click() } else { Sense::hover() });
    if ui.is_rect_visible(rect) {
        paint_button_frame(ui, rect, &resp, false, enabled);
        let fg = if enabled { t.btn_icon } else { t.disabled_fg };
        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, glyph, ui_font(14.0), fg);
    }
    resp
}

/// Flat toolbar icon in a panel: no border, hover fill.
pub fn flat_icon(ui: &mut Ui, id: &str, glyph: &str, enabled: bool) -> Response {
    let t = tk();
    let (rect, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
    let resp = ui.interact(rect, Id::new(id), if enabled { Sense::click() } else { Sense::hover() });
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        if enabled && resp.is_pointer_button_down_on() {
            p.rect_filled(rect, CornerRadius::same(2), t.pressed);
        } else if enabled && resp.hovered() {
            p.rect_filled(rect, CornerRadius::same(2), t.hover);
        }
        let fg = if !enabled {
            t.disabled_fg
        } else if resp.hovered() {
            t.icon_active
        } else {
            t.icon
        };
        p.text(rect.center(), egui::Align2::CENTER_CENTER, glyph, ui_font(13.0), fg);
    }
    resp
}

/// Segmented group of toggle buttons sharing one box. Returns the clicked index.
pub fn segmented(ui: &mut Ui, id: &str, labels: &[&str], active: usize) -> Option<usize> {
    let t = tk();
    let font = semibold(12.0);
    let widths: Vec<f32> = labels
        .iter()
        .map(|l| (ui.painter().layout_no_wrap((*l).to_owned(), font.clone(), t.btn_fg).size().x + 24.0).max(64.0))
        .collect();
    let total: f32 = widths.iter().sum();
    let (rect, _) = ui.allocate_exact_size(vec2(total, 26.0), Sense::hover());
    let p = ui.painter().clone();
    p.rect_filled(rect.translate(vec2(0.0, 1.0)), CornerRadius::same(7), t.btn_shadow);
    p.rect(rect, CornerRadius::same(7), t.btn_bg, Stroke::NONE, StrokeKind::Inside);
    let mut clicked = None;
    let mut x = rect.min.x;
    for (i, (l, w)) in labels.iter().zip(&widths).enumerate() {
        let r = Rect::from_min_size(egui::pos2(x, rect.min.y), vec2(*w, rect.height()));
        let resp = ui.interact(r, Id::new((id, i)), Sense::click());
        let cr = match (i == 0, i + 1 == labels.len()) {
            (true, true) => CornerRadius::same(7),
            (true, false) => CornerRadius { nw: 7, sw: 7, ne: 0, se: 0 },
            (false, true) => CornerRadius { nw: 0, sw: 0, ne: 7, se: 7 },
            _ => CornerRadius::ZERO,
        };
        if i == active {
            p.rect_filled(r, cr, t.selection_fill);
        } else if resp.hovered() {
            p.rect_filled(r, cr, t.btn_hover);
        }
        if i > 0 {
            p.vline(x, rect.y_range(), Stroke::new(1.0, t.btn_border));
        }
        let fg = if i == active { t.tab_text_active } else { t.tab_text };
        p.text(r.center(), egui::Align2::CENTER_CENTER, *l, font.clone(), fg);
        if resp.clicked() {
            clicked = Some(i);
        }
        x += w;
    }
    p.rect_stroke(rect, CornerRadius::same(7), Stroke::new(1.0, t.btn_border), StrokeKind::Inside);
    clicked
}
