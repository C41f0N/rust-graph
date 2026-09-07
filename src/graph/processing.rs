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

// Right-click context menu on the graph.
pub static CONTEXT_NODE: RwLock<Option<usize>> = RwLock::new(None);
pub static CONTEXT_POS: RwLock<(i32, i32)> = RwLock::new((0, 0));
pub static RENAMING: RwLock<bool> = RwLock::new(false);
pub static RENAME_NAME: RwLock<String> = RwLock::new(String::new());

pub struct Node {
    pub radius: f32,
    pub color: Color,
    pub position: Vector2,
    pub velocity: Vector2,
    pub name: String,
    pub file_name: String,
    pub path: PathBuf,
}

pub struct Edge {
    pub n1: usize,
    pub n2: usize,
}

pub fn generate_nodes_from_directory(dir: &Path) {
    let files = filesystem::scan_directory(dir);
    let mut rng = rand::rng();

    let mut nodes = NODES.write().unwrap();
    let mut edges = EDGES.write().unwrap();
    nodes.clear();
    edges.clear();

    for file in &files {
        let file_name = file.file_name().unwrap().to_string_lossy().to_string();
        let name = file_name.trim_end_matches(".md").to_string();

        nodes.push(Node {
            radius: 5.0,
            color: Color::WHITE,
            position: Vector2::new(
                rng.random_range((WIDTH as f32 / 2. - 100.)..(WIDTH as f32 / 2. + 100.)),
                rng.random_range((HEIGHT as f32 / 2. - 100.)..(HEIGHT as f32 / 2. + 100.)),
            ),
            velocity: Vector2::new(0.0, 0.0),
            name,
            file_name,
            path: file.clone(),
        });
    }

    drop(nodes);
    drop(edges);
    rebuild_edges();

    // Zoom the initial view out as more nodes appear so the whole graph fits
    // on screen. Fewer nodes allow a closer, larger view.
    let node_count = NODES.read().unwrap().len();
    let zoom = (1.0 / (node_count as f32).sqrt()).clamp(0.15, 1.0);
    let mut camera = crate::graph::renderer::CAMERA.write().unwrap();
    camera.zoom = zoom;
}

// Build directed edges from [[wikilink]] references in the .md files.
// [[target]] in file A creates a directed edge A -> target.
// A link may name the target with or without the extension: [[x]] and
// [[x.md]] both resolve to x.md, and any other extension (images, bare
// names that match no file) never creates an edge.
// Duplicate and self-links are ignored.
pub fn rebuild_edges() {
    let nodes = NODES.read().unwrap();
    let mut edges = EDGES.write().unwrap();
    edges.clear();

    // Map from both the exact filename and its bare stem (extension
    // stripped) to node index, so extension-less [[x]] links resolve.
    let mut name_to_idx: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (i, node) in nodes.iter().enumerate() {
        name_to_idx.insert(node.file_name.clone(), i);
        let stem = node.file_name.trim_end_matches(".md");
        name_to_idx.insert(stem.to_string(), i);
    }

    // Keep a set of existing edges to avoid duplicates
    let mut seen: std::collections::HashSet<(usize, usize)> = std::collections::HashSet::new();

    for (i, node) in nodes.iter().enumerate() {
        let content = filesystem::read_file(&node.path);
        let links = filesystem::parse_links(&content);

        for link in links {
            // Resolve with the extension if given, otherwise as a bare stem.
            let target = link.strip_suffix(".md").unwrap_or(&link);
            if let Some(&j) = name_to_idx.get(target) {
                if i == j {
                    // Self-link, ignore
                    continue;
                }
                if seen.insert((i, j)) {
                    edges.push(Edge { n1: i, n2: j });
                }
            }
        }
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

    let idx = nodes.len();
    nodes.push(Node {
        radius: 5.0,
        color: Color::WHITE,
        position: Vector2::new(
            rng.random_range((WIDTH as f32 / 2. - 50.)..(WIDTH as f32 / 2. + 50.)),
            rng.random_range((HEIGHT as f32 / 2. - 50.)..(HEIGHT as f32 / 2. + 50.)),
        ),
        velocity: Vector2::new(0.0, 0.0),
        name: stem.to_string(),
        file_name: filename,
        path: file_path,
    });

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

// Rename a note's .md file (and its node label) to `new_name`. Returns
// false if the new name is empty or the target file already exists.
pub fn rename_node(idx: usize, new_name: &str) -> bool {
    let mut nodes = NODES.write().unwrap();
    if idx >= nodes.len() {
        return false;
    }
    let old_path = nodes[idx].path.clone();
    if !filesystem::rename_file(&old_path, new_name) {
        return false;
    }
    let new_stem = new_name.trim().trim_end_matches(".md").to_string();
    nodes[idx].file_name = format!("{}.md", new_stem);
    nodes[idx].name = new_stem;
    nodes[idx].path = old_path.with_file_name(nodes[idx].file_name.clone());
    drop(nodes);
    rebuild_edges();
    true
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
