use crate::config::*;
use rand::prelude::*;
use raylib::prelude::*;
use std::sync::RwLock;

pub static DRAGGING_NODE: RwLock<Option<usize>> = RwLock::new(None);
pub static NODES: RwLock<Vec<Node>> = RwLock::new(Vec::<Node>::new());
pub static EDGES: RwLock<Vec<Edge>> = RwLock::new(Vec::<Edge>::new());

pub struct Node {
    pub radius: f32, // Changed to f32 to match Raylib's draw_circle_v expectations
    pub color: Color,
    pub position: Vector2,
    pub velocity: Vector2, // Changed to a Vector2 so they can move in 2D space
}

pub struct Edge {
    pub n1: usize,
    pub n2: usize,
    pub direction: i32,
}

pub fn generate_random_nodes() {
    let num_nodes = 100;
    let num_edges = 50;
    let mut rng = rand::rng();

    let mut nodes = NODES.write().unwrap();
    let mut edges = EDGES.write().unwrap();

    println!("HERE");
    // Generating dummy nodes
    for _ in 0..num_nodes {
        // Generate a random angle for movement direction

        nodes.push(Node {
            radius: rng.random_range(5.0..=5.0),
            color: Color::WHITE,
            position: Vector2::new(
                rng.random_range((WIDTH as f32 / 2. - 50.)..(WIDTH as f32 / 2. + 50. as f32)),
                rng.random_range((HEIGHT as f32 / 2. - 50.)..(HEIGHT as f32 / 2. + 50. as f32)),
            ),
            // Velocity split into X and Y components based on the angle
            velocity: Vector2::new(0.0, 0.0),
        });
    }

    for _ in 0..num_edges {
        let n1 = rng.random_range(0..num_nodes);
        let mut n2 = rng.random_range(0..num_nodes);

        // Prevent self-loops (node connecting to itself)
        while n1 == n2 {
            n2 = rng.random_range(0..num_nodes);
        }

        edges.push(Edge {
            n1,
            n2,
            direction: 1,
        });
    }
}

pub fn update_forces(rl: &mut RaylibHandle) {
    let mut nodes = NODES.write().unwrap();
    let edges = EDGES.read().unwrap();

    let dragging_node = DRAGGING_NODE.read().unwrap();
    let delta_time = rl.get_frame_time(); // Time passed since last frame

    let repulsion_k = 25000.0_f32;
    let spring_k = 0.90;
    let rest_length = 20.0_f32;
    let damping = 0.95;
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

    let center = Vector2::new(WIDTH as f32 / 2.0, HEIGHT as f32 / 2.0);
    let gravity_k = 0.1_f32;

    for i in 0..nodes.len() {
        let diff = center - nodes[i].position;
        forces[i] += diff * gravity_k;
    }

    for edge in edges.iter() {
        let pi = nodes[edge.n1].position;
        let pj = nodes[edge.n2].position;
        let diff = pj - pi;
        let dist = diff.length().max(1.0);
        let force = spring_k * (dist - rest_length);
        let direction = diff / dist;
        forces[edge.n1] += direction * force;
        forces[edge.n2] -= direction * force;
    }
    for (i, node) in nodes.iter_mut().enumerate() {
        if Some(i) == *dragging_node {
            continue;
        }
        node.velocity = (node.velocity + forces[i] * delta_time) * damping;
        node.position += node.velocity * delta_time;
    }
}
