use crate::config;
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

// Effective disc radius for a node of `degree`: the live radius scale
// multiplies every size, and radius variation compresses the degree growth
// toward zero (1.0 = all nodes uniform at the base size, 0.0 = the full
// base+growth spread). Reads the PARAM_* statics so the panel dials and the
// persisted .graph-params both flow through here.
fn radius_for(degree: u32) -> f32 {
    let scale = *PARAM_RADIUS_SCALE.read().unwrap();
    let variation = *PARAM_RADIUS_VARIATION.read().unwrap();
    let growth = NODE_RADIUS_GROWTH * (1.0 - variation);
    (NODE_BASE_RADIUS + growth * (degree as f32).sqrt()).min(NODE_MAX_RADIUS) * scale
}

// Re-apply the live radial controls (scale/variation) to `nodes` from the
// current edge degrees and report whether anything changed. Never locks NODES:
// the caller hands in its own (already-write-locked) slice - handle_input
// holds the NODES write lock for its whole frame, so locking here would
// self-deadlock. A true change needs the sim reheated afterwards.
pub fn apply_radii(nodes: &mut [Node]) -> bool {
    if nodes.is_empty() {
        return false;
    }
    let degree = {
        let edges = EDGES.read().unwrap();
        let mut degree = vec![0u32; nodes.len()];
        for edge in edges.iter() {
            degree[edge.n1] += 1;
            degree[edge.n2] += 1;
        }
        degree
    };
    let mut dirty = false;
    for (node, &count) in nodes.iter_mut().zip(&degree) {
        let r = radius_for(count);
        dirty |= (node.radius - r).abs() > 1e-4;
        node.radius = r;
    }
    dirty
}

// Spring rest gap derived from the spring strength: one dial (Spring
// Tightness) drives the whole spring. High strength = snug target distance
// (tight cluster); low strength = far target (loose, widely spread). At the
// default 0.40 this returns ~181, matching the old fixed 179 gap.
const SPRING_GAP_LO: f32 = 40.0;
const SPRING_GAP_RANGE: f32 = 480.0;
const SPRING_GAP_SHARPNESS: f32 = 6.0;
fn spring_rest_gap(spring_k: f32) -> f32 {
    SPRING_GAP_LO + SPRING_GAP_RANGE / (1.0 + SPRING_GAP_SHARPNESS * spring_k)
}

// Repulsion interaction radius (world units). Pairs closer than this feel each
// other's repulsion; beyond it the force is zero, so the spatial grid never
// checks them. Bigger spreads every cluster out, smaller keeps clusters
// compact.
const REPULSION_RADIUS: f32 = 652.0;
// Soft component (inverse-square of the pair distance): keeps clusters open
// and gives every node gentle breathing room under the cutoff. The only node
// separation force - nodes may momentarily squeeze close under spring tension,
// and that's accepted.
const REPULSION_K: f32 = 50000.0;

// Radial controls (force panel sliders 7-8). Radius scale multiplies every
// node disc; radius variation compresses the degree-based growth toward zero,
// so at 1.0 small and large nodes collapse onto one uniform size, at 0.0 the
// full base+growth spread returns.
const RADIUS_SCALE_DEFAULT: f32 = 1.0;
const RADIUS_VARIATION_DEFAULT: f32 = 0.0;

// Soft pull between non-linked pairs that share a repulsion grid cell. Done in
// the same spatial pass as the repulsion (no second O(n^2) scan) and kept
// weaker than the inverse-square repulsion so clusters stay open but coherent.
const NONLINK_ATTRACTION_DEFAULT: f32 = 0.05;

// Live-tunable force parameters. The statics below mirror the physical
// constants above (which stay as defaults/for tests) so a temporary debug
// panel can tweak them while the graph is running and watch the layout
// respond immediately.
pub static PARAM_SPRING_K: RwLock<f32> = RwLock::new(0.40);
pub static PARAM_DAMPING: RwLock<f32> = RwLock::new(0.95);
pub static PARAM_GRAVITY_K: RwLock<f32> = RwLock::new(0.04);
pub static PARAM_REPULSION_RADIUS: RwLock<f32> = RwLock::new(REPULSION_RADIUS);
pub static PARAM_REPULSION_K: RwLock<f32> = RwLock::new(REPULSION_K);
pub static PARAM_ALPHA_DECAY: RwLock<f32> = RwLock::new(ALPHA_DECAY);
pub static PARAM_RADIUS_SCALE: RwLock<f32> = RwLock::new(RADIUS_SCALE_DEFAULT);
pub static PARAM_RADIUS_VARIATION: RwLock<f32> = RwLock::new(RADIUS_VARIATION_DEFAULT);
pub static PARAM_NONLINK_ATTRACTION: RwLock<f32> = RwLock::new(NONLINK_ATTRACTION_DEFAULT);

// Temporary debug panel: a live switch to disable the alpha cooldown (and
// with it the settle-and-pause behaviour), plus the panel's visibility and
// the index of the slider currently being dragged.
pub static ALPHA_COOLING_ENABLED: RwLock<bool> = RwLock::new(true);
pub static SHOW_FORCE_PANEL: RwLock<bool> = RwLock::new(true);
pub static ACTIVE_SLIDER: RwLock<Option<usize>> = RwLock::new(None);

// Force panel geometry (screen-space pixels, unscaled; callers scale with
// config::scaled_size like every other piece of UI). TRACK_* are offsets from
// the panel's left edge.
pub const PANEL_X: i32 = 10;
pub const PANEL_Y: i32 = 60;
pub const PANEL_W: i32 = 260;
pub const PANEL_TITLE_H: i32 = 32;
pub const PANEL_ROW_H: i32 = 28;
pub const TRACK_LEFT: i32 = 100;
pub const TRACK_RIGHT: i32 = PANEL_W - 10;
pub const SLIDER_COUNT: usize = 9;

// (min, max) range of each slider, in the same order as the PARAM_* list.
// Index 0 is the spring (tightness), 1 damping, ... 8 is the non-link
// attraction.
pub const SLIDER_RANGES: [(f32, f32); SLIDER_COUNT] = [
    (0.0, 20.0),
    (0.5, 1.0),
    (0.0, 0.5),
    (50.0, 700.0),
    (0.0, 50000.0),
    (0.005, 0.05),
    (0.5, 2.0),
    (0.0, 1.0),
    (0.0, 0.01),
];

// Return True if the pointer is over the alpha-cooling toggle row (the first
// row of the panel body, just under the title bar).
pub fn hit_test_alpha_toggle(mx: f32, my: f32) -> bool {
    let px = PANEL_X as f32;
    let row_h = config::scaled_size(PANEL_ROW_H) as f32;
    mx >= px
        && mx <= px + config::scaled_size(PANEL_W) as f32
        && my >= panel_body_y() as f32
        && my <= panel_body_y() as f32 + row_h
}

// Return the index of the slider whose row the pointer is over, or None.
// Indexes run top-to-bottom: 0 = Spring Tightness ... 8 = Attraction. The
// whole row is the hit target (not just the thin track band) so grabbing a
// slider is forgiving; the renderer draws rows from the same helpers below.
pub fn hit_test_slider(mx: f32, my: f32) -> Option<usize> {
    let px = PANEL_X as f32;
    let row_h = config::scaled_size(PANEL_ROW_H) as f32;
    for idx in 0..SLIDER_COUNT {
        let row_y = slider_row_y(idx) as f32;
        if my >= row_y
            && my <= row_y + row_h
            && mx >= px
            && mx <= px + config::scaled_size(PANEL_W) as f32
        {
            return Some(idx);
        }
    }
    None
}

// Screen-space y of the panel body's first row (bottom of the title bar).
pub fn panel_body_y() -> i32 {
    PANEL_Y + config::scaled_size(PANEL_TITLE_H)
}

// Screen-space y of the row holding slider `idx` (0-based, sliders only; the
// alpha toggle owns the row directly above the first one).
pub fn slider_row_y(idx: usize) -> i32 {
    PANEL_Y + config::scaled_size(PANEL_TITLE_H) + config::scaled_size(PANEL_ROW_H) * (1 + idx as i32)
}

// Screen-space y of the "Respawn" button row (below the last slider).
pub fn respawn_button_y() -> i32 {
    PANEL_Y
        + config::scaled_size(PANEL_TITLE_H)
        + config::scaled_size(PANEL_ROW_H) * (1 + SLIDER_COUNT as i32)
}

// Return True if the pointer is over the Respawn button row.
pub fn hit_test_respawn_button(mx: f32, my: f32) -> bool {
    let px = PANEL_X as f32;
    let row_h = config::scaled_size(PANEL_ROW_H) as f32;
    mx >= px
        && mx <= px + config::scaled_size(PANEL_W) as f32
        && my >= respawn_button_y() as f32
        && my <= respawn_button_y() as f32 + row_h
}

// Re-initialize the current directory's graph from scratch: fresh phyllotaxis
// positions, rebuilt edges, reset camera/zoom, cleared selection, sim woken.
pub fn respawn_graph() {
    let dir = DIR_PATH.read().unwrap().clone();
    generate_nodes_from_directory(&dir);
}

// Map the pointer's x onto the slider at `idx`, store the resulting value, and
// (for the radial controls) resize discs on the caller's already-locked node
// slice. `nodes` must be the live NODES write guard - see apply_radii.
pub fn update_slider_from_mouse(idx: usize, mx: f32, nodes: &mut [Node]) {
    let px = PANEL_X as f32;
    let track_l = px + config::scaled_size(TRACK_LEFT) as f32;
    let track_r = px + config::scaled_size(TRACK_RIGHT) as f32;
    let t = ((mx - track_l) / (track_r - track_l)).clamp(0.0, 1.0);
    let Some((lo, hi)) = SLIDER_RANGES.get(idx) else {
        return;
    };
    let v = lo + t * (hi - lo);
    let value = match idx {
        0 => (v * 100.0).round() / 100.0,
        1 => (v * 100.0).round() / 100.0,
        2 => (v * 100.0).round() / 100.0,
        3 => v.round(),
        4 => (v / 100.0).round() * 100.0,
        5 => (v * 1000.0).round() / 1000.0,
        6 => (v * 100.0).round() / 100.0,
        7 => (v * 100.0).round() / 100.0,
        8 => (v * 100.0).round() / 100.0,
        _ => v,
    };
    match idx {
        0 => *PARAM_SPRING_K.write().unwrap() = value,
        1 => *PARAM_DAMPING.write().unwrap() = value,
        2 => *PARAM_GRAVITY_K.write().unwrap() = value,
        3 => *PARAM_REPULSION_RADIUS.write().unwrap() = value,
        4 => *PARAM_REPULSION_K.write().unwrap() = value,
        5 => *PARAM_ALPHA_DECAY.write().unwrap() = value,
        6 => *PARAM_RADIUS_SCALE.write().unwrap() = value,
        7 => *PARAM_RADIUS_VARIATION.write().unwrap() = value,
        8 => *PARAM_NONLINK_ATTRACTION.write().unwrap() = value,
        _ => {}
    }

    // Changing a radial control resizes every disc immediately.
    if idx == 6 || idx == 7 {
        if apply_radii(nodes) {
            wake_simulation();
        }
    }

    // Reheat the sim so the layout visibly responds to the new force: while
    // the button is held, each frame re-sets alpha to full so the graph stays
    // hot through the whole drag, then cools once released. This also wakes a
    // settled graph the first time a knob is touched.
    wake_simulation();
}

// ---- Parameter persistence ------------------------------------------------
// Slider-tuned force values survive a restart. Written as a tiny .graph-params
// key=value file next to the graph directory being viewed, so each folder can
// carry its own force settings. Unreadable/missing file = keep current values.

/// The 9 force values in slider order (spring_tightness ... nonlink_attraction).
pub fn param_values() -> [f32; 9] {
    [
        *PARAM_SPRING_K.read().unwrap(),
        *PARAM_DAMPING.read().unwrap(),
        *PARAM_GRAVITY_K.read().unwrap(),
        *PARAM_REPULSION_RADIUS.read().unwrap(),
        *PARAM_REPULSION_K.read().unwrap(),
        *PARAM_ALPHA_DECAY.read().unwrap(),
        *PARAM_RADIUS_SCALE.read().unwrap(),
        *PARAM_RADIUS_VARIATION.read().unwrap(),
        *PARAM_NONLINK_ATTRACTION.read().unwrap(),
    ]
}

/// Overwrite every force value from `values` (slider order).
pub fn set_param_values(values: [f32; 9]) {
    *PARAM_SPRING_K.write().unwrap() = values[0];
    *PARAM_DAMPING.write().unwrap() = values[1];
    *PARAM_GRAVITY_K.write().unwrap() = values[2];
    *PARAM_REPULSION_RADIUS.write().unwrap() = values[3];
    *PARAM_REPULSION_K.write().unwrap() = values[4];
    *PARAM_ALPHA_DECAY.write().unwrap() = values[5];
    *PARAM_RADIUS_SCALE.write().unwrap() = values[6];
    *PARAM_RADIUS_VARIATION.write().unwrap() = values[7];
    *PARAM_NONLINK_ATTRACTION.write().unwrap() = values[8];
}

const PARAM_KEYS: [&str; 9] = [
    "spring_k",
    "damping",
    "center_pull",
    "repulsion_radius",
    "repulsion_k",
    "alpha_decay",
    "radius_scale",
    "radius_variation",
    "attraction",
];

const GRAPH_PARAMS_FILE: &str = ".graph-params";

/// Serialize force settings to .graph-params text.
pub fn serialize_params(values: [f32; 9], alpha_cooling: bool, show_panel: bool) -> String {
    let mut out = String::new();
    for (i, key) in PARAM_KEYS.iter().enumerate() {
        out.push_str(&format!("{key}={:.4}\n", values[i]));
    }
    out.push_str(&format!(
        "alpha_cooling={}\nshow_force_panel={}\n",
        if alpha_cooling { 1 } else { 0 },
        if show_panel { 1 } else { 0 },
    ));
    out
}

/// Parse .graph-params text over `defaults`. Missing/unknown keys keep the
/// default slot; the two booleans come back as None when the key is absent.
pub fn parse_params(text: &str, defaults: [f32; 9]) -> ([f32; 9], Option<bool>, Option<bool>) {
    let mut values = defaults;
    let mut cooling = None;
    let mut panel = None;
    for raw in text.lines() {
        let line = raw.trim();
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let v = v.trim();
        if let Some(idx) = PARAM_KEYS.iter().position(|&key| key == k) {
            if let Ok(num) = v.parse::<f32>() {
                values[idx] = num;
            }
        } else if k == "alpha_cooling" {
            match v {
                "1" => cooling = Some(true),
                "0" => cooling = Some(false),
                _ => {}
            }
        } else if k == "show_force_panel" {
            match v {
                "1" => panel = Some(true),
                "0" => panel = Some(false),
                _ => {}
            }
        }
    }
    (values, cooling, panel)
}

/// Apply the graph's .graph-params file to the live statics, if present.
/// Missing file or parse noise leaves the offending slot untouched.
pub fn load_graph_params(dir: &Path) {
    let path = dir.join(GRAPH_PARAMS_FILE);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return;
    };
    let (values, cooling, panel) = parse_params(&text, param_values());
    set_param_values(values);
    if let Some(c) = cooling {
        *ALPHA_COOLING_ENABLED.write().unwrap() = c;
    }
    if let Some(p) = panel {
        *SHOW_FORCE_PANEL.write().unwrap() = p;
    }
}

/// Write the current force settings to the graph directory's .graph-params
/// file. Written to a temp sibling then renamed so a crash can't leave a
/// half-written file.
pub fn save_graph_params(dir: &Path) {
    let values = param_values();
    let cooling = *ALPHA_COOLING_ENABLED.read().unwrap();
    let panel = *SHOW_FORCE_PANEL.read().unwrap();
    let text = serialize_params(values, cooling, panel);
    let path = dir.join(GRAPH_PARAMS_FILE);
    let tmp = dir.join(format!(".{}.tmp", GRAPH_PARAMS_FILE));
    if std::fs::write(&tmp, &text).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

pub static DRAGGING_NODE: RwLock<Option<usize>> = RwLock::new(None);
pub static HOVER_NODE: RwLock<Option<usize>> = RwLock::new(None);
pub static NODES: RwLock<Vec<Node>> = RwLock::new(Vec::<Node>::new());
pub static EDGES: RwLock<Vec<Edge>> = RwLock::new(Vec::<Edge>::new());

// Force simulation "temperature" (d3-force's alpha model): a value between 1
// (fully hot) and 0 (frozen) that scales every applied force. Each tick alpha
// decays toward ALPHA_TARGET, so the graph eases to rest instead of jostling
// forever as it would at fixed-strength forces - the slower the climbing gets,
// the weaker the forces pushing it keep going. Once alpha crosses ALPHA_MIN the
// layout is provably at (near) rest, so update_forces pauses until something
// perturbs it again.
const ALPHA_START: f32 = 1.0;
// Floor enforced while a node is being dragged: the layout keeps following the
// pointer, but stays gentler than a full relayout (d3's default reheat level).
const ALPHA_REHEAT: f32 = 0.3;
const ALPHA_TARGET: f32 = 0.0;
// d3 default comes in at ~300 ticks; 0.005 keeps the layout hot longer so the
// user's dialed-in forces read fully before the graph eases to rest
// (~1380 ticks ≈ 23s at 60fps).
const ALPHA_DECAY: f32 = 0.0050;
const ALPHA_MIN: f32 = 0.001;
static SIM_SETTLED: AtomicBool = AtomicBool::new(false);
// Current simulation temperature. Decayed every frame by update_forces; reset
// to ALPHA_START by wake_simulation whenever the layout is perturbed. Read by
// the debug panel so it can show alpha live.
pub static SIM_ALPHA: RwLock<f32> = RwLock::new(ALPHA_START);

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
    // The whole layout is about to be replaced; the force sim must rebuild.
    wake_simulation();

    let files = filesystem::scan_directory(dir);
    let mut rng = rand::rng();

    let mut nodes = NODES.write().unwrap();
    let mut edges = EDGES.write().unwrap();
    nodes.clear();
    edges.clear();

    // Sunflower (phyllotaxis) initial layout: file i sits on a disc at radius
    // ~ sqrt(index) * scale, spiralled by the golden angle. Uniform density,
    // sized to the node count, so the layout starts near its natural rest
    // spacing. A random wobble (±100px around the centre) packs every node into
    // a fraction of the space they want, so the first force frames violently
    // scatter the graph and the alpha-cooled sim freezes a bloated mess.
    let n = files.len().max(1) as f32;
    let spiral_radius = n.sqrt() * 36.0;
    let golden_angle = std::f32::consts::PI * (3.0 - 5.0_f32.sqrt());
    let center = Vector2::new(config::width() as f32 / 2.0, config::height() as f32 / 2.0);

    for (i, file) in files.iter().enumerate() {
        let file_name = file.file_name().unwrap().to_string_lossy().to_string();
        let name = file_name.trim_end_matches(".md").to_string();
        let t = (i as f32 + 0.5) / n;
        let angle = golden_angle * i as f32;
        let r = spiral_radius * t.sqrt();

        nodes.push(Node {
            radius: NODE_BASE_RADIUS,
            color: Color::WHITE,
            position: Vector2::new(
                center.x + r * angle.cos() + rng.random_range(-4.0..4.0),
                center.y + r * angle.sin() + rng.random_range(-4.0..4.0),
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
    camera.target = Vector2::new(config::width() as f32 / 2.0, config::height() as f32 / 2.0);
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

// Target the edges of a single just-saved note (autosave / Ctrl+S). This is
// the hot path on large graphs: instead of re-reading every .md file like
// rebuild_edges, only the saved file is parsed, its outgoing link set is
// diffed against the current edges, and if nothing changed the graph is left
// untouched. Headers and the subgraph flag (cheap single-file checks) are
// still refreshed every save. Degrees and radii are recomputed in memory only
// when the link set actually changed.
pub fn refresh_saved_node(path: &Path) {
    let mut name_to_idx: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let node_idx = {
        let nodes = NODES.read().unwrap();
        for (i, node) in nodes.iter().enumerate() {
            name_to_idx.insert(node.file_name.clone(), i);
            name_to_idx.insert(node.file_name.trim_end_matches(".md").to_string(), i);
        }
        nodes.iter().position(|n| n.path == path)
    };
    let Some(idx) = node_idx else {
        return;
    };

    let content = filesystem::read_file(path);

    // Parse frontmatter for the header target; slice it off to extract body
    // links exactly like rebuild_edges does.
    let fm = frontmatter::parse(&content);
    let header = fm.as_ref().and_then(|f| f.header.clone());
    let body = if let Some(fm) = &fm {
        if fm.end_byte <= content.len() {
            &content[fm.end_byte..]
        } else {
            &content
        }
    } else {
        &content
    };

    let mut new_targets: Vec<usize> = Vec::new();
    for link in filesystem::parse_links(body) {
        let target = link.strip_suffix(".md").unwrap_or(&link);
        if let Some(&j) = name_to_idx.get(target) {
            if idx != j && !new_targets.contains(&j) {
                new_targets.push(j);
            }
        }
    }
    new_targets.sort();

    let mut nodes = NODES.write().unwrap();
    let mut edges = EDGES.write().unwrap();

    {
        let node = &mut nodes[idx];
        node.has_subgraph = filesystem::is_dir(&filesystem::subgraph_dir(path));
        node.header = header;
    }

    let mut old_targets: Vec<usize> = edges
        .iter()
        .filter(|e| e.n1 == idx)
        .map(|e| e.n2)
        .collect();
    old_targets.sort();
    if old_targets == new_targets {
        return;
    }

    // Link set changed: swap this node's outgoing edges. The layout must
    // recompute because the new springs pull differently.
    wake_simulation();
    edges.retain(|e| e.n1 != idx);
    for &j in &new_targets {
        edges.push(Edge { n1: idx, n2: j });
    }

    // Recompute sizes from the full (in-memory) edge set.
    let mut degree = vec![0u32; nodes.len()];
    for edge in edges.iter() {
        degree[edge.n1] += 1;
        degree[edge.n2] += 1;
    }
    for (node, &count) in nodes.iter_mut().zip(&degree) {
        node.radius = radius_for(count);
    }
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
        node.radius = radius_for(count);
    }
}

pub fn add_node(dir: &Path, filename: &str) -> usize {
    let mut rng = rand::rng();

    // A new node perturbs the layout; let it push its neighbours around.
    wake_simulation();

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
            rng.random_range((config::width() as f32 / 2. - 50.)..(config::width() as f32 / 2. + 50.)),
            rng.random_range((config::height() as f32 / 2. - 50.)..(config::height() as f32 / 2. + 50.)),
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
    // Removing a node/edges changes every spring in the layout.
    wake_simulation();

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
    // Renaming rewires links and rescales nodes; restart the layout sim.
    wake_simulation();

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

// Repulsion between every pair closer than the interaction radius, computed
// with a spatial grid. Cell size equals the interaction radius, so a repelling
// pair can only occupy the same cell or two adjacent ones: scanning the 3x3
// cell neighborhood of each node finds every pair within range (and none
// beyond, where the force would be zero anyway). Returns one accumulated force
// per node. Pure and unit-testable; callers pass the tuned radii/strengths
// (the runtime version reads the live PARAM_* statics, tests pass the fixed
// constants).
fn repulsion_forces(
    positions: &[Vector2],
    repulsion_radius: f32,
    repulsion_k: f32,
    linked: &std::collections::HashSet<(usize, usize)>,
    attraction_k: f32,
) -> Vec<Vector2> {
    let mut forces = vec![Vector2::zero(); positions.len()];

    let mut grid: std::collections::HashMap<(i32, i32), Vec<usize>> =
        std::collections::HashMap::with_capacity(positions.len());
    for (i, pos) in positions.iter().enumerate() {
        let cell = (
            (pos.x / repulsion_radius).floor() as i32,
            (pos.y / repulsion_radius).floor() as i32,
        );
        grid.entry(cell).or_default().push(i);
    }

    for i in 0..positions.len() {
        let pi = positions[i];
        let cx = (pi.x / repulsion_radius).floor() as i32;
        let cy = (pi.y / repulsion_radius).floor() as i32;
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
                    if dist >= repulsion_radius {
                        continue;
                    }
                    // Soft term: inverse-square of the pair distance, keeping
                    // clusters open at any range under the cutoff. Pure springs
                    // may press discs together at short range; that's allowed
                    // now that the old hard "never overlap" core is gone.
                    let soft = if dist > 1e-3 {
                        repulsion_k / (dist * dist)
                    } else {
                        0.0
                    };
                    // The fade keeps the force continuous out to the edge of
                    // the grid cell (no hard pop there).
                    let mag = soft * (1.0 - dist / repulsion_radius);
                    forces[i] += diff.scale(mag / dist.max(1e-6));

                    // Pairs WITHOUT an edge between them feel a soft pull toward
                    // each other, computed in this same spatial pass (no second
                    // O(n^2) scan): it fades to zero at the cell-adjacent
                    // boundary exactly like the repulsion, and stays weaker so
                    // clusters cohere without collapsing.
                    if attraction_k > 0.0 && !linked.contains(&(i.min(j), i.max(j))) {
                        let mag_attr = attraction_k * dist * (1.0 - dist / repulsion_radius);
                        forces[i] -= diff.scale(mag_attr / dist.max(1e-6));
                        forces[j] += diff.scale(mag_attr / dist.max(1e-6));
                    }
                }
            }
        }
    }
    forces
}

// Kick the force simulation out of its settled (paused) state and reheat it to
// full strength. Call after any structural change (add/remove/rename/regenerate)
// or manual nudge so the layout recomputes, then cools back down to sleep.
pub fn wake_simulation() {
    SIM_SETTLED.store(false, std::sync::atomic::Ordering::Relaxed);
    *SIM_ALPHA.write().unwrap() = ALPHA_START;
}

pub fn update_forces(rl: &mut RaylibHandle) {
    // While the graph is settled the layout is at rest: skip the whole force
    // pass (position snapshots, grid build, spring/repulsion math) every
    // frame. Woken by structural changes and drags, and it re-sleeps below.
    if SIM_SETTLED.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }

    let mut nodes = NODES.write().unwrap();
    let edges = EDGES.read().unwrap();

    let dragging_node = DRAGGING_NODE.read().unwrap();
    let delta_time = rl.get_frame_time();

    // Live-tunable forces (debug panel). Falling back to the tuned param
    // statics keeps a settled graph from re-awakening on slider tweaks; the
    // slider handlers wake the sim explicitly instead.
    let repulsion_radius = *PARAM_REPULSION_RADIUS.read().unwrap();
    let repulsion_k = *PARAM_REPULSION_K.read().unwrap();
    let spring_k = *PARAM_SPRING_K.read().unwrap();
    let damping = *PARAM_DAMPING.read().unwrap();
    let gravity_k = *PARAM_GRAVITY_K.read().unwrap();
    let edge_rest_gap = spring_rest_gap(spring_k);
    let alpha_decay = *PARAM_ALPHA_DECAY.read().unwrap();
    let alpha_cooling_enabled = *ALPHA_COOLING_ENABLED.read().unwrap();

    // Cool the simulation: alpha moves toward ALPHA_TARGET and every force
    // below is scaled by it. While a node is dragged the alpha is held at
    // ALPHA_REHEAT so the layout keeps following the pointer; without that
    // floor the sim would freeze mid-gesture once alpha cooled. With the
    // cooldown switched off, alpha is pinned hot so the graph churns forever.
    let mut alpha = *SIM_ALPHA.read().unwrap();
    if alpha_cooling_enabled {
        alpha += (ALPHA_TARGET - alpha) * alpha_decay;
        if dragging_node.is_some() {
            alpha = alpha.max(ALPHA_REHEAT);
        }
    } else {
        alpha = 1.0;
    }
    *SIM_ALPHA.write().unwrap() = alpha;

    let positions: Vec<Vector2> = nodes.iter().map(|n| n.position).collect();
    let mut linked: std::collections::HashSet<(usize, usize)> =
        std::collections::HashSet::with_capacity(edges.len());
    for edge in edges.iter() {
        linked.insert((edge.n1.min(edge.n2), edge.n1.max(edge.n2)));
    }
    let attraction_k = *PARAM_NONLINK_ATTRACTION.read().unwrap();
    let mut forces = repulsion_forces(&positions, repulsion_radius, repulsion_k, &linked, attraction_k);

    let center = Vector2::new(config::width() as f32 / 2.0, config::height() as f32 / 2.0);

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
        let rest = nodes[edge.n1].radius + nodes[edge.n2].radius + edge_rest_gap;
        let force = spring_k * (dist - rest);
        forces[edge.n1] += direction * force;
        forces[edge.n2] -= direction * force;
    }
    for (i, node) in nodes.iter_mut().enumerate() {
        if Some(i) == *dragging_node {
            continue;
        }
        node.velocity = (node.velocity + forces[i] * alpha * delta_time) * damping;
        node.position += node.velocity * delta_time;
    }

    // The simulation has cooled to the freeze point: exactly ALPHA_DECAY-bound,
    // regardless of node count, so large graphs can't jostle forever. No speed
    // threshold to chase - alpha bounds the force, so residual motion at the
    // freeze point is provably negligible.
    if alpha_cooling_enabled && alpha < ALPHA_MIN {
        SIM_SETTLED.store(true, std::sync::atomic::Ordering::Relaxed);
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
        let _radii = vec![7.0; cluster.len()];
        let f = repulsion_forces(
            &cluster,
            REPULSION_RADIUS,
            REPULSION_K,
            &Default::default(),
            0.0,
        );
        for i in 0..cluster.len() {
            assert!(f[i].length() > 0.0, "cluster members must repel each other");
        }

        // A pair further apart than the interaction radius feels nothing at
        // all. Scaled off the constant so it tracks future tuning.
        let far = vec![
            Vector2::zero(),
            Vector2::new(REPULSION_RADIUS * 2.0, REPULSION_RADIUS * 2.0),
        ];
        let _far_radii = vec![7.0; 2];
        let g = repulsion_forces(
            &far,
            REPULSION_RADIUS,
            REPULSION_K,
            &Default::default(),
            0.0,
        );
        assert_eq!(g[0].length(), 0.0);
        assert_eq!(g[1].length(), 0.0);
    }

    #[test]
    fn non_linked_pairs_attract_through_the_grid() {
        let left = Vector2::new(0.0, 0.0);
        let right = Vector2::new(120.0, 0.0);
        let positions = vec![left, right];

        // No edge: the grid pass applies the soft pull on top of the repulsion, and
        // at 120px distance (radius 652) the attraction outweighs the soft
        // term, so the net force points toward the other node.
        let f = repulsion_forces(
            &positions,
            REPULSION_RADIUS,
            REPULSION_K,
            &Default::default(),
            0.05,
        );
        let toward = right - left;
        assert!(f[0].dot(toward) > 0.0, "free pair must pull together");
        assert!(f[1].dot(toward) < 0.0);

        // Linked (edge present): spring owns that pair, the grid only
        // repulses, so the forces point apart again.
        let mut linked = std::collections::HashSet::new();
        linked.insert((0usize, 1usize));
        let g = repulsion_forces(
            &positions,
            REPULSION_RADIUS,
            REPULSION_K,
            &linked,
            0.05,
        );
        assert!(g[0].dot(toward) < 0.0, "linked pair must keep repelling");
        assert!(g[1].dot(toward) > 0.0);
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
        let fast = repulsion_forces(
            &positions,
            REPULSION_RADIUS,
            REPULSION_K,
            &Default::default(),
            0.0,
        );

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
                let soft = if dist > 1e-3 {
                    REPULSION_K / (dist * dist)
                } else {
                    0.0
                };
                let mag = soft * (1.0 - dist / REPULSION_RADIUS);
                brute[i] += diff.scale(mag / dist.max(1e-6));
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
        // spring rest length is radii + spring_rest_gap(k) and the soft
        // repulsion over-pushes, so the settled gap is always clear of the
        // discs.
        let r1 = NODE_BASE_RADIUS;
        let r2 = NODE_MAX_RADIUS;
        let mut positions = vec![Vector2::new(0.0, 0.0), Vector2::new(3.0, 0.0)];
        let mut velocities = vec![Vector2::zero(), Vector2::zero()];
        let spring_k = 0.90_f32;
        let damping = 0.55_f32;
        let dt = 0.01_f32;
        for _ in 0..5000 {
            let mut forces = repulsion_forces(
                &positions,
                REPULSION_RADIUS,
                REPULSION_K,
                &Default::default(),
                0.0,
            );
            let diff = positions[1] - positions[0];
            let dist = diff.length().max(1e-4);
            let direction = diff.scale(1.0 / dist);
            let rest = r1 + r2 + spring_rest_gap(spring_k);
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

        // Disconnected pairs only have repulsion; starting overlapped they shove
        // apart and nothing draws them back together (soft term has zero range
        // beyond the interaction radius).
        let mut p2 = vec![Vector2::new(0.0, 0.0), Vector2::new(1.0, 1.0)];
        let mut v2 = vec![Vector2::zero(), Vector2::zero()];
        for _ in 0..3000 {
            let forces = repulsion_forces(
                &p2,
                REPULSION_RADIUS,
                REPULSION_K,
                &Default::default(),
                0.0,
            );
            for i in 0..2 {
                v2[i] = (v2[i] + forces[i] * dt) * damping;
                p2[i] += v2[i] * dt;
            }
        }
        let sep2 = (p2[1] - p2[0]).length();
        assert!(sep2 >= 2.0 * NODE_BASE_RADIUS, "disconnected pair must spread apart, got {sep2}");
    }

    #[test]
    fn alpha_cooling_guarantees_rest_for_cramped_graphs() {
        // A cramped, random tree would jostle forever under fixed-strength
        // forces. With the alpha model every force is scaled by a temperature
        // that decays each tick, so the layout eases to rest within ALPHA_DECAY
        // ticks no matter how tangled the start positions are.
        let mut rng = rand::rng();
        let n = 40;
        let mut positions: Vec<Vector2> = (0..n)
            .map(|_| {
                Vector2::new(
                    rng.random_range(-150.0..150.0),
                    rng.random_range(-150.0..150.0),
                )
            })
            .collect();
        let mut velocities = vec![Vector2::zero(); n];
        let radii = vec![NODE_BASE_RADIUS; n];
        let edges: Vec<(usize, usize)> = (1..n).map(|i| (i, rng.random_range(0..i))).collect();

        let spring_k = 0.40_f32;
        let damping = 0.95_f32;
        let dt = 1.0 / 60.0_f32;
        let gravity_k = 0.04_f32;
        let center = Vector2::new(config::width() as f32 / 2.0, config::height() as f32 / 2.0);

        let mut alpha = 1.0_f32;
        let mut max_speed = f32::MAX;
        // With the baked-in 0.005 decay alpha needs ~1380 ticks to cross the
        // freeze point; budget 2000 so the tail definitely ends below it.
        for _ in 0..2000 {
            alpha += (ALPHA_TARGET - alpha) * ALPHA_DECAY;
            let mut forces = repulsion_forces(
                &positions,
                REPULSION_RADIUS,
                REPULSION_K,
                &Default::default(),
                0.0,
            );
            for i in 0..n {
                forces[i] += (center - positions[i]) * gravity_k;
            }
            for &(a, b) in &edges {
                let diff = positions[b] - positions[a];
                let dist = diff.length();
                let direction = if dist > 0.001 { diff.scale(1.0 / dist) } else { Vector2::zero() };
                let rest = radii[a] + radii[b] + spring_rest_gap(spring_k);
                let force = spring_k * (dist - rest);
                forces[a] += direction * force;
                forces[b] -= direction * force;
            }
            for i in 0..n {
                velocities[i] = (velocities[i] + forces[i] * alpha * dt) * damping;
            }
            for i in 0..n {
                positions[i] += velocities[i] * dt;
            }
            max_speed = velocities.iter().map(|v| v.length()).fold(0.0_f32, f32::max);
        }
        assert!(alpha < ALPHA_MIN, "simulation must cool below the freeze point");
        assert!(
            max_speed < 0.1,
            "residual motion should be negligible, got {max_speed}"
        );
    }

    #[test]
    fn force_panel_hit_testing_tracks_shared_geometry() {
        // The renderer draws each slider row from slider_row_y() and the input
        // handler tests the same helpers, so a click on any row center must
        // always resolve to that slider at the default zoom.
        let row_h = config::scaled_size(PANEL_ROW_H) as f32;
        for idx in 0..SLIDER_COUNT {
            let row_y = slider_row_y(idx) as f32;
            let probe_x = PANEL_X as f32 + config::scaled_size(PANEL_W) as f32 / 2.0;
            assert_eq!(
                hit_test_slider(probe_x, row_y + row_h / 2.0),
                Some(idx),
                "slider {idx} not grabable at its row center"
            );
        }
        // The alpha-cooldown toggle owns the row above the first slider, which
        // must NOT resolve to any slider.
        let toggle_y = panel_body_y() as f32;
        assert!(
            hit_test_alpha_toggle(PANEL_X as f32 + 50.0, toggle_y + row_h / 2.0),
            "toggle row must be the alpha toggle, not a slider"
        );
        assert!(
            hit_test_slider(PANEL_X as f32 + 50.0, toggle_y + row_h / 2.0).is_none(),
            "toggle row must not hit a slider"
        );

        // The Respawn button sits below the last slider and must also not
        // resolve to any slider.
        let respawn_y = respawn_button_y() as f32;
        assert!(
            hit_test_respawn_button(PANEL_X as f32 + 50.0, respawn_y + row_h / 2.0),
            "respawn row must hit the respawn button"
        );
        assert!(
            hit_test_slider(PANEL_X as f32 + 50.0, respawn_y + row_h / 2.0).is_none(),
            "respawn row must not hit a slider"
        );
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
    fn refresh_saved_node_diffs_outgoing_edges() {
        let _guard = TEST_NAV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join("rg_refresh_edges_test");
        let _ = std::fs::remove_dir_all(&dir);
        filesystem::create_dir(&dir);
        let a = dir.join("a.md");
        let b = dir.join("b.md");
        let c = dir.join("c.md");
        filesystem::write_file(&a, "# A\n\n[[b]]\n");
        filesystem::write_file(&b, "# B\n");
        filesystem::write_file(&c, "# C\n");
        NAV_STACK.write().unwrap().clear();
        generate_nodes_from_directory(&dir);

        fn outgoing(nodes: &[Node], edges: &[Edge], name: &str) -> Vec<String> {
            let mut targets: Vec<String> = edges
                .iter()
                .filter(|e| nodes[e.n1].name == name)
                .map(|e| nodes[e.n2].name.clone())
                .collect();
            targets.sort();
            targets
        }

        {
            let nodes = NODES.read().unwrap();
            let edges = EDGES.read().unwrap();
            assert_eq!(outgoing(&nodes, &edges, "a"), vec!["b".to_string()]);
        }

        // Same content saved again: no link change, edges untouched.
        filesystem::write_file(&a, "# A\n\n[[b]]\n");
        refresh_saved_node(&a);
        {
            let nodes = NODES.read().unwrap();
            let edges = EDGES.read().unwrap();
            assert_eq!(outgoing(&nodes, &edges, "a"), vec!["b".to_string()]);
        }

        // Add a link to c: a's edge set grows.
        filesystem::write_file(&a, "# A\n\n[[b]]\n[[c]]\n");
        refresh_saved_node(&a);
        {
            let nodes = NODES.read().unwrap();
            let edges = EDGES.read().unwrap();
            assert_eq!(outgoing(&nodes, &edges, "a"), vec!["b".to_string(), "c".to_string()]);
        }

        // Remove the link to b: only the c edge remains.
        filesystem::write_file(&a, "# A\n\n[[c]]\n");
        refresh_saved_node(&a);
        {
            let nodes = NODES.read().unwrap();
            let edges = EDGES.read().unwrap();
            assert_eq!(outgoing(&nodes, &edges, "a"), vec!["c".to_string()]);

            // Radii reflect the new degree: b lost a's link so it shrinks back
            // to base, c gained one so it outgrows b.
            let b_rad = nodes.iter().find(|n| n.name == "b").unwrap().radius;
            let c_rad = nodes.iter().find(|n| n.name == "c").unwrap().radius;
            assert_eq!(b_rad, NODE_BASE_RADIUS);
            assert!(c_rad > b_rad);
        }

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

    #[test]
    fn graph_params_round_trip_through_serialization() {
        // Pure functions only: no global statics, so this is parallel-safe.
        let values = [1.5, 0.97, 0.42, 300.0, 12000.0, 0.02, 1.3, 0.4, 0.15];
        let text = serialize_params(values, false, true);
        let defaults = [0.90, 0.95, 0.1, 360.0, 10000.0, 0.0228, 1.0, 0.0, 0.05];
        let (got, cooling, panel) = parse_params(&text, defaults);
        assert_eq!(got, values);
        assert_eq!(cooling, Some(false));
        assert_eq!(panel, Some(true));
    }

    #[test]
    fn graph_params_partial_file_keeps_defaults_for_missing_keys() {
        let defaults = [0.90, 0.95, 0.1, 360.0, 10000.0, 0.0228, 1.0, 0.0, 0.05];
        // Only spring_k and the panel flag present; junk lines and an unnamed
        // key must be ignored, everything else falls back to the defaults.
        let text = "spring_k=2.25\n\nnot-a-param=99\nbogus\nalpha_cooling=0\n      \n";
        let (got, cooling, panel) = parse_params(text, defaults);
        let mut expect = defaults;
        expect[0] = 2.25;
        assert_eq!(got, expect);
        assert_eq!(cooling, Some(false));
        assert_eq!(panel, None);
    }

    #[test]
    fn spring_rest_gap_tracks_tightness() {
        // The merged spring dial: grows the rest gap as the spring weakens,
        // and never lets discs sit closer than the floor gap.
        let loose = spring_rest_gap(0.05);
        let default = spring_rest_gap(0.40);
        let tight = spring_rest_gap(5.0);
        assert!(loose > default, "weak spring must settle far apart");
        assert!(default > tight, "strong spring must settle tight");
        assert!(default > 100.0, "default keeps a real gap, got {default}");
        assert!(tight >= SPRING_GAP_LO, "tight cannot undershoot floor gap");
    }
}
