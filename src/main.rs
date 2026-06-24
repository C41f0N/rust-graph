use rand::prelude::*;
use raylib::prelude::*;
// use std::cell::RefCell;
// use std::rc::Rc;

mod config;
mod editor;

struct Node {
    radius: f32, // Changed to f32 to match Raylib's draw_circle_v expectations
    color: Color,
    position: Vector2,
    velocity: Vector2, // Changed to a Vector2 so they can move in 2D space
}

struct Edge {
    n1: usize,
    n2: usize,
    direction: i32,
}

fn main() {
    let height = config::HEIGHT;
    let width = config::WIDTH;

    let mut editor_open = false;
    let editor_dimentions = Vector2::new(0.8, 0.8);
    let mut editor_buffer: String = String::new();

    // 1. Initialize the Raylib window and context
    let (mut rl, thread) = raylib::init()
        .size(width, height)
        .title("Raylib Nodes")
        .build();
    rl.set_exit_key(Some(KeyboardKey::KEY_NULL));
    rl.set_trace_log(TraceLogLevel::LOG_ERROR);

    rl.set_target_fps(60);

    let colors = [
        Color::new(255, 0, 127, 255), // Neon Pink
        Color::new(0, 240, 255, 255), // Electric Cyan
        Color::new(57, 255, 20, 255), // Acid Lime
        Color::new(189, 0, 255, 255), // Bright Violet
        Color::new(255, 255, 255, 255),
    ];
    let num_nodes = 10;
    let mut rng = rand::rng();
    // let mut nodes: Vec<Rc<RefCell<Node>>> = Vec::new();
    let mut nodes: Vec<Node> = Vec::new();
    let mut edges: Vec<Edge> = Vec::new();

    // Generating dummy nodes
    for _ in 0..num_nodes {
        // Generate a random angle for movement direction
        let angle: f32 = rng.random_range(0.0..std::f32::consts::TAU);
        let speed: f32 = rng.random_range(50.0..150.0); // Pixels per second

        nodes.push(Node {
            radius: rng.random_range(5.0..=30.0),
            color: *colors.choose(&mut rng).unwrap(),
            position: Vector2::new(
                rng.random_range(0.0..width as f32),
                rng.random_range(0.0..height as f32),
            ),
            // Velocity split into X and Y components based on the angle
            velocity: Vector2::new(0.0, 0.0),
        });
    }

    // Generate dummy edges
    // edges.push(Edge {
    //     n1: nodes[0].clone(),
    //     n2: nodes[1].clone(),
    //     direction: 1,
    // });

    // edges.push(Edge {
    //     n1: nodes[5].clone(),
    //     n2: nodes[1].clone(),
    //     direction: 1,
    // });

    // edges.push(Edge {
    //     n1: nodes[3].clone(),
    //     n2: nodes[8].clone(),
    //     direction: 1,
    // });
    edges.push(Edge {
        n1: 0,
        n2: 1,
        direction: 1,
    });
    edges.push(Edge {
        n1: 5,
        n2: 1,
        direction: 1,
    });
    edges.push(Edge {
        n1: 3,
        n2: 8,
        direction: 1,
    });
    let mut dragging_node: Option<usize> = None;
    let mut last_click_time: f64 = 0.0;
    let mut last_click_node: Option<usize> = None;
    // 2. The Main Game Loop
    while !rl.window_should_close() {
        // Read user input

        // Toggle editor
        // if rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT) {
        //     print!("Button clicked!");
        //     editor_open = !editor_open;
        // }
        // if editor_open {
        //     editor::input_handler::handle_input(&mut rl);
        // }
        let mouse_pos = rl.get_mouse_position();

        if rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT) {
            let current_time = rl.get_time();
            let mut clicked_node: Option<usize> = None;

            for (i, node) in nodes.iter().enumerate() {
                let dist = (node.position - mouse_pos).length();
                if dist <= node.radius {
                    clicked_node = Some(i);
                    break;
                }
            }

            if let Some(i) = clicked_node {
                // Double click check
                if last_click_node == Some(i) && (current_time - last_click_time) < 0.3 {
                    editor_open = true; // double click — editor kholo
                    last_click_node = None; // reset
                } else {
                    // Single click — drag shuru
                    dragging_node = Some(i);
                    last_click_time = current_time;
                    last_click_node = Some(i);
                }
            }
        }

        if rl.is_mouse_button_released(MouseButton::MOUSE_BUTTON_LEFT) {
            dragging_node = None;
        }
        if editor_open {
            editor::input_handler::handle_input(&mut rl);
        }

        if let Some(i) = dragging_node {
            nodes[i].position = mouse_pos;
            nodes[i].velocity = Vector2::zero();
        }
        if rl.is_key_pressed(KeyboardKey::KEY_ESCAPE) {
            editor_open = false;
        }
        // --- Update Phase ---
        let delta_time = rl.get_frame_time(); // Time passed since last frame

        let repulsion_k = 15000.0_f32;
        let spring_k = 0.05_f32;
        let rest_length = 200.0_f32;
        let damping = 0.95_f32;

        let mut forces = vec![Vector2::zero(); nodes.len()];

        for i in 0..nodes.len() {
            for j in 0..nodes.len() {
                if i == j {
                    continue;
                }
                let pi = nodes[i].position;
                let pj = nodes[j].position;
                let diff = pi - pj;
                let dist = diff.length().max(1.0);
                forces[i] += (diff / dist) * (repulsion_k / (dist * dist));
            }
        }
        let center = Vector2::new(width as f32 / 2.0, height as f32 / 2.0);
        let gravity_k = 0.1_f32;

        for i in 0..nodes.len() {
            let diff = center - nodes[i].position;
            forces[i] += diff * gravity_k;
        }
        for edge in &edges {
            let pi = nodes[edge.n1].position;
            let pj = nodes[edge.n2].position;
            let diff = pj - pi;
            let dist = diff.length().max(1.0);
            let force = spring_k * (dist - rest_length);
            let direction = diff / dist;
            forces[edge.n1] += direction * force;
            forces[edge.n2] -= direction * force;
        }
        for i in 0..nodes.len() {
            if Some(i) == dragging_node {
                continue;
            }
            forces[i] += Vector2::new(
                rng.random_range(-150.0..150.0),
                rng.random_range(-150.0..150.0),
            );
        }
        for (i, node) in nodes.iter_mut().enumerate() {
            if Some(i) == dragging_node {
                continue;
            }
            node.velocity = (node.velocity + forces[i] * delta_time) * damping;
            node.position += node.velocity * delta_time;
        }

        // --- Draw Phase ---
        let mut d = rl.begin_drawing(&thread);
        d.clear_background(Color::BLACK);

        // Draw each edge
        // for edge in &edges {
        //     if let (Ok(n1), Ok(n2)) = (edge.n1.try_borrow(), edge.n2.try_borrow()) {
        //         d.draw_line_ex(n1.position, n2.position, 4., Color::WHITE);
        //     }
        // }

        // // Draw each node
        // for node in &nodes {
        //     if let Ok(n) = node.try_borrow() {
        //         d.draw_circle_v(n.position, n.radius, n.color);
        //     }
        // }
        for edge in &edges {
            let n1 = &nodes[edge.n1];
            let n2 = &nodes[edge.n2];
            d.draw_line_ex(n1.position, n2.position, 10., Color::WHITE);
        }

        for node in &nodes {
            d.draw_circle_v(node.position, node.radius, node.color);
        }

        // Editor
        editor::renderer::draw(d, editor_open, editor_dimentions);
    }
}
