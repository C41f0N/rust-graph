use std::sync::RwLock;

use crate::config::HEIGHT;
use crate::config::WIDTH;
use crate::graph::processing::*;
use raylib::ffi;
use raylib::prelude::*;
use std::ffi::CString;

pub static CAMERA: RwLock<Camera2D> = RwLock::new(Camera2D {
    target: Vector2 {
        x: WIDTH as f32 / 2.0,
        y: HEIGHT as f32 / 2.0,
    },
    offset: Vector2 {
        x: WIDTH as f32 / 2.0,
        y: HEIGHT as f32 / 2.0,
    },
    rotation: 0.0,
    zoom: 1.0,
});

pub fn draw(d: &mut RaylibDrawHandle) {
    let dragging_node = DRAGGING_NODE.read().unwrap();
    let hover_node = HOVER_NODE.read().unwrap();
    let nodes = NODES.read().unwrap();
    let edges = EDGES.read().unwrap();
    let camera = CAMERA.read().unwrap();

    let mut mode = d.begin_mode2D(*camera);

    mode.clear_background(Color::BLACK);

    for edge in edges.iter() {
        let n1 = &nodes[edge.n1];
        let n2 = &nodes[edge.n2];
        mode.draw_line_ex(n1.position, n2.position, 2., Color::LIGHTGRAY);
    }

    for (i, node) in nodes.iter().enumerate() {
        mode.draw_circle_v(
            node.position,
            if i == dragging_node.unwrap_or(usize::MAX) {
                node.radius * 1.5
            } else {
                node.radius
            },
            if i == hover_node.unwrap_or(usize::MAX) {
                Color::LIGHTPINK
            } else {
                node.color
            },
        );

        let font = mode.get_font_default();

        let c_text = CString::new(node.name.clone()).unwrap();

        let text_size =
            unsafe { ffi::MeasureTextEx(ffi::GetFontDefault(), c_text.as_ptr(), 5.0, 0.0) };

        mode.draw_text_ex(
            font,
            &node.name,
            Vector2::new(
                node.position.x - text_size.x / 2.0,
                node.position.y + node.radius + 5.0,
            ),
            5.0,
            0.0,
            Color::WHITE.alpha(((camera.zoom - 2.0) / 0.5).clamp(0.0, 1.0)),
        );
    }
}
