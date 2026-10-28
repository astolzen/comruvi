//! Vector icons painted with egui (no Unicode / extra font needed).

use eframe::egui::{self, Color32, Pos2, Sense, Stroke, Vec2};

#[derive(Clone, Copy)]
pub enum Icon {
    ChevronRight,
    ChevronDown,
    ArrowUp,
    Refresh,
    FolderOpen,
    Plus,
    Close,
}

/// Icon-only clickable control (folder tree expand/collapse, remove, …).
pub fn icon_button(ui: &mut egui::Ui, icon: Icon) -> egui::Response {
    let size = Vec2::splat(ui.spacing().interact_size.y.min(22.0));
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let visuals = ui.style().interact(&response);
    if response.hovered() || response.is_pointer_button_down_on() {
        ui.painter().rect_filled(rect.expand(2.0), 3.0, visuals.bg_fill);
    }
    paint_icon(ui, rect, icon, visuals.fg_stroke.color, 1.6);
    response
}

/// Toolbar-style button: icon + label.
pub fn icon_text_button(ui: &mut egui::Ui, icon: Icon, label: &str) -> egui::Response {
    let icon_size = Vec2::splat(14.0);
    let galley = ui.fonts(|f| f.layout_no_wrap(label.to_owned(), egui::TextStyle::Button.resolve(ui.style()), Color32::PLACEHOLDER));
    let pad = ui.spacing().button_padding;
    let desired = Vec2::new(
        pad.x * 2.0 + icon_size.x + 6.0 + galley.size().x,
        (pad.y * 2.0 + galley.size().y.max(icon_size.y)).max(ui.spacing().interact_size.y),
    );
    let (rect, response) = ui.allocate_exact_size(desired, Sense::click());
    let visuals = ui.style().interact(&response);
    let bg_fill = visuals.weak_bg_fill;
    let bg_stroke = visuals.bg_stroke;
    let text_color = visuals.text_color();
    ui.painter().rect(
        rect,
        egui::CornerRadius::same(2),
        bg_fill,
        bg_stroke,
        egui::StrokeKind::Inside,
    );
    let icon_rect = egui::Rect::from_center_size(
        Pos2::new(rect.left() + pad.x + icon_size.x * 0.5, rect.center().y),
        icon_size,
    );
    paint_icon(ui, icon_rect, icon, text_color, 1.5);
    let text_pos = Pos2::new(icon_rect.right() + 6.0, rect.center().y - galley.size().y * 0.5);
    ui.painter().galley(text_pos, galley, text_color);
    response
}

fn paint_icon(ui: &mut egui::Ui, rect: egui::Rect, icon: Icon, color: Color32, thickness: f32) {
    let stroke = Stroke::new(thickness, color);
    let c = rect.center();
    let s = rect.width().min(rect.height()) * 0.5;
    let painter = ui.painter();

    match icon {
        Icon::ChevronRight => {
            let tip = c + Vec2::new(s * 0.35, 0.0);
            let top = c + Vec2::new(-s * 0.25, -s * 0.45);
            let bot = c + Vec2::new(-s * 0.25, s * 0.45);
            painter.line_segment([top, tip], stroke);
            painter.line_segment([tip, bot], stroke);
        }
        Icon::ChevronDown => {
            let tip = c + Vec2::new(0.0, s * 0.35);
            let left = c + Vec2::new(-s * 0.45, -s * 0.25);
            let right = c + Vec2::new(s * 0.45, -s * 0.25);
            painter.line_segment([left, tip], stroke);
            painter.line_segment([tip, right], stroke);
        }
        Icon::ArrowUp => {
            let tip = c + Vec2::new(0.0, -s * 0.45);
            let left = c + Vec2::new(-s * 0.4, -s * 0.05);
            let right = c + Vec2::new(s * 0.4, -s * 0.05);
            let shaft_top = c + Vec2::new(0.0, -s * 0.15);
            let shaft_bot = c + Vec2::new(0.0, s * 0.45);
            painter.line_segment([left, tip], stroke);
            painter.line_segment([tip, right], stroke);
            painter.line_segment([shaft_top, shaft_bot], stroke);
        }
        Icon::Refresh => {
            // Open circle with an arrow head at the top-right.
            let r = s * 0.55;
            let steps = 28;
            let start = -std::f32::consts::FRAC_PI_2 + 0.55;
            let sweep = std::f32::consts::TAU - 1.35;
            let mut prev = None;
            for i in 0..=steps {
                let t = start + sweep * (i as f32 / steps as f32);
                let p = c + Vec2::angled(t) * r;
                if let Some(q) = prev {
                    painter.line_segment([q, p], stroke);
                }
                prev = Some(p);
            }
            let end = c + Vec2::angled(start + sweep) * r;
            let tangent = Vec2::angled(start + sweep + std::f32::consts::FRAC_PI_2);
            let tip = end + tangent * (s * 0.05) + Vec2::angled(start + sweep) * (s * 0.08);
            let wing = end - tangent * (s * 0.35) - Vec2::angled(start + sweep) * (s * 0.15);
            painter.line_segment([end, tip], stroke);
            painter.line_segment([end, wing], stroke);
        }
        Icon::FolderOpen => {
            let left = rect.left() + s * 0.15;
            let right = rect.right() - s * 0.1;
            let top = rect.top() + s * 0.25;
            let bot = rect.bottom() - s * 0.2;
            let tab_w = s * 0.45;
            // Back tab
            painter.line_segment([Pos2::new(left, top + s * 0.2), Pos2::new(left, top)], stroke);
            painter.line_segment([Pos2::new(left, top), Pos2::new(left + tab_w, top)], stroke);
            painter.line_segment([Pos2::new(left + tab_w, top), Pos2::new(left + tab_w + s * 0.15, top + s * 0.2)], stroke);
            // Body
            painter.line_segment([Pos2::new(left, top + s * 0.2), Pos2::new(left, bot)], stroke);
            painter.line_segment([Pos2::new(left, bot), Pos2::new(right, bot)], stroke);
            painter.line_segment([Pos2::new(right, bot), Pos2::new(right - s * 0.1, top + s * 0.2)], stroke);
            painter.line_segment([Pos2::new(right - s * 0.1, top + s * 0.2), Pos2::new(left, top + s * 0.2)], stroke);
        }
        Icon::Plus => {
            painter.line_segment([c - Vec2::new(s * 0.4, 0.0), c + Vec2::new(s * 0.4, 0.0)], stroke);
            painter.line_segment([c - Vec2::new(0.0, s * 0.4), c + Vec2::new(0.0, s * 0.4)], stroke);
        }
        Icon::Close => {
            let d = s * 0.35;
            painter.line_segment([c + Vec2::new(-d, -d), c + Vec2::new(d, d)], stroke);
            painter.line_segment([c + Vec2::new(d, -d), c + Vec2::new(-d, d)], stroke);
        }
    }
}
