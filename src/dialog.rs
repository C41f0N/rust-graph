// Shared modal dialogs.
//
// The delete confirmation and the filename prompts used to be thin strips at
// the top of the screen answered with Y/N or a bare text line. They now share
// one centred panel: a title, a message or text field, and explicit Cancel /
// action buttons that work with the mouse as well as the keyboard (Enter
// confirms, Esc cancels). Geometry depends only on the window size and the UI
// scale, so the input handlers and the renderer always agree on where every
// control is without sharing mutable state.
use raylib::prelude::*;

use crate::config;
use crate::editor::text;
use crate::ui;

// The choice a click on the dialog resolves to. A press outside the panel
// counts as Cancel, matching the Esc convention.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Confirm,
    Cancel,
}

pub struct DialogLayout {
    pub panel: Rectangle,
    pub field: Option<Rectangle>,
    pub cancel: Rectangle,
    pub confirm: Rectangle,
}

impl DialogLayout {
    // A message dialog with Cancel / primary buttons.
    pub fn confirm() -> Self {
        Self::build(false)
    }

    // A dialog that also carries a single-line text field.
    pub fn input() -> Self {
        Self::build(true)
    }

    fn build(has_field: bool) -> Self {
        let pad = config::scaled_size(22);
        let title_h = config::scaled_size(24);
        let gap = config::scaled_size(12);
        let btn_h = config::scaled_size(32);
        let btn_w = config::scaled_size(96);
        let btn_gap = config::scaled_size(10);
        let field_h = config::scaled_size(36);

        // Wide enough for a comfortable sentence, but never wider than the
        // window (plus a small margin) on a narrow display.
        let width = config::scaled_size(430)
            .min((config::width() - config::scaled_size(48)).max(config::scaled_size(280)));

        let height = if has_field {
            pad + title_h
                + gap
                + config::scaled_size(16) // field label
                + config::scaled_size(8)
                + field_h
                + config::scaled_size(22)
                + btn_h
                + pad
        } else {
            pad + title_h
                + gap
                + config::scaled_size(36) // room for two message lines
                + config::scaled_size(24)
                + btn_h
                + pad
        };

        let x = (config::width() - width) / 2;
        let y = (config::height() - height) / 2;
        let panel = Rectangle::new(x as f32, y as f32, width as f32, height as f32);

        let buttons_y = y + height - pad - btn_h;
        let confirm = Rectangle::new(
            (x + width - pad - btn_w) as f32,
            buttons_y as f32,
            btn_w as f32,
            btn_h as f32,
        );
        let cancel = Rectangle::new(
            (x + width - pad - btn_w * 2 - btn_gap) as f32,
            buttons_y as f32,
            btn_w as f32,
            btn_h as f32,
        );

        let field = has_field.then(|| {
            let fy = y + pad + title_h + gap + config::scaled_size(16) + config::scaled_size(8);
            Rectangle::new(
                (x + pad) as f32,
                fy as f32,
                (width - 2 * pad) as f32,
                field_h as f32,
            )
        });

        Self {
            panel,
            field,
            cancel,
            confirm,
        }
    }
}

// Resolve a mouse position over a dialog to a choice. Presses on the scrim
// outside the panel cancel, so the modal always has an obvious way out.
pub fn action_at(layout: &DialogLayout, mouse: Vector2) -> Option<Action> {
    if hit(layout.cancel, mouse) {
        Some(Action::Cancel)
    } else if hit(layout.confirm, mouse) {
        Some(Action::Confirm)
    } else if !hit(layout.panel, mouse) {
        Some(Action::Cancel)
    } else {
        None
    }
}

fn hit(rect: Rectangle, p: Vector2) -> bool {
    p.x >= rect.x && p.x <= rect.x + rect.width && p.y >= rect.y && p.y <= rect.y + rect.height
}

fn pad() -> i32 {
    config::scaled_size(22)
}

fn title_size() -> i32 {
    config::scaled_size(20)
}

fn draw_frame(d: &mut RaylibDrawHandle, layout: &DialogLayout, title: &str) {
    d.draw_rectangle(
        0,
        0,
        config::width(),
        config::height(),
        Color::new(0, 0, 0, 185),
    );
    ui::panel(d, layout.panel, 0.12);

    let x = layout.panel.x as i32 + pad();
    let y = layout.panel.y as i32 + pad();
    text::draw(d, title, x, y, title_size(), ui::TEXT);

    // Quiet rule separating the body from the buttons.
    let rule_y = layout.confirm.y as i32 - config::scaled_size(14);
    d.draw_line(
        x,
        rule_y,
        layout.panel.x as i32 + layout.panel.width as i32 - pad(),
        rule_y,
        ui::BORDER,
    );
}

pub fn draw_confirm(
    d: &mut RaylibDrawHandle,
    layout: &DialogLayout,
    title: &str,
    message: &str,
    confirm_label: &str,
) {
    draw_frame(d, layout, title);

    let fs = config::scaled_size(15);
    let x = layout.panel.x as i32 + pad();
    let y = layout.panel.y as i32 + pad() + title_size() + config::scaled_size(12);
    let max_w = layout.panel.width as i32 - 2 * pad();
    draw_wrapped(
        d,
        message,
        x,
        y,
        max_w,
        fs,
        ui::TEXT_SECONDARY,
        config::scaled_size(5),
    );

    draw_buttons(d, layout, confirm_label);
}

pub fn draw_input(
    d: &mut RaylibDrawHandle,
    layout: &DialogLayout,
    title: &str,
    label: &str,
    value: &str,
    confirm_label: &str,
) {
    draw_frame(d, layout, title);

    let label_fs = config::scaled_size(13);
    let x = layout.panel.x as i32 + pad();
    let label_y = layout.panel.y as i32 + pad() + title_size() + config::scaled_size(12);
    text::draw(d, label, x, label_y, label_fs, ui::TEXT_MUTED);

    if let Some(field) = layout.field {
        d.draw_rectangle_rounded(field, 0.25, 8, ui::PANEL_SOLID);
        ui::border(d, field, 0.25, ui::ACCENT_STRONG);

        let fs = config::scaled_size(17);
        let tx = field.x as i32 + config::scaled_size(12);
        let ty = field.y as i32 + (field.height as i32 - fs) / 2;
        text::draw(d, value, tx, ty, fs, ui::TEXT);

        let caret_x = tx + text::measure(d, value, fs) + config::scaled_size(1);
        d.draw_rectangle(
            caret_x,
            ty,
            config::scaled_size(2).max(1),
            fs,
            ui::ACCENT,
        );
    }

    draw_buttons(d, layout, confirm_label);
}

fn draw_buttons(d: &mut RaylibDrawHandle, layout: &DialogLayout, confirm_label: &str) {
    let mouse = d.get_mouse_position();
    let cancel_hover = hit(layout.cancel, mouse);
    let confirm_hover = hit(layout.confirm, mouse);
    let fs = config::scaled_size(14);

    // Cancel: quiet raised surface.
    ui::surface(
        d,
        layout.cancel,
        0.28,
        if cancel_hover {
            Color::new(46, 46, 46, 255)
        } else {
            ui::SURFACE_RAISED
        },
    );
    ui::border(d, layout.cancel, 0.28, ui::BORDER);
    let cancel_w = text::measure(d, "Cancel", fs);
    text::draw(
        d,
        "Cancel",
        layout.cancel.x as i32 + (layout.cancel.width as i32 - cancel_w) / 2,
        layout.cancel.y as i32 + (layout.cancel.height as i32 - fs) / 2,
        fs,
        if cancel_hover {
            ui::TEXT
        } else {
            ui::TEXT_SECONDARY
        },
    );

    // The action is the one solid white element in the dialog.
    ui::surface(
        d,
        layout.confirm,
        0.28,
        if confirm_hover {
            Color::new(255, 255, 255, 255)
        } else {
            Color::new(232, 232, 232, 255)
        },
    );
    let confirm_w = text::measure(d, confirm_label, fs);
    text::draw(
        d,
        confirm_label,
        layout.confirm.x as i32 + (layout.confirm.width as i32 - confirm_w) / 2,
        layout.confirm.y as i32 + (layout.confirm.height as i32 - fs) / 2,
        fs,
        Color::new(10, 10, 10, 255),
    );
}

// Greedy word wrap for the dialog message. Names are short and panels are
// wide, so this is only here so a long note title cannot run under the
// buttons.
fn draw_wrapped(
    d: &mut RaylibDrawHandle,
    message: &str,
    x: i32,
    y: i32,
    max_w: i32,
    fs: i32,
    color: Color,
    line_gap: i32,
) {
    let mut line = String::new();
    let mut cy = y;
    for word in message.split_whitespace() {
        let candidate = if line.is_empty() {
            word.to_string()
        } else {
            format!("{} {}", line, word)
        };
        if !line.is_empty() && text::measure(d, &candidate, fs) > max_w {
            text::draw(d, &line, x, cy, fs, color);
            cy += fs + line_gap;
            line = word.to_string();
        } else {
            line = candidate;
        }
    }
    if !line.is_empty() {
        text::draw(d, &line, x, cy, fs, color);
    }
}

fn truncate(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut out: String = value.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('\u{2026}');
    out
}

// Draw whichever dialog currently owns the frame. Called once at the very end
// of the frame so the modal sits above the graph, editor, sidebar and helper
// bar. The states are mutually exclusive in practice; the order only decides
// which one wins if a bug leaves two of them set.
pub fn draw_active(d: &mut RaylibDrawHandle) {
    if *crate::graph::processing::DELETE_PENDING.read().unwrap() {
        let selected = *crate::graph::processing::SELECTED_NODE.read().unwrap();
        let name = {
            let nodes = crate::graph::processing::NODES.read().unwrap();
            selected.and_then(|idx| nodes.get(idx).map(|n| n.name.clone()))
        };
        let message = match name {
            Some(name) => format!(
                "\"{}\" will be deleted from disk. This cannot be undone.",
                truncate(&name, 64)
            ),
            None => "This note will be deleted from disk. This cannot be undone.".to_string(),
        };
        let layout = DialogLayout::confirm();
        draw_confirm(d, &layout, "Delete note", &message, "Delete");
        return;
    }

    if *crate::graph::processing::ADDING_NOTE.read().unwrap() {
        let name = crate::graph::processing::ADDING_NAME.read().unwrap();
        let layout = DialogLayout::input();
        draw_input(d, &layout, "New note", "Filename", &name, "Create");
        return;
    }

    if crate::editor::CREATING_NODE.load(std::sync::atomic::Ordering::Relaxed) {
        let name = crate::editor::NEW_NODE_NAME.read().unwrap();
        let layout = DialogLayout::input();
        draw_input(d, &layout, "Create note", "Filename", &name, "Create");
    }
}
