use crate::config::*;
use crate::filesystem;
use rand::prelude::*;
use raylib::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

pub static DRAGGING_NODE: RwLock<Option<usize>> = RwLock::new(None);
pub static HOVER_NODE: RwLock<Option<usize>> = RwLock::new(None);
pub static NODES: RwLock<Vec<Node>> = RwLock::new(Vec::<Node>::new());
pub static EDGES: RwLock<Vec<Edge>> = RwLock::new(Vec::<Edge>::new());

pub static DIR_PATH: RwLock<PathBuf> = RwLock::new(PathBuf::new());
pub static SELECTED_NODE: RwLock<Option<usize>> = RwLock::new(None);
pub static EDITING_NODE: RwLock<Option<usize>> = RwLock::new(None);
pub static DELETE_PENDING: RwLock<bool> = RwLock::new(false);
pub static ADDING_NOTE: RwLock<bool> = RwLock::new(false);
pub static ADDING_NAME: RwLock<String> = RwLock::new(String::new());

pub struct Node {
    pub radius: f32,
    pub color: Color,
    pub position: Vector2,
    pub velocity: Vector2,
    pub name: String,
    pub path: PathBuf,
}

pub struct Edge {
    pub n1: usize,
    pub n2: usize,
    pub direction: i32,
}

pub fn generate_nodes_from_directory(dir: &Path) {
    let files = filesystem::scan_directory(dir);
    let mut rng = rand::rng();
    let num_edges = (files.len() / 2).max(1);

    let mut nodes = NODES.write().unwrap();
    let mut edges = EDGES.write().unwrap();

    for file in &files {
        let name = file
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();

        nodes.push(Node {
            radius: 5.0,
            color: Color::WHITE,
            position: Vector2::new(
                rng.random_range((WIDTH as f32 / 2. - 100.)..(WIDTH as f32 / 2. + 100.)),
                rng.random_range((HEIGHT as f32 / 2. - 100.)..(HEIGHT as f32 / 2. + 100.)),
            ),
            velocity: Vector2::new(0.0, 0.0),
            name,
            path: file.clone(),
        });
    }

    let num_nodes = nodes.len();
    for _ in 0..num_edges {
        if num_nodes < 2 {
            break;
        }
        let n1 = rng.random_range(0..num_nodes);
        let mut n2 = rng.random_range(0..num_nodes);
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

pub fn add_node(dir: &Path, filename: &str) -> usize {
    let mut rng = rand::rng();

    // Normalize the stem (strip .md extension if provided)
    let stem = filename.trim_end_matches(".md");
    let stem = if stem.is_empty() { "untitled" } else { stem };
    let filename = filesystem::unique_filename(dir, stem);
    let file_path = dir.join(&filename);
    let title = filename.trim_end_matches(".md");

    filesystem::create_file(&file_path, &format!("# {}\n", title));

    let mut nodes = NODES.write().unwrap();
    let mut edges = EDGES.write().unwrap();

    let idx = nodes.len();
    nodes.push(Node {
        radius: 5.0,
        color: Color::WHITE,
        position: Vector2::new(
            rng.random_range((WIDTH as f32 / 2. - 50.)..(WIDTH as f32 / 2. + 50.)),
            rng.random_range((HEIGHT as f32 / 2. - 50.)..(HEIGHT as f32 / 2. + 50.)),
        ),
        velocity: Vector2::new(0.0, 0.0),
        name: filename,
        path: file_path,
    });

    // Connect to a random existing node if possible
    if idx > 0 {
        let n1 = rng.random_range(0..idx);
        edges.push(Edge {
            n1,
            n2: idx,
            direction: 1,
        });
    }

    idx
}

pub fn remove_node(idx: usize) {
    let mut nodes = NODES.write().unwrap();
    let mut edges = EDGES.write().unwrap();

    if idx >= nodes.len() {
        return;
    }

    let path = nodes[idx].path.clone();
    filesystem::delete_file(&path);

    nodes.remove(idx);

    // Remove edges referencing this node and remap indices > idx
    edges.retain(|e| e.n1 != idx && e.n2 != idx);
    for edge in edges.iter_mut() {
        if edge.n1 > idx {
            edge.n1 -= 1;
        }
        if edge.n2 > idx {
            edge.n2 -= 1;
        }
    }
}

pub fn update_forces(rl: &mut RaylibHandle) {
    let mut nodes = NODES.write().unwrap();
    let edges = EDGES.read().unwrap();

    let dragging_node = DRAGGING_NODE.read().unwrap();
    let delta_time = rl.get_frame_time();

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
            forces[i] += diff * (repulsion_k / (dist * dist * dist));
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
        let direction = diff.normalize();
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
