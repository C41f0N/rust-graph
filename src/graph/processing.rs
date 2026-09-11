use crate::config::*;
use crate::filesystem;
use crate::frontmatter;
use rand::prelude::*;
use raylib::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::RwLock;

// Base node disc radius (world units). Large enough that nodes read clearly at
// the default zoom, yet below the spring rest length so force-laid-out graphs
// don't overlap.
pub const NODE_BASE_RADIUS: f32 = 7.0;
pub const NODE_MAX_RADIUS: f32 = 15.0;
// Radius growth per connection (edges are treated as bidirectional): BASE +
// GROWTH*sqrt(degree). Sub-linear on purpose, so node size still signals
// hub-ness without exploding linearly.
pub const NODE_RADIUS_GROWTH: f32 = 2.0;

// Extra clearance every edge's spring keeps between the two discs it joins.
// Added to the combined radii below, so every pair settles well beyond
// touching regardless of how big the individual nodes are.
const EDGE_REST_GAP: f32 = 60.0;

// Repulsion interaction radius (world units). Pairs closer than this feel each
// other's repulsion; beyond it the force is zero, so the spatial grid never
// checks them. Bigger spreads every cluster out, smaller keeps clusters
// compact while still preventing nodes from touching.
const REPULSION_RADIUS: f32 = 360.0;
// Floor applied to the pair distance inside the 1/d^2 law. Deliberately small
// (NOT near the radius — a large floor flattens the force into a constant):
// it only avoids a divide-by-zero when two discs coincide, so the inverse
// square term can still grow as nodes approach and shove them apart.
const REPULSION_MIN_DIST: f32 = 4.0;
const REPULSION_K: f32 = 8000.0;
// Cap on a single pair's repulsion acceleration, so an initial pile-up cannot
// fling nodes across the screen in one frame.
const REPULSION_MAX_MAG: f32 = 1600.0;

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

// Right-click context menu on the graph. CONTEXT_NODE targets a node;
// CONTEXT_EMPTY is the empty-space menu (Add Node, more items may follow).
// Exactly one of the two is open at a time.
pub static CONTEXT_NODE: RwLock<Option<usize>> = RwLock::new(None);
pub static CONTEXT_EMPTY: RwLock<bool> = RwLock::new(false);
pub static CONTEXT_POS: RwLock<(i32, i32)> = RwLock::new((0, 0));
pub static RENAMING: RwLock<bool> = RwLock::new(false);
pub static RENAME_NAME: RwLock<String> = RwLock::new(String::new());

// Sub-graph navigation: stack of previous directory paths so the breadcrumb
// trail can jump back to any ancestor level.
pub static NAV_STACK: RwLock<Vec<PathBuf>> = RwLock::new(Vec::new());
// Set by the renderer when a breadcrumb component is clicked; consumed once
// by the input handler so the click logic stays in the draw frame where
// text::measure is available.
pub static BREADCRUMB_CLICK: RwLock<Option<usize>> = RwLock::new(None);

// Set by the node context menu's "Set Header Image" action; consumed once by
// main.rs which spawns the native file dialog on a background thread.
pub static HEADER_PICK_REQUEST: RwLock<Option<usize>> = RwLock::new(None);
// Guards against stacking dialogs: set while a picker thread is live, cleared
// by that thread when the dialog closes.
pub static HEADER_PICK_ACTIVE: AtomicBool = AtomicBool::new(false);
// (node index, absolute path of the chosen file) written by the picker
// thread; consumed by main.rs on the main thread where file ops and GL
// texture loading belong.
pub static HEADER_PICK_RESULT: RwLock<Option<(usize, PathBuf)>> = RwLock::new(None);

pub struct Node {
    pub radius: f32,
    pub color: Color,
    pub position: Vector2,
    pub velocity: Vector2,
    pub name: String,
    pub file_name: String,
    pub path: PathBuf,
    pub header: Option<String>,
    pub has_subgraph: bool,
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
            radius: NODE_BASE_RADIUS,
            color: Color::WHITE,
            position: Vector2::new(
                rng.random_range((WIDTH as f32 / 2. - 100.)..(WIDTH as f32 / 2. + 100.)),
                rng.random_range((HEIGHT as f32 / 2. - 100.)..(HEIGHT as f32 / 2. + 100.)),
            ),
            velocity: Vector2::new(0.0, 0.0),
            name,
            file_name,
            path: file.clone(),
            header: None,
            has_subgraph: filesystem::is_dir(&filesystem::subgraph_dir(file)),
        });
    }

    drop(nodes);
    drop(edges);
    rebuild_edges();

    // The whole node set was rebuilt, so any index into the previous set is
    // stale. Clear selection/drag/context state to avoid pointing at the
    // wrong node (the delete prompt in particular indexes nodes[idx]).
    *SELECTED_NODE.write().unwrap() = None;
    *EDITING_NODE.write().unwrap() = None;
    *DRAGGING_NODE.write().unwrap() = None;
    *HOVER_NODE.write().unwrap() = None;
    *DELETE_PENDING.write().unwrap() = false;
    *CONTEXT_NODE.write().unwrap() = None;
    *CONTEXT_EMPTY.write().unwrap() = false;

    // Frame the freshly-laid-out graph at a comfortable default zoom. Fewer
    // nodes allow a closer view; the baseline (the max) keeps small graphs from
    // looking like a tiny speck in the middle of the screen. Node discs are
    // NODE_BASE_RADIUS, so zoom scales them to readable on-screen sizes.
    let node_count = NODES.read().unwrap().len();
    let zoom = (2.2_f32 / (node_count as f32).sqrt()).clamp(0.3, 2.2);
    let mut camera = crate::graph::renderer::CAMERA.write().unwrap();
    camera.zoom = zoom;
    // Re-centre the camera so the new graph appears in the middle of the
    // screen regardless of where the previous graph was panned.
    camera.target = Vector2::new(WIDTH as f32 / 2.0, HEIGHT as f32 / 2.0);
}

// Navigate into a sub-graph folder. Pushes the current directory onto the
// navigation stack so the breadcrumb trail can later return here.
pub fn navigate_into(subdir_name: &str) {
    let mut stack = NAV_STACK.write().unwrap();
    let mut dir = DIR_PATH.write().unwrap();
    stack.push(dir.clone());
    *dir = dir.join(subdir_name);
}

// Jump back to a specific ancestor level in the breadcrumb trail (0 = root).
pub fn navigate_to_level(level: usize) {
    let mut stack = NAV_STACK.write().unwrap();
    if level < stack.len() {
        let new_dir = stack[level].clone();
        stack.truncate(level);
        *DIR_PATH.write().unwrap() = new_dir;
    }
}

// The graph's root directory: the first breadcrumb entry when inside a
// sub-graph, otherwise the current directory. Assets live under this root.
pub fn project_root() -> PathBuf {
    let stack = NAV_STACK.read().unwrap();
    stack
        .first()
        .cloned()
        .unwrap_or_else(|| DIR_PATH.read().unwrap().clone())
}

// Resolve a node header target (stored relative to the project root, e.g.
// "assets/name.png") to an absolute path on disk.
pub fn resolve_header_path(header: &str) -> Option<PathBuf> {
    let h = header.trim();
    if h.is_empty() {
        return None;
    }
    Some(project_root().join(h))
}

// Write `raw_header` (a [[...]]-wrapped target) into the note's frontmatter
// and refresh the node cache/edges so the graph picks up the change.
pub fn attach_header(idx: usize, raw_header: &str) -> bool {
    let path = {
        let nodes = NODES.read().unwrap();
        nodes.get(idx).map(|n| n.path.clone())
    };
    let Some(path) = path else {
        return false;
    };
    let content = filesystem::read_file(&path);
    let new_content = frontmatter::upsert_header(&content, raw_header);
    if new_content == content {
        return false;
    }
    filesystem::write_file(&path, &new_content);
    rebuild_edges();
    true
}

// Build directed edges from [[wikilink]] references in the .md files.
// [[target]] in file A creates a directed edge A -> target.
// A link may name the target with or without the extension: [[x]] and
// [[x.md]] both resolve to x.md, and any other extension (images, bare
// names that match no file) never creates an edge.
// Duplicate and self-links are ignored.
pub fn rebuild_edges() {
    let mut nodes = NODES.write().unwrap();
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

    // Re-parse every node: update header from frontmatter, build edges only
    // from the body (everything after the frontmatter block).
    let mut fm_headers: Vec<Option<String>> = Vec::with_capacity(nodes.len());
    for (i, node) in nodes.iter_mut().enumerate() {
        let content = filesystem::read_file(&node.path);

        // Re-check whether a companion sub-graph folder exists (rename/delete
        // can change it) so the graph always reflects the filesystem.
        node.has_subgraph = filesystem::is_dir(&filesystem::subgraph_dir(&node.path));

        // Parse frontmatter and cache the header target on the node.
        let fm = frontmatter::parse(&content);
        node.header = fm.as_ref().and_then(|f| f.header.clone());
        fm_headers.push(node.header.clone());

        // Slice past frontmatter for content-link extraction.
        let body = if let Some(fm) = &fm {
            if fm.end_byte <= content.len() {
                &content[fm.end_byte..]
            } else {
                &content
            }
        } else {
            &content
        };

        let links = filesystem::parse_links(body);

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

    // Size each node by its total connections, treated as bidirectional: every
    // edge counts towards both ends, so a note that many others [[link]] to
    // grows just like one that links out to many. The growth is sub-linear
    // (sqrt of the connection count) so hubs still stand out but diminishing
    // returns stop a 20-link note from dominating the graph. Re-run on every
    // edge rebuild so add/remove/rename/navigation all keep sizes current.
    let mut degree = vec![0u32; nodes.len()];
    for edge in edges.iter() {
        degree[edge.n1] += 1;
        degree[edge.n2] += 1;
    }
    for (node, &count) in nodes.iter_mut().zip(&degree) {
        node.radius =
            (NODE_BASE_RADIUS + NODE_RADIUS_GROWTH * (count as f32).sqrt()).min(NODE_MAX_RADIUS);
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
        radius: NODE_BASE_RADIUS,
        color: Color::WHITE,
        position: Vector2::new(
            rng.random_range((WIDTH as f32 / 2. - 50.)..(WIDTH as f32 / 2. + 50.)),
            rng.random_range((HEIGHT as f32 / 2. - 50.)..(HEIGHT as f32 / 2. + 50.)),
        ),
        velocity: Vector2::new(0.0, 0.0),
        name: stem.to_string(),
        file_name: filename,
        path: file_path,
        header: None,
        has_subgraph: false,
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

// Rename a note's .md file (and its companion sub-graph folder, if any),
// its node label, and every [[wikilink]] that points at it from the .md
// files in the current directory. Returns false if the new name is empty,
// the target file exists, or (for notes with a sub-graph) the target
// folder exists.
pub fn rename_node(idx: usize, new_name: &str) -> bool {
    let mut nodes = NODES.write().unwrap();
    if idx >= nodes.len() {
        return false;
    }
    let new_stem = new_name.trim().trim_end_matches(".md").to_string();
    if new_stem.is_empty() {
        return false;
    }

    let old_path = nodes[idx].path.clone();
    let old_stem = old_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let sub_folder = filesystem::subgraph_dir(&old_path);
    let has_sub = filesystem::is_dir(&sub_folder);

    // A note with a sub-graph needs the destination folder free too; bail
    // before touching the file so the note survives a conflicting name.
    if has_sub && sub_folder.with_file_name(&new_stem).exists() {
        return false;
    }

    if !filesystem::rename_file(&old_path, &new_stem) {
        return false;
    }

    // Rename the companion sub-graph folder. If this somehow fails, roll the
    // file rename back so the pair stays consistent.
    if has_sub && !filesystem::rename_dir(&sub_folder, &new_stem) {
        filesystem::rename_file(&old_path.with_file_name(format!("{}.md", new_stem)), &old_stem);
        return false;
    }

    // Rewire [[old_stem]] / [[old_stem.md]] references in every .md file in
    // the current directory so they follow the note to its new name.
    let dir = DIR_PATH.read().unwrap().clone();
    for file in filesystem::scan_directory(&dir) {
        let content = filesystem::read_file(&file);
        let rewritten = filesystem::replace_links(&content, &old_stem, &new_stem);
        if rewritten != content {
            filesystem::write_file(&file, &rewritten);
        }
    }

    nodes[idx].file_name = format!("{}.md", new_stem);
    nodes[idx].name = new_stem;
    nodes[idx].path = old_path.with_file_name(nodes[idx].file_name.clone());
    nodes[idx].has_subgraph = has_sub;
    drop(nodes);
    rebuild_edges();
    true
}

// Repulsion between every pair closer than REPULSION_RADIUS, computed with a
// spatial grid. Cell size equals the interaction radius, so a repelling pair
// can only occupy the same cell or two adjacent ones: scanning the 3x3 cell
// neighborhood of each node finds every pair within range (and none beyond,
// where the force would be zero anyway). Returns one accumulated force per
// node. Pure and unit-testable; the graph's own repulsion constants are used.
fn repulsion_forces(positions: &[Vector2]) -> Vec<Vector2> {
    let mut forces = vec![Vector2::zero(); positions.len()];

    let mut grid: std::collections::HashMap<(i32, i32), Vec<usize>> =
        std::collections::HashMap::with_capacity(positions.len());
    for (i, pos) in positions.iter().enumerate() {
        let cell = (
            (pos.x / REPULSION_RADIUS).floor() as i32,
            (pos.y / REPULSION_RADIUS).floor() as i32,
        );
        grid.entry(cell).or_default().push(i);
    }

    for i in 0..positions.len() {
        let pi = positions[i];
        let cx = (pi.x / REPULSION_RADIUS).floor() as i32;
        let cy = (pi.y / REPULSION_RADIUS).floor() as i32;
        for cy2 in cy - 1..=cy + 1 {
            for cx2 in cx - 1..=cx + 1 {
                let Some(cell) = grid.get(&(cx2, cy2)) else {
                    continue;
                };
                for &j in cell {
                    if i == j {
                        continue;
                    }
                    let diff = pi - positions[j];
                    let dist = diff.length();
                    if dist >= REPULSION_RADIUS {
                        continue;
                    }
                    let d = dist.max(REPULSION_MIN_DIST);
                    // Softened inverse-square repulsion that vanishes smoothly
                    // at the interaction radius (never a hard pop at the edge).
                    let mag = (REPULSION_K / (d * d) * (1.0 - dist / REPULSION_RADIUS))
                        .min(REPULSION_MAX_MAG);
                    forces[i] += diff * (1.0 / d) * mag;
                }
            }
        }
    }
    forces
}

pub fn update_forces(rl: &mut RaylibHandle) {
    let mut nodes = NODES.write().unwrap();
    let edges = EDGES.read().unwrap();

    let dragging_node = DRAGGING_NODE.read().unwrap();
    let delta_time = rl.get_frame_time();

    let spring_k = 0.90;
    let damping = 0.95;

    let positions: Vec<Vector2> = nodes.iter().map(|n| n.position).collect();
    let mut forces = repulsion_forces(&positions);

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
        let dist = diff.length();
        let direction = if dist > 0.001 { diff.scale(1.0 / dist) } else { Vector2::zero() };
        // Rest length scales with the two disc radii plus a gap, so hubs (which
        // grow) keep the same clear distance as the smallest nodes.
        let rest = nodes[edge.n1].radius + nodes[edge.n2].radius + EDGE_REST_GAP;
        let force = spring_k * (dist - rest);
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

#[cfg(test)]
mod tests {
    use super::*;

    // These tests drive the same process-global NAV_STACK/DIR_PATH statics, so
    // cargo's parallel test threads would stomp on each other. Serialize them.
    static TEST_NAV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn repulsion_is_local_to_the_radius() {
        let cluster = vec![
            Vector2::new(0.0, 0.0),
            Vector2::new(5.0, 0.0),
            Vector2::new(0.0, 5.0),
            Vector2::new(-3.0, -2.0),
        ];
        let f = repulsion_forces(&cluster);
        for i in 0..cluster.len() {
            assert!(f[i].length() > 0.0, "cluster members must repel each other");
        }

        // A pair further apart than the interaction radius feels nothing at
        // all. Scaled off the constant so it tracks future tuning.
        let far = vec![
            Vector2::zero(),
            Vector2::new(REPULSION_RADIUS * 2.0, REPULSION_RADIUS * 2.0),
        ];
        let g = repulsion_forces(&far);
        assert_eq!(g[0].length(), 0.0);
        assert_eq!(g[1].length(), 0.0);
    }

    #[test]
    fn grid_repulsion_matches_brute_force() {
        let mut rng = rand::rng();
        let positions: Vec<Vector2> = (0..200)
            .map(|_| {
                Vector2::new(
                    rng.random_range(-400.0..400.0),
                    rng.random_range(-400.0..400.0),
                )
            })
            .collect();

        let fast = repulsion_forces(&positions);

        let mut brute = vec![Vector2::zero(); positions.len()];
        for i in 0..positions.len() {
            for j in 0..positions.len() {
                if i == j {
                    continue;
                }
                let diff = positions[i] - positions[j];
                let dist = diff.length();
                if dist >= REPULSION_RADIUS {
                    continue;
                }
                let d = dist.max(REPULSION_MIN_DIST);
                let mag = (REPULSION_K / (d * d) * (1.0 - dist / REPULSION_RADIUS))
                    .min(REPULSION_MAX_MAG);
                brute[i] += diff * (1.0 / d) * mag;
            }
        }

        for (a, b) in fast.iter().zip(brute.iter()) {
            assert!(
                (a.x - b.x).abs() < 1e-2 && (a.y - b.y).abs() < 1e-2,
                "grid and brute-force repulsion disagree"
            );
        }
    }

    #[test]
    fn settled_pairs_stay_clear_regardless_of_size() {
        // Integrate the same spring + repulsion forces update_forces uses (no
        // gravity/camera) for a connected pair of very different sizes. The
        // spring rest length is radii + EDGE_REST_GAP and repulsion balances
        // just beyond it, so the settled gap is never an overlap.
        let r1 = NODE_BASE_RADIUS;
        let r2 = NODE_MAX_RADIUS;
        let mut positions = vec![Vector2::new(0.0, 0.0), Vector2::new(3.0, 0.0)];
        let mut velocities = vec![Vector2::zero(), Vector2::zero()];
        let spring_k = 0.90_f32;
        let damping = 0.55_f32;
        let dt = 0.01_f32;
        for _ in 0..5000 {
            let mut forces = repulsion_forces(&positions);
            let diff = positions[1] - positions[0];
            let dist = diff.length().max(1e-4);
            let direction = diff.scale(1.0 / dist);
            let rest = r1 + r2 + EDGE_REST_GAP;
            let force = spring_k * (dist - rest);
            forces[0] += direction * force;
            forces[1] -= direction * force;
            for i in 0..2 {
                velocities[i] = (velocities[i] + forces[i] * dt) * damping;
                positions[i] += velocities[i] * dt;
            }
        }

        let sep = (positions[1] - positions[0]).length();
        assert!(sep >= r1 + r2 + 4.0, "connected pair must sit clearly apart, got {sep}");
        // Disconnected pairs only have repulsion; starting overlapped they too
        // shove apart and never re-collapse.
        let mut p2 = vec![Vector2::new(0.0, 0.0), Vector2::new(1.0, 1.0)];
        let mut v2 = vec![Vector2::zero(), Vector2::zero()];
        for _ in 0..3000 {
            let forces = repulsion_forces(&p2);
            for i in 0..2 {
                v2[i] = (v2[i] + forces[i] * dt) * damping;
                p2[i] += v2[i] * dt;
            }
        }
        let sep2 = (p2[1] - p2[0]).length();
        assert!(sep2 >= 2.0 * NODE_BASE_RADIUS, "disconnected pair must not overlap, got {sep2}");
    }

    #[test]
    fn node_size_counts_connections_in_both_directions() {
        let _guard = TEST_NAV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join("rg_radius_test");
        let _ = std::fs::remove_dir_all(&dir);
        filesystem::create_dir(&dir);
        filesystem::write_file(&dir.join("hub.md"), "# Hub\n\n[[a]]\n[[b]]\n[[c]]\n[[d]]\n");
        for x in ["a", "b", "c", "d"] {
            filesystem::write_file(&dir.join(format!("{x}.md")), &format!("# {x}\n\n[[hub]]\n"));
        }
        filesystem::write_file(&dir.join("leaf.md"), "# Leaf\n\n[[hub]]\n");

        *DIR_PATH.write().unwrap() = dir.clone();
        NAV_STACK.write().unwrap().clear();
        generate_nodes_from_directory(&dir);

        let nodes = NODES.read().unwrap();
        let hub = nodes.iter().find(|n| n.name == "hub").unwrap();
        let leaf = nodes.iter().find(|n| n.name == "leaf").unwrap();
        // hub: 4 outgoing links and 4 incoming ones (a..d all link back) plus
        // leaf's link = degree 9. A leaf linking only to it has degree 1, so
        // the inbound links must have grown the hub.
        assert!(
            hub.radius > leaf.radius,
            "a node many notes link to must outgrow a leaf (hub {} vs leaf {})",
            hub.radius,
            leaf.radius
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn navigate_into_pushes_and_changes_dir() {
        let _guard = TEST_NAV_LOCK.lock().unwrap();
        NAV_STACK.write().unwrap().clear();
        *DIR_PATH.write().unwrap() = PathBuf::from("/root");

        navigate_into("my-note");

        assert_eq!(*DIR_PATH.read().unwrap(), PathBuf::from("/root/my-note"));
        assert_eq!(*NAV_STACK.read().unwrap(), vec![PathBuf::from("/root")]);
    }

    #[test]
    fn navigate_to_level_truncates_stack() {
        let _guard = TEST_NAV_LOCK.lock().unwrap();
        NAV_STACK.write().unwrap().clear();
        *DIR_PATH.write().unwrap() = PathBuf::from("/root");
        navigate_into("a");
        navigate_into("b");
        navigate_into("c");

        navigate_to_level(1);
        assert_eq!(*DIR_PATH.read().unwrap(), PathBuf::from("/root/a"));
        assert_eq!(*NAV_STACK.read().unwrap(), vec![PathBuf::from("/root")]);

        // Out of range level is a no-op
        navigate_to_level(5);
        assert_eq!(*DIR_PATH.read().unwrap(), PathBuf::from("/root/a"));
    }

    #[test]
    fn rename_rewrites_links_in_other_notes() {
        let _guard = TEST_NAV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join("rg_rename_links_test");
        let _ = std::fs::remove_dir_all(&dir);
        filesystem::create_dir(&dir);
        filesystem::write_file(&dir.join("old-name.md"), "# Old\n");
        filesystem::write_file(&dir.join("a.md"), "see [[old-name]] and [[old-name.md]]\n");
        filesystem::write_file(&dir.join("b.md"), "keep [[other]]\n");

        *DIR_PATH.write().unwrap() = dir.clone();
        generate_nodes_from_directory(&dir);
        let idx = NODES
            .read().unwrap()
            .iter().position(|n| n.name == "old-name")
            .expect("node not found");

        assert!(rename_node(idx, "new-name"));

        assert_eq!(
            filesystem::read_file(&dir.join("a.md")),
            "see [[new-name]] and [[new-name]]\n"
        );
        assert_eq!(filesystem::read_file(&dir.join("b.md")), "keep [[other]]\n");
        assert!(dir.join("new-name.md").exists());
        assert!(!dir.join("old-name.md").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn attach_header_writes_frontmatter_and_refreshes_cache() {
        let _guard = TEST_NAV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join("rg_attach_header_test");
        let _ = std::fs::remove_dir_all(&dir);
        filesystem::create_dir(&dir);
        filesystem::write_file(&dir.join("a.md"), "# A\n");
        filesystem::write_file(&dir.join("b.md"), "# B\nlink [[a.md]]\n");

        *DIR_PATH.write().unwrap() = dir.clone();
        NAV_STACK.write().unwrap().clear();
        generate_nodes_from_directory(&dir);
        let idx = NODES
            .read().unwrap()
            .iter().position(|n| n.name == "a")
            .expect("node not found");

        assert!(attach_header(idx, "[[assets/pic.png]]"));
        assert_eq!(
            filesystem::read_file(&dir.join("a.md")),
            "---\nheader: [[assets/pic.png]]\n---\n# A\n"
        );
        // rebuild_edges refreshed the node cache.
        let node = &NODES.read().unwrap()[idx];
        assert_eq!(node.header.as_deref(), Some("assets/pic.png"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
