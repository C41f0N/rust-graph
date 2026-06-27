use crate::graph::processing::*;
use raylib::prelude::*;

pub fn draw(d: &mut RaylibDrawHandle) {
    let dragging_node = DRAGGING_NODE.read().unwrap();
    let nodes = NODES.read().unwrap();
    let edges = EDGES.read().unwrap();

    d.clear_background(Color::BLACK);

    for edge in edges.iter() {
        let n1 = &nodes[edge.n1];
        let n2 = &nodes[edge.n2];
        d.draw_line_ex(n1.position, n2.position, 2., Color::LIGHTGRAY);
    }

    for (i, node) in nodes.iter().enumerate() {
        d.draw_circle_v(
            node.position,
            node.radius,
            if i == dragging_node.unwrap_or(usize::MAX) {
                Color::LIGHTPINK
            } else {
                node.color
            },
        );
    }
}
