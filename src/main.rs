use rand::prelude::*;
use raylib::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

mod config;
mod editor;

struct Node {
    radius: f32, // Changed to f32 to match Raylib's draw_circle_v expectations
    color: Color,
    position: Vector2,
    velocity: Vector2, // Changed to a Vector2 so they can move in 2D space
}

struct Edge {
    n1: Rc<RefCell<Node>>,
    n2: Rc<RefCell<Node>>,
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
    let mut nodes: Vec<Rc<RefCell<Node>>> = Vec::new();
    let mut edges: Vec<Edge> = Vec::new();

    // Generating dummy nodes
    for _ in 0..num_nodes {
        // Generate a random angle for movement direction
        let angle: f32 = rng.random_range(0.0..std::f32::consts::TAU);
        let speed: f32 = rng.random_range(50.0..150.0); // Pixels per second

        nodes.push(Rc::new(RefCell::new(Node {
            radius: rng.random_range(5.0..=30.0),
            color: *colors.choose(&mut rng).unwrap(),
            position: Vector2::new(
                rng.random_range(0.0..width as f32),
                rng.random_range(0.0..height as f32),
            ),
            // Velocity split into X and Y components based on the angle
            velocity: Vector2::new(angle.cos() * speed, angle.sin() * speed),
        })));
    }

    // Generate dummy edges
    edges.push(Edge {
        n1: nodes[0].clone(),
        n2: nodes[1].clone(),
        direction: 1,
    });

    edges.push(Edge {
        n1: nodes[5].clone(),
        n2: nodes[1].clone(),
        direction: 1,
    });

    edges.push(Edge {
        n1: nodes[3].clone(),
        n2: nodes[8].clone(),
        direction: 1,
    });

    // 2. The Main Game Loop
    while !rl.window_should_close() {
        // Read user input

        // Toggle editor
        if rl.is_mouse_button_pressed(MouseButton::MOUSE_BUTTON_LEFT) {
            print!("Button clicked!");
            editor_open = !editor_open;
        }

        while let Some(ch) = rl.get_char_pressed() {
            editor_buffer.push(char::from_u32(ch as u32).unwrap());
        }

        // --- Update Phase ---
        let delta_time = rl.get_frame_time(); // Time passed since last frame

        for node in nodes.iter_mut() {
            if let Ok(mut n) = node.try_borrow_mut() {
                // Move the node based on velocity and delta time

                let n = &mut *n;

                n.position += n.velocity * delta_time;

                // Simple screen bounce collision
                if n.position.x - n.radius < 0.0 || n.position.x + n.radius > width as f32 {
                    n.velocity.x *= -1.0;
                }
                if n.position.y - n.radius < 0.0 || n.position.y + n.radius > height as f32 {
                    n.velocity.y *= -1.0;
                }
            }
        }

        // --- Draw Phase ---
        let mut d = rl.begin_drawing(&thread);
        d.clear_background(Color::BLACK);

        // Draw each edge
        for edge in &edges {
            if let (Ok(n1), Ok(n2)) = (edge.n1.try_borrow(), edge.n2.try_borrow()) {
                d.draw_line_ex(n1.position, n2.position, 4., Color::WHITE);
            }
        }

        // Draw each node
        for node in &nodes {
            if let Ok(n) = node.try_borrow() {
                d.draw_circle_v(n.position, n.radius, n.color);
            }
        }

        // Editor
        editor::renderer::draw(
            d,
            editor_open,
            editor_dimentions,
            &editor_buffer.to_string(),
        );
    }
}
