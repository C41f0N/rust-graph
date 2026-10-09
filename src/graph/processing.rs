use crate::config;
use crate::filesystem;
use crate::frontmatter;
use rand::prelude::*;
use raylib::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::RwLock;

// Node disc radius: Logseq's exact page-node formula
// (extensions/graph/pixi/logic.cljs `node-radius`): base 3.8, growth
// 3.4*sqrt(degree) capped at +12, so hubs read clearly without outgrowing the
// 82-unit link rest length. forceCollide adds another +10 per node, so the two
// smallest discs keep 3.8+10+10+3.8 = 27.6 world units apart.
pub const NODE_BASE_RADIUS: f32 = 3.8;
pub const NODE_MAX_RADIUS: f32 = 15.8; // 3.8 + Logseq's 12.0 growth cap
pub const NODE_RADIUS_GROWTH: f32 = 3.4;
const NODE_RADIUS_GROWTH_CAP: f32 = 12.0;

// Effective disc radius for a node of `degree`: Logseq's formula scaled by the
// live radius-scale dial. "Radius variation" scales the degree growth instead
// of shrinking toward uniform (1.0 = Logseq's growth, 0.0 = every node at the
// base radius). Reads the PARAM_* statics so the panel dials and the persisted
// app config both flow through here.
fn radius_for(degree: u32) -> f32 {
    let scale = *PARAM_RADIUS_SCALE.read().unwrap();
    let growth = NODE_RADIUS_GROWTH * *PARAM_RADIUS_VARIATION.read().unwrap();
    (NODE_BASE_RADIUS + NODE_RADIUS_GROWTH_CAP.min(growth * (degree as f32).sqrt())) * scale
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

// ---- d3-force 3.0.0, Logseq's exact layout algorithm --------------------
// Logseq lays its global graph out with d3-force@3.0.0
// (extensions/graph/pixi/logic.cljs): forceLink(82, 0.82) + forceManyBody
// (strength -140, distanceMax 420) + forceCollide(radius+10, 0.86,
// iterations 2) + forceCenter(0,0), integrated over a fixed per-size tick
// budget (160/110/90/70) with d3's velocityVerlet (velocityDecay 0.6). The
// constants below are that recipe verbatim; the sim is a straight port of
// d3-force@3.0.0 (simulation/link/manyBody/collide/center) and
// d3-quadtree@3.0.1.
pub const D3_LINK_DISTANCE: f32 = 82.0;
pub const D3_LINK_STRENGTH: f32 = 0.82;
pub const D3_CHARGE_STRENGTH: f32 = -140.0;
pub const D3_DISTANCE_MAX: f32 = 420.0;
pub const D3_COLLIDE_PAD: f32 = 10.0;
pub const D3_COLLIDE_STRENGTH: f32 = 0.86;
pub const D3_COLLIDE_ITERATIONS: usize = 2;
pub const D3_VELOCITY_DECAY: f32 = 0.6;
/// d3's default alpha decay: alpha += (0 - alpha) * decay each tick, cooling 1
/// toward the 0.001 floor over ~300 ticks (Math.pow(0.001, 1/300) ≈ 0.0227628).
pub const D3_ALPHA_DECAY: f32 = 0.0227628;
/// Barnes-Hut accuracy (theta 0.9 squared).
pub const D3_THETA2: f32 = 0.81;
pub const D3_DISTANCE_MIN2: f32 = 1.0;
/// d3-force runs an lcg() seeded with undefined, which evaluates to a constant
/// 0, so every jiggle = (0 - 0.5) * 1e-6 = -5e-7.
pub const D3_JIGGLE: f32 = -5.0e-7;
/// d3's seed geometry for position-less nodes: radius 10*sqrt(0.5+i) at angle
/// i * pi(3-sqrt(5)) (the golden angle). pi*(3 - sqrt(5)) pinned as a literal
/// because sqrt is not const.
pub const D3_INITIAL_RADIUS: f32 = 10.0;
pub const D3_INITIAL_ANGLE: f32 = 2.39996323;

// Live-tunable force parameters (d3-force terms; defaults = Logseq's recipe).
pub static PARAM_SPRING_K: RwLock<f32> = RwLock::new(D3_LINK_STRENGTH);   // forceLink strength
pub static PARAM_DAMPING: RwLock<f32> = RwLock::new(D3_VELOCITY_DECAY);  // velocityDecay
pub static PARAM_GRAVITY_K: RwLock<f32> = RwLock::new(1.0);              // forceCenter strength
pub static PARAM_REPULSION_RADIUS: RwLock<f32> = RwLock::new(D3_DISTANCE_MAX); // charge distanceMax
pub static PARAM_REPULSION_K: RwLock<f32> = RwLock::new(-D3_CHARGE_STRENGTH); // |charge| (Logseq -140)
pub static PARAM_ALPHA_DECAY: RwLock<f32> = RwLock::new(D3_ALPHA_DECAY);
pub static PARAM_RADIUS_SCALE: RwLock<f32> = RwLock::new(1.0);
pub static PARAM_RADIUS_VARIATION: RwLock<f32> = RwLock::new(1.0);       // degree-growth multiplier
pub static PARAM_COLLIDE_PAD: RwLock<f32> = RwLock::new(D3_COLLIDE_PAD); // forceCollide +pad per node
/// Presentation, not a d3 force: the camera zoom at which node labels reach
/// full opacity (the renderer fades them in over LABEL_FADE_WINDOW below it).
pub static PARAM_LABEL_FADE: RwLock<f32> = RwLock::new(0.5);

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
pub const SLIDER_COUNT: usize = 10;

// (min, max) range of each slider, in the same order as the PARAM_* list.
// Ranges are Logseq's d3-force recipes where they exist; 9 (Label Fade) is a
// presentation lever - the zoom at which labels turn fully opaque.
pub const SLIDER_RANGES: [(f32, f32); SLIDER_COUNT] = [
    (0.0, 2.0),     // 0 Link Strength  (0.82)
    (0.0, 1.0),     // 1 Velocity Decay (0.60)
    (0.0, 2.0),     // 2 Center Pull    (1.00)
    (50.0, 1200.0), // 3 Charge Radius  (420)
    (0.0, 600.0),   // 4 Rep K          (140)
    (0.002, 0.1),   // 5 Alpha Decay    (~0.0228)
    (0.5, 2.0),     // 6 Radius Scale   (1.0)
    (0.0, 2.0),     // 7 Radius Var.    (1.0)
    (0.0, 40.0),    // 8 Collide Pad    (10)
    (0.1, 3.0),     // 9 Label Fade     (0.5)
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
// Indexes run top-to-bottom: 0 = Link Strength ... 8 = Collide Pad. The
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

// Screen-space y of the bottom button row (below the last slider), which holds
// "Reset Tweaks" (left half) and "Respawn" (right half).
pub fn respawn_button_y() -> i32 {
    PANEL_Y
        + config::scaled_size(PANEL_TITLE_H)
        + config::scaled_size(PANEL_ROW_H) * (1 + SLIDER_COUNT as i32)
}

// Return True if the pointer is over the Reset Tweaks button (left half of the
// bottom row). Mirrors the renderer's split at the panel midpoint.
pub fn hit_test_reset_button(mx: f32, my: f32) -> bool {
    let px = PANEL_X as f32;
    let pw = config::scaled_size(PANEL_W) as f32;
    let row_h = config::scaled_size(PANEL_ROW_H) as f32;
    let y = respawn_button_y() as f32;
    mx >= px + 8.0
        && mx <= px + pw / 2.0 - 4.0
        && my >= y
        && my <= y + row_h
}

// Return True if the pointer is over the Respawn button (right half of the
// bottom row).
pub fn hit_test_respawn_button(mx: f32, my: f32) -> bool {
    let px = PANEL_X as f32;
    let pw = config::scaled_size(PANEL_W) as f32;
    let row_h = config::scaled_size(PANEL_ROW_H) as f32;
    let y = respawn_button_y() as f32;
    mx >= px + pw / 2.0 + 4.0
        && mx <= px + pw - 8.0
        && my >= y
        && my <= y + row_h
}

// Re-initialize the current directory's graph from scratch: fresh phyllotaxis
// positions, rebuilt edges, reset camera/zoom, cleared selection, sim woken.
pub fn respawn_graph() {
    let dir = DIR_PATH.read().unwrap().clone();
    generate_nodes_from_directory(&dir);
}

// Reset every force-panel tweak to the compiled d3 defaults (Logseq's exact
// recipe): the 9 force values, alpha cooldown back ON, and disc radii
// recomputed from the live degrees. `nodes` must be the live NODES write guard
// (see apply_radii). Reheats the sim so the graph visibly settles to defaults.
pub fn reset_graph_tweaks(nodes: &mut [Node]) {
    set_param_values([
        D3_LINK_STRENGTH,    // 0 Link Strength
        D3_VELOCITY_DECAY,   // 1 Velocity Decay
        1.0,                 // 2 Center Pull
        D3_DISTANCE_MAX,     // 3 Charge Radius
        -D3_CHARGE_STRENGTH, // 4 Rep K
        D3_ALPHA_DECAY,      // 5 Alpha Decay
        1.0,                 // 6 Radius Scale
        1.0,                 // 7 Radius Var.
        D3_COLLIDE_PAD,      // 8 Collide Pad
        0.5,                 // 9 Label Fade
    ]);
    *ALPHA_COOLING_ENABLED.write().unwrap() = true;
    let _ = apply_radii(nodes);
    wake_simulation();
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
        4 => v.round(),
        5 => (v * 10000.0).round() / 10000.0,
        6 => (v * 100.0).round() / 100.0,
        7 => (v * 100.0).round() / 100.0,
        8 => (v * 10.0).round() / 10.0,
        9 => (v * 100.0).round() / 100.0,
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
        8 => *PARAM_COLLIDE_PAD.write().unwrap() = value,
        9 => *PARAM_LABEL_FADE.write().unwrap() = value,
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
// Force values live in the appdata config (see crate::app_config) so every
// graph shares one set and the panel survives a restart. These accessors are
// the model interface the config module and the debug panel both use.

/// The 10 force values in slider order (link_strength ... label_fade).
pub fn param_values() -> [f32; 10] {
    [
        *PARAM_SPRING_K.read().unwrap(),
        *PARAM_DAMPING.read().unwrap(),
        *PARAM_GRAVITY_K.read().unwrap(),
        *PARAM_REPULSION_RADIUS.read().unwrap(),
        *PARAM_REPULSION_K.read().unwrap(),
        *PARAM_ALPHA_DECAY.read().unwrap(),
        *PARAM_RADIUS_SCALE.read().unwrap(),
        *PARAM_RADIUS_VARIATION.read().unwrap(),
        *PARAM_COLLIDE_PAD.read().unwrap(),
        *PARAM_LABEL_FADE.read().unwrap(),
    ]
}

/// Overwrite every force value from `values` (slider order).
pub fn set_param_values(values: [f32; 10]) {
    *PARAM_SPRING_K.write().unwrap() = values[0];
    *PARAM_DAMPING.write().unwrap() = values[1];
    *PARAM_GRAVITY_K.write().unwrap() = values[2];
    *PARAM_REPULSION_RADIUS.write().unwrap() = values[3];
    *PARAM_REPULSION_K.write().unwrap() = values[4];
    *PARAM_ALPHA_DECAY.write().unwrap() = values[5];
    *PARAM_RADIUS_SCALE.write().unwrap() = values[6];
    *PARAM_RADIUS_VARIATION.write().unwrap() = values[7];
    *PARAM_COLLIDE_PAD.write().unwrap() = values[8];
    *PARAM_LABEL_FADE.write().unwrap() = values[9];
}

/// Key names (in slider order) used to persist the force values. Exposed so
/// crate::app_config can (de)serialize the same letters in the global config.
pub(crate) const PARAM_KEYS: [&str; 10] = [
    "spring_k",
    "damping",
    "center_pull",
    "repulsion_radius",
    "repulsion_k",
    "alpha_decay",
    "radius_scale",
    "radius_variation",
    "collide_pad",
    "label_fade",
];

pub static DRAGGING_NODE: RwLock<Option<usize>> = RwLock::new(None);
pub static HOVER_NODE: RwLock<Option<usize>> = RwLock::new(None);
pub static NODES: RwLock<Vec<Node>> = RwLock::new(Vec::<Node>::new());
pub static EDGES: RwLock<Vec<Edge>> = RwLock::new(Vec::<Edge>::new());

// Force simulation "temperature" (d3-force's alpha model): a value between 1
// (fully hot) and 0 (frozen) that scales every applied force. Each tick alpha
// decays toward ALPHA_TARGET, so the graph eases to rest instead of jostling
// forever as it would at fixed-strength forces - the slower the climbing gets,
// the weaker the forces pushing it keep going. Logseq freezes the layout after
// a fixed per-size tick budget (see layout_tick_count) rather than on an alpha
// floor, so update_forces pauses at that budget until something wakes it.
const ALPHA_START: f32 = 1.0;
// Floor enforced while a node is being dragged: the layout keeps following the
// pointer, but stays gentler than a full relayout (d3's default reheat level).
const ALPHA_REHEAT: f32 = 0.3;
const ALPHA_TARGET: f32 = 0.0;
static SIM_SETTLED: AtomicBool = AtomicBool::new(false);
// Ticks the current run has executed since the last wake; compared against
// layout_tick_count to freeze the layout exactly where Logseq would.
static SIM_TICK: AtomicUsize = AtomicUsize::new(0);
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

// Node context-menu rows. The renderer draws a menu from this list and the
// input handler hit-tests and dispatches against the same list, so the two can
// never drift apart (a click can't target a row that was never drawn, or vice
// versa).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ContextRow {
    Rename,
    Delete,
    OpenSubGraph,
    FoldSubGraph,
    UnwrapSubGraph,
    SetHeader,
}

impl ContextRow {
    pub fn label(self) -> &'static str {
        match self {
            ContextRow::Rename => "Rename",
            ContextRow::Delete => "Delete",
            ContextRow::OpenSubGraph => "Open Sub-Graph",
            ContextRow::FoldSubGraph => "Fold Sub-Graph",
            ContextRow::UnwrapSubGraph => "Unwrap Sub-Graph",
            ContextRow::SetHeader => "Set Header",
        }
    }

    pub fn hover_color(self) -> Color {
        match self {
            ContextRow::Rename => Color::new(76, 128, 204, 160),
            ContextRow::Delete => Color::new(200, 60, 60, 160),
            ContextRow::OpenSubGraph => Color::new(76, 128, 204, 160),
            ContextRow::FoldSubGraph => Color::new(76, 180, 120, 160),
            ContextRow::UnwrapSubGraph => Color::new(76, 180, 120, 160),
            ContextRow::SetHeader => Color::new(76, 128, 204, 160),
        }
    }

    // 0 rename, 1 delete, 2 sub-graph actions, 3 header. Separators are drawn
    // between groups so the conditional rows read as one block.
    pub fn group(self) -> u8 {
        match self {
            ContextRow::Rename => 0,
            ContextRow::Delete => 1,
            ContextRow::OpenSubGraph
            | ContextRow::FoldSubGraph
            | ContextRow::UnwrapSubGraph => 2,
            ContextRow::SetHeader => 3,
        }
    }
}

// The folder that owns a node's sub-graph contents: the companion folder for a
// plain note (`a.md` -> `a/`), or the note's own parent folder for a
// folder-backed main note (`a/a.md` -> `a/`). Everything about fold/unwrap is
// read back from these folders; there is no in-memory fold state.
fn node_subgraph_folder(node: &Node) -> PathBuf {
    if node.folder_backed {
        node.path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_default()
    } else {
        filesystem::subgraph_dir(&node.path)
    }
}

// Bare names of every note that currently lives inside one of this level's
// sub-graph folders. A stray reference to one of them from another note is
// sub-graph content reached through its main node, so it must NOT materialize a
// ghost at this level. The file on disk is the test: a real loose note of the
// same name is resolved through `name_to_idx` before this ever matters.
fn collect_folded_names(nodes: &[Node]) -> std::collections::HashSet<String> {
    let mut names = std::collections::HashSet::new();
    for node in nodes {
        if let Ok(entries) = std::fs::read_dir(node_subgraph_folder(node)) {
            for entry in entries.flatten() {
                if entry.path().is_file() {
                    if let Some(stem) = entry.path().file_stem() {
                        names.insert(stem.to_string_lossy().to_string());
                    }
                }
            }
        }
    }
    names
}

// Fold guards, derived from the live edge set. A fold may not cross the one-way
// boundary: no packed child may be referenced from outside the fold set, and
// nothing in the fold set (the main note or a packed child) may link out to a
// real note that stays behind. Ghost links (no file on disk) never block.
fn fold_guards_ok(idx: usize, nodes: &[Node]) -> bool {
    let edges = EDGES.read().unwrap();
    // The fold set: this node plus every real-file direct child.
    let mut fold: std::collections::HashSet<usize> = std::collections::HashSet::new();
    fold.insert(idx);
    for e in edges.iter().filter(|e| e.n1 == idx) {
        if nodes.get(e.n2).map_or(false, |c| c.path.is_file()) {
            fold.insert(e.n2);
        }
    }
    for e in edges.iter() {
        let a_real = nodes.get(e.n1).map_or(false, |n| n.path.is_file());
        let b_real = nodes.get(e.n2).map_or(false, |n| n.path.is_file());
        // Outward: something folded links to a real note staying behind.
        if fold.contains(&e.n1) && !fold.contains(&e.n2) && b_real {
            return false;
        }
        // Inbound to a packed child from outside the fold set (references to
        // the main note from outside are allowed).
        if e.n2 != idx && fold.contains(&e.n2) && !fold.contains(&e.n1) && a_real {
            return false;
        }
    }
    true
}

// A node may be folded when it is a plain note at this level (not itself the
// main md of a folder) and the one-way boundary holds. A childless note folds
// into an empty nest.
fn fold_eligible(node: &Node, idx: usize, nodes: &[Node]) -> bool {
    if node.folder_backed || !node.path.is_file() {
        return false;
    }
    fold_guards_ok(idx, nodes)
}

// Ordered rows for the node context menu: Rename + Delete, then the sub-graph
// actions (Fold for a plain note, Open + Unwrap for a folder-backed main note),
// then Set Header. Fold and "create sub-graph" are the same operation now, so
// there is no separate create row.
pub fn context_menu_rows(idx: Option<usize>, nodes: &[Node]) -> Vec<ContextRow> {
    let mut rows = vec![ContextRow::Rename, ContextRow::Delete];
    if let Some(i) = idx {
        if let Some(node) = nodes.get(i) {
            if node.folder_backed {
                rows.push(ContextRow::OpenSubGraph);
                if node.path.is_file() {
                    rows.push(ContextRow::UnwrapSubGraph);
                }
            } else {
                if fold_eligible(node, i, nodes) {
                    rows.push(ContextRow::FoldSubGraph);
                }
                if node.has_subgraph {
                    rows.push(ContextRow::OpenSubGraph);
                }
            }
        }
    }
    rows.push(ContextRow::SetHeader);
    rows
}

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
    // True when this node is the main note of a folder: its `.md` lives inside
    // a same-named folder (`<name>/<name>.md`). Such a node is a self-contained
    // sub-graph at this level: it can be opened and unwrapped, and its own
    // outgoing links render only while the folder is open.
    pub folder_backed: bool,
}

pub struct Edge {
    pub n1: usize,
    pub n2: usize,
}

pub fn generate_nodes_from_directory(dir: &Path) {
    // The whole layout is about to be replaced; the force sim must rebuild.
    wake_simulation();

    let files = filesystem::scan_directory(dir);

    let mut nodes = NODES.write().unwrap();
    let mut edges = EDGES.write().unwrap();
    nodes.clear();
    edges.clear();

    // d3-force's seed (d3-force@3.0.0 `position`): node i starts at radius
    // initialRadius*sqrt(0.5+i) on the golden angle, exactly the state d3 hands
    // Logseq before running its forces. No wobble: like d3, the seed is
    // deterministic and the forces alone shape the layout. Centred on the
    // screen; the forceCenter pass keeps the centroid there.
    let center = Vector2::new(config::width() as f32 / 2.0, config::height() as f32 / 2.0);

    for (i, file) in files.iter().enumerate() {
        let file_name = file.file_name().unwrap().to_string_lossy().to_string();
        let name = file_name.trim_end_matches(".md").to_string();
        let angle = D3_INITIAL_ANGLE * i as f32;
        let radius = D3_INITIAL_RADIUS * (i as f32 + 0.5).sqrt();

        nodes.push(Node {
            radius: NODE_BASE_RADIUS,
            color: Color::WHITE,
            position: Vector2::new(
                center.x + radius * angle.cos(),
                center.y + radius * angle.sin(),
            ),
            velocity: Vector2::new(0.0, 0.0),
            name,
            file_name,
            path: file.clone(),
            header: None,
            has_subgraph: filesystem::is_dir(&filesystem::subgraph_dir(file)),
            folder_backed: false,
        });
    }

    // Folder-backed nodes: an immediate sub-folder `X/` that contains `X/X.md`
    // is a self-contained sub-graph whose main note is the folder's own .md.
    // It appears at this level as a single node named `X`; the folder's other
    // notes stay inside until the folder is opened. A same-named loose note at
    // this level wins (the file scan above already produced it).
    let mut backed: Vec<(String, PathBuf)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if !p.is_dir() {
                continue;
            }
            let Some(folder_name) = p.file_name().map(|s| s.to_string_lossy().to_string()) else {
                continue;
            };
            if folder_name == "assets" || folder_name.starts_with('.') {
                continue;
            }
            let main = p.join(format!("{folder_name}.md"));
            if main.is_file() && !nodes.iter().any(|n| n.name == folder_name) {
                backed.push((folder_name, main));
            }
        }
    }
    backed.sort_by(|a, b| a.1.cmp(&b.1));
    for (name, main) in backed {
        let i = nodes.len();
        let angle = D3_INITIAL_ANGLE * i as f32;
        let radius = D3_INITIAL_RADIUS * (i as f32 + 0.5).sqrt();
        nodes.push(Node {
            radius: NODE_BASE_RADIUS,
            color: Color::WHITE,
            position: Vector2::new(
                center.x + radius * angle.cos(),
                center.y + radius * angle.sin(),
            ),
            velocity: Vector2::new(0.0, 0.0),
            name: name.clone(),
            file_name: format!("{name}.md"),
            path: main,
            header: None,
            has_subgraph: true,
            folder_backed: true,
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

// Resolve a node header target (stored relative to a directory, e.g.
// "assets/name.png") to an absolute path on disk. The note's own directory is
// tried first: a fold packs the note's assets into the sub-graph's own
// `assets/` folder, so the moved note keeps rendering from there. When the
// local copy is absent (the normal case for root-level notes), the project
// root is the fallback, matching how assets are copied in today.
pub fn resolve_header_path(note_path: &Path, header: &str) -> Option<PathBuf> {
    let h = header.trim();
    if h.is_empty() {
        return None;
    }
    if let Some(dir) = note_path.parent() {
        let local = dir.join(h);
        if local.is_file() {
            return Some(local);
        }
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

// ---------------------------------------------------------------------------
// Fold / Unwrap Sub-Graphs
//
// "Fold Sub-Graph" turns a note into a folder-backed sub-graph: the note's own
// `.md` moves inside a same-named folder (`note.md` -> `note/note.md`, the
// folder's main node) and its real-file direct-link children are packed beside
// it, each child's referenced assets moving into `<name>/assets/`. Unwrap is
// the exact reverse (the main note, every packed note, companion folders and
// assets back out; empty folder removed). Everything is read back from disk, so
// there is no in-memory fold state and unwrap works after a restart or for a
// project folder grafted in from outside. Both end in a directory regenerate,
// so the layout stays an exact d3 port on the (smaller) current node set.
// ---------------------------------------------------------------------------

// Every file under `assets_dir` that `content` references: the frontmatter
// header plus every `[[assets/...]]` body link (the canonical form the editor
// inserts). Deduplicated, and only existing files are returned, so a link
// typed before its file was copied in is skipped harmlessly.
fn referenced_assets(content: &str, assets_dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    let mut consider = |target: &str| {
        let t = target.trim();
        if !t.starts_with("assets/") {
            return;
        }
        let rel = Path::new(t).strip_prefix("assets").unwrap_or(Path::new(t));
        let src = assets_dir.join(rel);
        if src.is_file() && seen.insert(src.clone()) {
            out.push(src);
        }
    };
    if let Some(fm) = frontmatter::parse(content) {
        if let Some(h) = fm.header {
            consider(&h);
        }
    }
    for link in filesystem::parse_links(content) {
        consider(&link);
    }
    out
}

// Move one asset (plus its `thumbnails/` companion, when present) from the
// project's `assets/` tree into a sub-graph's own `assets/` tree, preserving
// the relative path. An existing destination file is left alone: asset names
// are uid-based, so a same-named file is the same asset already on the way.
fn move_asset_into(src: &Path, root_assets: &Path, sub_assets: &Path) {
    let rel = src.strip_prefix(root_assets).unwrap_or(src).to_path_buf();
    let dst = sub_assets.join(&rel);
    if !dst.exists() {
        if let Some(parent) = dst.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if std::fs::rename(src, &dst).is_ok() {
            // Thumbnail companion (next to the asset under `thumbnails/`).
            let thumb = crate::editor::images::thumb_path(src);
            if thumb.is_file() {
                let t_rel = thumb.strip_prefix(root_assets).unwrap_or(&thumb).to_path_buf();
                let t_dst = sub_assets.join(t_rel);
                if !t_dst.exists() {
                    if let Some(parent) = t_dst.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    let _ = std::fs::rename(&thumb, &t_dst);
                }
            }
        }
    }
}

// Fold a node into its own sub-graph folder: the note itself moves inside
// (`<name>.md` -> `<name>/<name>.md`) and its direct link children are packed
// beside it, together with their companion folders and referenced assets.
// Returns false when the one-way boundary guards reject the fold or the move
// fails. A childless note folds into an empty nest.
pub fn fold_node(dir: &Path, idx: usize) -> bool {
    let (name, node_path, folder_backed) = {
        let nodes = NODES.read().unwrap();
        let Some(node) = nodes.get(idx) else {
            return false;
        };
        (node.name.clone(), node.path.clone(), node.folder_backed)
    };
    if folder_backed {
        return false;
    }
    // The main note's destination must be free; if `dir/name/name.md` already
    // exists the node is already folder-backed (or a name clash), so bail.
    let subdir = dir.join(&name);
    if subdir.join(format!("{name}.md")).is_file() {
        return false;
    }
    // One-way boundary guards: no packed child may be referenced from outside
    // the fold set, and nothing folded may link out to a note staying behind.
    {
        let nodes = NODES.read().unwrap();
        if !fold_guards_ok(idx, &nodes) {
            return false;
        }
    }

    // Children: outgoing edges whose target is a real note file in the current
    // directory. Ghost targets have no file, so they cannot move; they follow
    // their referrer implicitly because a ghost is re-created where the note
    // that links to it lives, i.e. inside the sub-graph once it is scanned.
    let children: Vec<(PathBuf, String, String)> = {
        let nodes = NODES.read().unwrap();
        let edges = EDGES.read().unwrap();
        let mut out: Vec<(PathBuf, String, String)> = Vec::new();
        for edge in edges.iter().filter(|e| e.n1 == idx) {
            let Some(child) = nodes.get(edge.n2) else {
                continue;
            };
            if child.path.is_file()
                && child.path.parent() == Some(dir)
                && !out.iter().any(|(p, _, _)| *p == child.path)
            {
                let content = filesystem::read_file(&child.path);
                out.push((child.path.clone(), child.file_name.clone(), content));
            }
        }
        out
    };

    let parent_content = filesystem::read_file(&node_path);

    filesystem::create_dir(&subdir);
    let root_assets = dir.join("assets");
    let sub_assets = subdir.join("assets");
    let mut moved_assets: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();

    // The main note moves inside its own folder, becoming `name/name.md`.
    if filesystem::move_into(&node_path, &subdir).is_none() {
        eprintln!(
            "[fold] could not move {} into its sub-graph",
            node_path.display()
        );
        return false;
    }
    for src in referenced_assets(&parent_content, &root_assets) {
        if moved_assets.insert(src.clone()) {
            move_asset_into(&src, &root_assets, &sub_assets);
        }
    }

    // Pack each direct link child (note file + companion folder + assets).
    for (child_path, file_name, content) in &children {
        if subdir.join(file_name).exists() {
            eprintln!("[fold] {file_name} already in the sub-graph; left at the parent level");
            continue;
        }
        // The note file itself.
        if filesystem::move_into(child_path, &subdir).is_none() {
            eprintln!(
                "[fold] could not move {} into the sub-graph; skipping it",
                child_path.display()
            );
            continue;
        }
        // Its companion folder (own assets/ or nested sub-graph) moves along.
        let stem = file_name.trim_end_matches(".md");
        if stem != "assets" {
            let companion = dir.join(stem);
            if companion.is_dir() {
                let dest = subdir.join(stem);
                if !dest.exists() {
                    let _ = std::fs::rename(&companion, &dest);
                }
            }
        }
        // Referenced assets travel into the sub-graph's own assets/.
        for src in referenced_assets(content, &root_assets) {
            if !moved_assets.insert(src.clone()) {
                continue;
            }
            move_asset_into(&src, &root_assets, &sub_assets);
        }
    }

    generate_nodes_from_directory(dir);
    true
}

// Unwrap a folder-backed sub-graph: every note and companion folder moves back
// into the parent directory, `<name>/assets/` merges back into the project
// `assets/`, and the now-empty sub-graph folder is removed. Returns false when
// the node is not a folder-backed main note. Eligibility is read entirely from
// disk, so unwrap works after a restart and for a project folder grafted in
// from outside.
pub fn unwrap_node(dir: &Path, idx: usize) -> bool {
    let (name, folder_backed) = {
        let nodes = NODES.read().unwrap();
        let Some(node) = nodes.get(idx) else {
            return false;
        };
        (node.name.clone(), node.folder_backed)
    };
    if !folder_backed {
        return false;
    }
    let subdir = dir.join(&name);
    if !subdir.join(format!("{name}.md")).is_file() {
        return false;
    }

    // 1) Every file (notes and anything else) back to the parent. A note whose
    //    name is taken by a file the user created meanwhile is warned about and
    //    restored as "stem (2).ext" instead of overwriting it.
    if let Ok(entries) = std::fs::read_dir(&subdir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if !p.is_file() {
                continue;
            }
            let Some(name) = p.file_name().map(|s| s.to_string_lossy().to_string()) else {
                continue;
            };
            let dest = dir.join(&name);
            if dest.exists() {
                let new_name = filesystem::unique_collision_name(dir, &name);
                eprintln!(
                    "[unwrap] {dest:?} already exists; restoring as {new_name:?}"
                );
                let _ = std::fs::rename(&p, dir.join(&new_name));
            } else {
                let _ = std::fs::rename(&p, dest);
            }
        }
    }

    // 2) Companion folders (a packed child's own sub-graph) return whole.
    if let Ok(entries) = std::fs::read_dir(&subdir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if !p.is_dir() {
                continue;
            }
            let Some(name) = p.file_name().map(|s| s.to_string_lossy().to_string()) else {
                continue;
            };
            if name == "assets" {
                continue; // merged back in step 3
            }
            let dest = dir.join(&name);
            if dest.exists() {
                let new_name = filesystem::unique_collision_name(dir, &name);
                eprintln!("[unwrap] {dest:?} already exists; restoring as {new_name:?}");
                let _ = std::fs::rename(&p, dir.join(&new_name));
            } else {
                let _ = std::fs::rename(&p, dest);
            }
        }
    }

    // 3) The sub-graph's assets merge back into the project assets/. Asset
    //    names are uid-based, so a same-named file at the destination is the
    //    same asset and is skipped rather than duplicated.
    let sub_assets = subdir.join("assets");
    if sub_assets.is_dir() {
        let (_, skipped) = filesystem::move_tree_merge(&sub_assets, &dir.join("assets"));
        if skipped > 0 {
            eprintln!(
                "[unwrap] {skipped} asset(s) already at the destination; skipped (uid names mean the same file)"
            );
        }
    }

    // 4) Drop the sub-graph folder, but only once it is fully empty (a failed
    //    move leaves the user's files in place instead of deleting them).
    if filesystem::remove_empty_tree(&subdir) {
        // removed
    } else {
        eprintln!("[unwrap] {} not empty; left in place", subdir.display());
    }

    generate_nodes_from_directory(dir);
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
    let (node_idx, folder_backed, folded_names) = {
        let nodes = NODES.read().unwrap();
        for (i, node) in nodes.iter().enumerate() {
            name_to_idx.insert(node.file_name.clone(), i);
            name_to_idx.insert(node.file_name.trim_end_matches(".md").to_string(), i);
        }
        let idx = nodes.iter().position(|n| n.path == path);
        let backed = idx.map_or(false, |i| nodes[i].folder_backed);
        (idx, backed, collect_folded_names(&nodes))
    };
    let Some(idx) = node_idx else {
        return;
    };

    let content = filesystem::read_file(path);

    // Parse frontmatter for the header target; slice it off to extract body
    // links exactly like rebuild_edges does.
    let fm = frontmatter::parse(&content);
    let header = fm.as_ref().and_then(|f| f.header.clone());

    let mut new_targets: Vec<usize> = Vec::new();
    let mut ghost_links: Vec<String> = Vec::new();
    // The main note of a closed sub-graph renders no outgoing links at this
    // level (they appear only when the folder is opened), so its body is not
    // scanned here; incoming references to it still resolve by name.
    if !folder_backed {
        let body = if let Some(fm) = &fm {
            if fm.end_byte <= content.len() {
                &content[fm.end_byte..]
            } else {
                &content
            }
        } else {
            &content
        };
        for link in filesystem::parse_links(body) {
            let target = link.strip_suffix(".md").unwrap_or(&link);
            if let Some(&j) = name_to_idx.get(target) {
                if idx != j && !new_targets.contains(&j) {
                    new_targets.push(j);
                }
            } else if is_ghostable_target(target) && !folded_names.contains(target) {
                ghost_links.push(target.to_string());
            }
        }
    }

    let mut nodes = NODES.write().unwrap();
    let mut edges = EDGES.write().unwrap();

    // A freshly typed [[link]] to a note that has no file yet becomes a ghost
    // node right here, mirroring rebuild_edges, so the graph updates live on
    // autosave/Ctrl+S instead of waiting for the next full scan.
    if !ghost_links.is_empty() {
        let center = Vector2::new(config::width() as f32 / 2.0, config::height() as f32 / 2.0);
        for target in &ghost_links {
            let j = match name_to_idx.get(target) {
                Some(&j) => j,
                None => {
                    let gi = nodes.len();
                    let file_name = format!("{target}.md");
                    nodes.push(Node {
                        radius: NODE_BASE_RADIUS,
                        color: Color::WHITE,
                        position: ghost_position(target, center),
                        velocity: Vector2::zero(),
                        name: target.clone(),
                        file_name: file_name.clone(),
                        path: path.parent().map_or_else(
                            || PathBuf::from(&file_name),
                            |d| d.join(&file_name),
                        ),
                        header: None,
                        has_subgraph: false,
                        folder_backed: false,
                    });
                    name_to_idx.insert(file_name, gi);
                    name_to_idx.insert(target.clone(), gi);
                    gi
                }
            };
            if idx != j && !new_targets.contains(&j) {
                new_targets.push(j);
            }
        }
    }
    new_targets.sort();

    {
        let node = &mut nodes[idx];
        node.has_subgraph =
            node.folder_backed || filesystem::is_dir(&filesystem::subgraph_dir(path));
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
// A [[target]] that names no file on disk becomes a "ghost" note: a node
// that is real in the app (visible, openable, editable) even though its
// .md has not been created yet. Saving it writes the file, after which it
// is an ordinary note. Duplicate and self-links are ignored.
pub fn rebuild_edges() {
    let mut nodes = NODES.write().unwrap();
    let mut edges = EDGES.write().unwrap();
    edges.clear();

    let center = Vector2::new(config::width() as f32 / 2.0, config::height() as f32 / 2.0);

    // Map from both the exact filename and its bare stem (extension
    // stripped) to node index, so extension-less [[x]] links resolve.
    let mut name_to_idx: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (i, node) in nodes.iter().enumerate() {
        name_to_idx.insert(node.file_name.clone(), i);
        let stem = node.file_name.trim_end_matches(".md");
        name_to_idx.insert(stem.to_string(), i);
    }

    // Names of notes living inside this level's sub-graph folders. A stray
    // reference to one of them from another note is sub-graph content, not a
    // ghost at this level (the file on disk is the test).
    let folded_names = collect_folded_names(&nodes);

    // Keep a set of existing edges to avoid duplicates
    let mut seen: std::collections::HashSet<(usize, usize)> = std::collections::HashSet::new();

    // Re-parse every node: update header from frontmatter, build edges only
    // from the body (everything after the frontmatter block).
    let mut fm_headers: Vec<Option<String>> = Vec::with_capacity(nodes.len());
    let mut ghost_links: Vec<(usize, PathBuf, String)> = Vec::new();
    for (i, node) in nodes.iter_mut().enumerate() {
        let content = filesystem::read_file(&node.path);

        // Re-check whether a sub-graph folder exists (rename/delete can change
        // it) so the graph always reflects the filesystem. A folder-backed main
        // note always owns its folder, so its flag stays set.
        node.has_subgraph =
            node.folder_backed || filesystem::is_dir(&filesystem::subgraph_dir(&node.path));

        // Parse frontmatter and cache the header target on the node.
        let fm = frontmatter::parse(&content);
        node.header = fm.as_ref().and_then(|f| f.header.clone());
        fm_headers.push(node.header.clone());

        // The main note of a closed sub-graph renders no outgoing links at this
        // level: they appear only when the folder is opened. Its header still
        // shows, and incoming references to it still resolve by name.
        if node.folder_backed {
            continue;
        }

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
            } else if is_ghostable_target(target) {
                // Referenced but missing: defer so the ghost note is created
                // after the parse loop (the loop holds a mutable borrow of
                // `nodes`). The ghost lives next to the note that links to it.
                // A target that lives inside one of this level's sub-graph
                // folders is never ghosted here: it belongs inside the folder.
                if folded_names.contains(target) {
                    continue;
                }
                let referrer_dir = node
                    .path
                    .parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_default();
                ghost_links.push((i, referrer_dir, target.to_string()));
            }
        }
    }

    // Materialize any referenced-but-missing notes as ghosts. They persist in
    // NODES until the next full directory scan; once their file is saved they
    // are ordinary notes and the next scan normalizes them.
    let mut created_ghost = false;
    for (i, referrer_dir, target) in &ghost_links {
        let j = match name_to_idx.get(target) {
            Some(&j) => j,
            None => {
                let gi = nodes.len();
                let file_name = format!("{target}.md");
                nodes.push(Node {
                    radius: NODE_BASE_RADIUS,
                    color: Color::WHITE,
                    position: ghost_position(target, center),
                    velocity: Vector2::zero(),
                    name: target.clone(),
                    file_name: file_name.clone(),
                    path: referrer_dir.join(&file_name),
                    header: None,
                    has_subgraph: false,
                    folder_backed: false,
                });
                name_to_idx.insert(file_name, gi);
                name_to_idx.insert(target.clone(), gi);
                created_ghost = true;
                gi
            }
        };
        if *i != j && seen.insert((*i, j)) {
            edges.push(Edge { n1: *i, n2: j });
        }
    }
    // A fresh ghost needs a few force frames to leave its neighbours' space.
    if created_ghost {
        wake_simulation();
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

// Non-note targets that a [[link]] may name but that can never be opened as a
// note: image and media files (the editor renders those inline or as
// thumbnails). Everything else with an extension still resolves as a note.
fn is_asset_name(target: &str) -> bool {
    let ext = target
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    matches!(
        ext.as_str(),
        "png" | "jpg"
            | "jpeg"
            | "gif"
            | "webp"
            | "svg"
            | "bmp"
            | "ico"
            | "mp3"
            | "mp4"
            | "webm"
            | "mov"
            | "avi"
            | "ogg"
            | "wav"
            | "pdf"
    )
}

// A [[link]] target that can be materialized as a real note: same-folder bare
// names only (no path separators, no leading/trailing whitespace), with either
// no extension or a trailing .md, and never an image/media asset. Links to
// folders (`sub/name`) and files like `data.txt` stay unresolved instead.
fn is_ghostable_target(target: &str) -> bool {
    if target.is_empty() || target.contains('/') || target.contains('\\') {
        return false;
    }
    match target.rsplit_once('.') {
        Some((_, ext)) => ext.to_ascii_lowercase() == "md",
        None => !is_asset_name(target),
    }
}

// Deterministic starting position for a ghost note: stable across rebuilds
// (same target -> same spot, no jitter on every save) and spread around the
// centre so a scatter of ghosts doesn't pile on one pixel. The force sim does
// the rest once woken.
fn ghost_position(target: &str, center: Vector2) -> Vector2 {
    let mut h: u64 = 14695981039346656037; // FNV-1a offset basis
    for b in target.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(1099511628211);
    }
    let angle = ((h % 6283) as f32) / 1000.0; // 0..2π
    let radius = (h % 160) as f32; // 0..160 px from centre
    Vector2::new(center.x + radius * angle.cos(), center.y + radius * angle.sin())
}

// Resolve a [[target]] written in a note body to the file it points at, for
// the editor's click-to-open. The resolution order mirrors both worlds: the
// current graph folder's nodes first (exactly like `rebuild_edges`), then
// path-like targets (e.g. [[sub/name]]) relative to the project root, then a
// bare-stem match anywhere under the root (the Ctrl+K palette's scope). Image
// and other non-note targets never resolve. Returns (path, note name).
pub fn resolve_wikilink(target: &str) -> Option<(PathBuf, String)> {
    let t = target.trim().trim_end_matches(".md");
    if t.is_empty() || is_asset_name(t) {
        return None;
    }

    // Folder-first: the note the graph already knows in the current directory,
    // matching `rebuild_edges`'s stem/filename map.
    {
        let nodes = NODES.read().unwrap();
        for node in nodes.iter() {
            let stem = node.file_name.trim_end_matches(".md");
            if node.file_name == t || stem == t {
                return Some((node.path.clone(), node.name.clone()));
            }
        }
    }

    let root = project_root();

    // Path-like targets: [[sub/deep/name]] under the project root.
    if t.contains('/') || t.contains('\\') {
        let cand = root.join(t);
        let cand = if cand.extension().is_none() {
            cand.with_extension("md")
        } else {
            cand
        };
        if cand.is_file() {
            let name = cand
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            return Some((cand, name));
        }
        return None;
    }

    // Bare-stem fallback anywhere in the project tree (deterministic: the
    // tree scan is path-sorted, so a duplicate stem wins on lexicographic
    // order).
    for p in crate::filesystem::scan_tree(&root) {
        let name = p
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        if name == t {
            return Some((p, name));
        }
    }

    None
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

    // New notes start empty: the file name is the node's identity, not a
    // "# Title" first line the user then has to delete.
    filesystem::create_file(&file_path, "");

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
        folder_backed: false,
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

// Rename a note's .md file (and the folder that owns its sub-graph, if any),
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
    let folder_backed = nodes[idx].folder_backed;

    if folder_backed {
        // The note's own folder carries the name: rename the folder and the
        // `.md` inside it so the pair stays `new/new.md`.
        let parent = match old_path.parent() {
            Some(p) => p.to_path_buf(),
            None => return false,
        };
        if parent.with_file_name(&new_stem).exists() {
            return false;
        }
        if !filesystem::rename_dir(&parent, &new_stem) {
            return false;
        }
        let inner = parent
            .with_file_name(&new_stem)
            .join(old_path.file_name().unwrap_or_default());
        if !filesystem::rename_file(&inner, &new_stem) {
            // Roll the folder rename back so the note survives intact.
            let _ = filesystem::rename_dir(&parent.with_file_name(&new_stem), &old_stem);
            return false;
        }
    } else {
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

        // Rename the companion sub-graph folder. If this somehow fails, roll
        // the file rename back so the pair stays consistent.
        if has_sub && !filesystem::rename_dir(&sub_folder, &new_stem) {
            filesystem::rename_file(
                &old_path.with_file_name(format!("{}.md", new_stem)),
                &old_stem,
            );
            return false;
        }
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

    let new_path = if folder_backed {
        old_path
            .parent()
            .map(|p| p.with_file_name(&new_stem).join(format!("{new_stem}.md")))
            .unwrap_or_else(|| PathBuf::from(format!("{new_stem}.md")))
    } else {
        old_path.with_file_name(format!("{new_stem}.md"))
    };
    nodes[idx].file_name = format!("{new_stem}.md");
    nodes[idx].name = new_stem;
    nodes[idx].path = new_path;
    nodes[idx].folder_backed = folder_backed;
    drop(nodes);
    rebuild_edges();
    true
}

// ---- d3-force 3.0.0 port (Logseq's exact layout) ------------------------

// Per-link constants d3 precomputes once per simulation (forceLink
// initialize): distance, strength, and the source-target bias.
struct LinkMeta {
    bias: Vec<f32>,
    distance: Vec<f32>,
    strength: Vec<f32>,
}

fn build_link_meta(node_count: usize, edges: &[Edge], distance: f32, strength: f32) -> LinkMeta {
    let mut count = vec![0u32; node_count];
    for e in edges.iter() {
        if e.n1 != e.n2 {
            count[e.n1] += 1;
            count[e.n2] += 1;
        }
    }
    let mut meta = LinkMeta {
        bias: Vec::with_capacity(edges.len()),
        distance: Vec::with_capacity(edges.len()),
        strength: Vec::with_capacity(edges.len()),
    };
    for e in edges.iter() {
        let sum = (count[e.n1] + count[e.n2]).max(1);
        meta.bias.push(count[e.n1] as f32 / sum as f32);
        meta.distance.push(distance);
        meta.strength.push(strength);
    }
    meta
}

// d3 forceLink's apply: position-Verlet spring on predicted positions.
fn apply_link(nodes: &mut [Node], edges: &[Edge], meta: &LinkMeta, alpha: f32) {
    for (k, e) in edges.iter().enumerate() {
        if e.n1 == e.n2 {
            continue;
        }
        let (s, t) = (e.n1, e.n2);
        let mut x = nodes[t].position.x + nodes[t].velocity.x
            - nodes[s].position.x
            - nodes[s].velocity.x;
        let mut y = nodes[t].position.y + nodes[t].velocity.y
            - nodes[s].position.y
            - nodes[s].velocity.y;
        if x == 0.0 {
            x = D3_JIGGLE;
        }
        if y == 0.0 {
            y = D3_JIGGLE;
        }
        let mut l = (x * x + y * y).sqrt();
        l = (l - meta.distance[k]) / l * alpha * meta.strength[k];
        x *= l;
        y *= l;
        let b = meta.bias[k];
        nodes[t].velocity.x -= x * b;
        nodes[t].velocity.y -= y * b;
        nodes[s].velocity.x += x * (1.0 - b);
        nodes[s].velocity.y += y * (1.0 - b);
    }
}

// One cell of a d3-quadtree. Cells store their own bounds (d3 reconstructs
// them during traversal; storing them is equivalent). Leaves hold a chain of
// coincident node indices (head first). `value/cx/cy` are the manyBody
// accumulators, `r` the collide quadrant bound.
struct Quad {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    child: [Option<Box<Quad>>; 4],
    leaf: Vec<usize>,
    px: f32,
    py: f32,
    value: f32,
    cx: f32,
    cy: f32,
    r: f32,
}

impl Quad {
    fn internal(x0: f32, y0: f32, x1: f32, y1: f32) -> Quad {
        Quad {
            x0,
            y0,
            x1,
            y1,
            child: [None, None, None, None],
            leaf: Vec::new(),
            px: 0.0,
            py: 0.0,
            value: 0.0,
            cx: 0.0,
            cy: 0.0,
            r: 0.0,
        }
    }

    fn make_leaf(x0: f32, y0: f32, x1: f32, y1: f32, idx: usize, x: f32, y: f32) -> Quad {
        let mut q = Quad::internal(x0, y0, x1, y1);
        q.leaf = vec![idx];
        q.px = x;
        q.py = y;
        q
    }
}

// X-extent of the child cell of (x0,y0,x1,y1) at quadrant `q`
// (bit 0 = right of xm, bit 1 = below ym).
fn quadrant_bounds(x0: f32, y0: f32, x1: f32, y1: f32, q: usize) -> (f32, f32, f32, f32) {
    let xm = (x0 + x1) * 0.5;
    let ym = (y0 + y1) * 0.5;
    let (nx0, nx1) = if q & 1 == 1 { (xm, x1) } else { (x0, xm) };
    let (ny0, ny1) = if q & 2 == 2 { (ym, y1) } else { (y0, ym) };
    (nx0, ny0, nx1, ny1)
}

// d3-quadtree, built the same way d3's addAll does: extent -> cover(min) ->
// cover(max) -> add each point. Used by the charge (positions) and collide
// (positions + velocities) passes.
struct Quadtree {
    root: Option<Box<Quad>>,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
}

impl Quadtree {
    // d3 cover(): double the extent away from (x, y) until it is covered. A
    // leaf root is never wrapped (d3 discards the wrapper); internal roots get
    // re-rooted under the expanded extent.
    fn cover(&mut self, x: f32, y: f32) {
        if x.is_nan() || y.is_nan() {
            return;
        }
        let (mut x0, mut y0, mut x1, mut y1) = (self.x0, self.y0, self.x1, self.y1);
        if x0.is_nan() {
            x0 = x.floor();
            y0 = y.floor();
            x1 = x0 + 1.0;
            y1 = y0 + 1.0;
        } else {
            let root_was_internal = self.root.as_ref().is_some_and(|q| q.leaf.is_empty());
            let mut z = x1 - x0;
            if z == 0.0 {
                z = 1.0;
            }
            let mut node = if root_was_internal {
                self.root.take()
            } else {
                None
            };
            while x0 > x || x >= x1 || y0 > y || y >= y1 {
                let i = (((y < y0) as usize) << 1) | (x < x0) as usize;
                let mut parent = Quad::internal(x0, y0, x1, y1);
                parent.child[i] = node;
                node = Some(Box::new(parent));
                z *= 2.0;
                match i {
                    0 => {
                        x1 = x0 + z;
                        y1 = y0 + z;
                    }
                    1 => {
                        x0 = x1 - z;
                        y1 = y0 + z;
                    }
                    2 => {
                        x1 = x0 + z;
                        y0 = y1 - z;
                    }
                    _ => {
                        x0 = x1 - z;
                        y0 = y1 - z;
                    }
                }
            }
            if root_was_internal {
                // The top wrapper spans the final expanded extent.
                if let Some(top) = node.as_mut() {
                    top.x0 = x0;
                    top.y0 = y0;
                    top.x1 = x1;
                    top.y1 = y1;
                }
                self.root = node;
            }
        }
        self.x0 = x0;
        self.y0 = y0;
        self.x1 = x1;
        self.y1 = y1;
    }

    // d3 add(): coincident chains live in the quadtree itself.
    fn add(&mut self, x: f32, y: f32, idx: usize) {
        if x.is_nan() || y.is_nan() {
            return;
        }
        match self.root.as_mut() {
            Some(root) => insert_into(root, x, y, idx),
            None => {
                self.root = Some(Box::new(Quad::make_leaf(self.x0, self.y0, self.x1, self.y1, idx, x, y)));
            }
        }
    }
}

fn insert_into(node: &mut Quad, x: f32, y: f32, idx: usize) {
    if node.leaf.is_empty() {
        // Internal: descend into the quadrant containing (x, y).
        let xm = (node.x0 + node.x1) * 0.5;
        let ym = (node.y0 + node.y1) * 0.5;
        let q = (((y >= ym) as usize) << 1) | (x >= xm) as usize;
        match node.child[q].as_mut() {
            Some(child) => insert_into(child, x, y, idx),
            None => {
                let (x0, y0, x1, y1) = quadrant_bounds(node.x0, node.y0, node.x1, node.y1, q);
                node.child[q] = Some(Box::new(Quad::make_leaf(x0, y0, x1, y1, idx, x, y)));
            }
        }
    } else {
        let (xp, yp) = (node.px, node.py);
        if x == xp && y == yp {
            // Exactly coincident: chain it at the head.
            node.leaf.insert(0, idx);
            return;
        }
        split_leaf(node, x, y, idx, xp, yp);
    }
}

// d3 add()'s leaf-split loop: subdivide until the old point (xp,yp) and the
// new point (x,y) land in different quadrants, then place both.
fn split_leaf(node: &mut Quad, x: f32, y: f32, idx: usize, xp: f32, yp: f32) {
    let chain = std::mem::take(&mut node.leaf); // all coincident old members
    place_chain(node, chain, xp, yp, x, y, idx);
}

fn place_chain(
    node: &mut Quad,
    chain: Vec<usize>,
    xp: f32,
    yp: f32,
    x: f32,
    y: f32,
    idx: usize,
) {
    let xm = (node.x0 + node.x1) * 0.5;
    let ym = (node.y0 + node.y1) * 0.5;
    let qn = (((y >= ym) as usize) << 1) | (x >= xm) as usize;
    let qo = (((yp >= ym) as usize) << 1) | (xp >= xm) as usize;
    if qn == qo {
        let (x0, y0, x1, y1) = quadrant_bounds(node.x0, node.y0, node.x1, node.y1, qn);
        node.child[qn] = Some(Box::new(Quad::internal(x0, y0, x1, y1)));
        place_chain(node.child[qn].as_mut().unwrap(), chain, xp, yp, x, y, idx);
    } else {
        let (ox0, oy0, ox1, oy1) = quadrant_bounds(node.x0, node.y0, node.x1, node.y1, qo);
        let mut old = Quad::internal(ox0, oy0, ox1, oy1);
        old.leaf = chain;
        old.px = xp;
        old.py = yp;
        let (nx0, ny0, nx1, ny1) = quadrant_bounds(node.x0, node.y0, node.x1, node.y1, qn);
        node.child[qo] = Some(Box::new(old));
        node.child[qn] = Some(Box::new(Quad::make_leaf(nx0, ny0, nx1, ny1, idx, x, y)));
    }
}

// d3-quadtree visit(): pre-order; return true from the callback to prune the
// subtree. Children are pushed in 3,2,1,0 order so they pop 0,1,2,3 like d3.
fn visit_quad(node: &Quad, f: &mut dyn FnMut(&Quad) -> bool) {
    let mut stack: Vec<&Quad> = vec![node];
    while let Some(q) = stack.pop() {
        if !f(q) && q.leaf.is_empty() {
            for k in (0..4).rev() {
                if let Some(c) = &q.child[k] {
                    stack.push(c);
                }
            }
        }
    }
}

// d3-quadtree visitAfter(): children before parents.
fn visit_after_quad(node: &mut Quad, f: &mut dyn FnMut(&mut Quad)) {
    if node.leaf.is_empty() {
        for k in 0..4 {
            if let Some(c) = &mut node.child[k] {
                visit_after_quad(c, f);
            }
        }
    }
    f(node);
}

fn build_quadtree(xs: &[f32], ys: &[f32]) -> Quadtree {
    let n = xs.len();
    let mut t = Quadtree {
        root: None,
        x0: f32::NAN,
        y0: f32::NAN,
        x1: f32::NAN,
        y1: f32::NAN,
    };
    if n == 0 {
        return t;
    }
    let mut x0 = xs[0];
    let mut y0 = ys[0];
    let mut x1 = xs[0];
    let mut y1 = ys[0];
    for i in 1..n {
        x0 = x0.min(xs[i]);
        y0 = y0.min(ys[i]);
        x1 = x1.max(xs[i]);
        y1 = y1.max(ys[i]);
    }
    t.cover(x0, y0);
    t.cover(x1, y1);
    for i in 0..n {
        t.add(xs[i], ys[i], i);
    }
    t
}

// d3 forceManyBody: Barnes-Hut charge. `charge_strength` is the signed per-node
// charge (-140 for every node in Logseq's view).
#[allow(clippy::needless_borrow)]
fn apply_charge(nodes: &mut [Node], alpha: f32, distance_max: f32, charge_strength: f32) {
    let n = nodes.len();
    if n == 0 {
        return;
    }
    let xs: Vec<f32> = nodes.iter().map(|nd| nd.position.x).collect();
    let ys: Vec<f32> = nodes.iter().map(|nd| nd.position.y).collect();
    let tree = build_quadtree(&xs, &ys);
    let strengths = vec![charge_strength; n];
    let distance_max2 = distance_max * distance_max;

    let mut root = tree.root;
    let Some(root) = root.as_mut() else { return };

    // Accumulate per-cell value / centroid (visitAfter).
    visit_after_quad(root, &mut |q: &mut Quad| {
        if q.leaf.is_empty() {
            let mut strength = 0.0f32;
            let mut weight = 0.0f32;
            let mut sx = 0.0f32;
            let mut sy = 0.0f32;
            for k in 0..4 {
                if let Some(c) = &q.child[k] {
                    let m = c.value.abs();
                    if m > 0.0 {
                        strength += c.value;
                        weight += m;
                        sx += m * c.cx;
                        sy += m * c.cy;
                    }
                }
            }
            if weight > 0.0 {
                q.cx = sx / weight;
                q.cy = sy / weight;
            }
            q.value = strength;
        } else {
            q.cx = q.px;
            q.cy = q.py;
            let mut strength = 0.0f32;
            for &j in &q.leaf {
                strength += strengths[j];
            }
            q.value = strength;
        }
    });

    for i in 0..n {
        let node_x = nodes[i].position.x;
        let node_y = nodes[i].position.y;
        visit_quad(root, &mut |q: &Quad| -> bool {
            if q.value == 0.0 {
                return true;
            }
            let mut x = q.cx - node_x;
            let mut y = q.cy - node_y;
            let w = q.x1 - q.x0;
            let mut l = x * x + y * y;
            if w * w / D3_THETA2 < l {
                // Barnes-Hut: whole subtree through its centroid.
                if l < distance_max2 {
                    if x == 0.0 {
                        x = D3_JIGGLE;
                        l += x * x;
                    }
                    if y == 0.0 {
                        y = D3_JIGGLE;
                        l += y * y;
                    }
                    if l < D3_DISTANCE_MIN2 {
                        l = (D3_DISTANCE_MIN2 * l).sqrt();
                    }
                    nodes[i].velocity.x += x * q.value * alpha / l;
                    nodes[i].velocity.y += y * q.value * alpha / l;
                }
                return true;
            }
            if !q.leaf.is_empty() || l >= distance_max2 {
                return false;
            }
            // Leaf within reach: apply to the whole coincident chain.
            if q.leaf.first() != Some(&i) || q.leaf.len() > 1 {
                if x == 0.0 {
                    x = D3_JIGGLE;
                    l += x * x;
                }
                if y == 0.0 {
                    y = D3_JIGGLE;
                    l += y * y;
                }
                if l < D3_DISTANCE_MIN2 {
                    l = (D3_DISTANCE_MIN2 * l).sqrt();
                }
            }
            for &j in &q.leaf {
                if j != i {
                    let w = strengths[j] * alpha / l;
                    nodes[i].velocity.x += x * w;
                    nodes[i].velocity.y += y * w;
                }
            }
            false
        });
    }
}

// d3 forceCollide: build a tree over predicted positions (x+vx), then resolve
// overlapping pairs with the weight split rj^2/(ri^2+rj^2). Runs `iterations`
// times per tick (Logseq uses 2).
fn apply_collide(
    nodes: &mut [Node],
    radii: &[f32],
    pad: f32,
    strength: f32,
    iterations: usize,
) {
    let n = nodes.len();
    if n == 0 {
        return;
    }
    for _ in 0..iterations {
        let xs: Vec<f32> = nodes.iter().map(|nd| nd.position.x + nd.velocity.x).collect();
        let ys: Vec<f32> = nodes.iter().map(|nd| nd.position.y + nd.velocity.y).collect();
        let mut tree = build_quadtree(&xs, &ys);
        let Some(root) = tree.root.as_mut() else { return };

        // prepare (visitAfter): quadrant bound r = max radius within the cell.
        visit_after_quad(root, &mut |q: &mut Quad| {
            if q.leaf.is_empty() {
                let mut r = 0.0f32;
                for k in 0..4 {
                    if let Some(c) = &q.child[k] {
                        r = r.max(c.r);
                    }
                }
                q.r = r;
            } else {
                q.r = radii[q.leaf[0]] + pad;
            }
        });

        for i in 0..n {
            let ri = radii[i] + pad;
            let ri2 = ri * ri;
            let xi = nodes[i].position.x + nodes[i].velocity.x;
            let yi = nodes[i].position.y + nodes[i].velocity.y;
            visit_quad(root, &mut |q: &Quad| -> bool {
                if !q.leaf.is_empty() {
                    let head = q.leaf[0];
                    if head > i {
                        let rj = q.r;
                        let r = ri + rj;
                        let mut x = xi - q.px;
                        let mut y = yi - q.py;
                        let mut l = x * x + y * y;
                        if l < r * r {
                            if x == 0.0 {
                                x = D3_JIGGLE;
                                l += x * x;
                            }
                            if y == 0.0 {
                                y = D3_JIGGLE;
                                l += y * y;
                            }
                            let sl = l.sqrt();
                            l = (r - sl) / sl * strength;
                            x *= l;
                            y *= l;
                            let w = (rj * rj) / (ri2 + rj * rj);
                            nodes[i].velocity.x += x * w;
                            nodes[i].velocity.y += y * w;
                            nodes[head].velocity.x -= x * (1.0 - w);
                            nodes[head].velocity.y -= y * (1.0 - w);
                        }
                    }
                    true
                } else {
                    let rq = q.r;
                    q.x0 > xi + ri + rq
                        || q.x1 < xi - ri - rq
                        || q.y0 > yi + ri + rq
                        || q.y1 < yi - ri - rq
                }
            });
        }
    }
}

// d3 forceCenter: translate every node so the centroid sits exactly on
// `center` (a pure translation; strength scales the correction per tick).
fn apply_center(nodes: &mut [Node], center: Vector2, strength: f32) {
    let n = nodes.len();
    if n == 0 {
        return;
    }
    let mut sx = 0.0f32;
    let mut sy = 0.0f32;
    for nd in nodes.iter() {
        sx += nd.position.x;
        sy += nd.position.y;
    }
    let dx = (sx / n as f32 - center.x) * strength;
    let dy = (sy / n as f32 - center.y) * strength;
    for nd in nodes.iter_mut() {
        nd.position.x -= dx;
        nd.position.y -= dy;
    }
}

// Kick the force simulation out of its settled (paused) state and reheat it to
// full strength. Call after any structural change (add/remove/rename/regenerate),
// a slider tweak, or a drag so the layout recomputes, then it runs Logseq's
// tick budget and cools back down to sleep.
pub fn wake_simulation() {
    SIM_SETTLED.store(false, std::sync::atomic::Ordering::Relaxed);
    *SIM_ALPHA.write().unwrap() = ALPHA_START;
    SIM_TICK.store(0, std::sync::atomic::Ordering::Relaxed);
}

// Start a node drag: d3-drag's start handler does alphaTarget(0.3).restart(),
// so the layout un-freezes and follows the pointer at the gentle drag heat
// (ALPHA_REHEAT) rather than a full relayout (wake_simulation -> alpha 1.0).
// A fresh tick budget lets the neighbours settle back properly once the drag
// ends. Called from the input handler only after real pointer movement, so a
// plain click never wakes a settled graph.
pub fn reheat_drag() {
    SIM_SETTLED.store(false, std::sync::atomic::Ordering::Relaxed);
    let alpha = *SIM_ALPHA.read().unwrap();
    *SIM_ALPHA.write().unwrap() = alpha.max(ALPHA_REHEAT);
    SIM_TICK.store(0, std::sync::atomic::Ordering::Relaxed);
}

// Logseq's fixed per-size simulation budget (non-tags/global view): the layout
// runs exactly this many d3 ticks and then stops.
pub fn layout_tick_count(node_count: usize) -> usize {
    if node_count <= 120 {
        160
    } else if node_count <= 400 {
        110
    } else if node_count <= 900 {
        90
    } else {
        70
    }
}

pub fn update_forces(_rl: &mut RaylibHandle) {
    // While the graph is settled the layout is at rest: skip the whole force
    // pass. Woken by structural changes and drags; re-sleeps at the Logseq tick
    // budget below.
    if SIM_SETTLED.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }

    let mut nodes = NODES.write().unwrap();
    let node_count = nodes.len();
    if node_count == 0 {
        return;
    }
    let edges = EDGES.read().unwrap();
    let dragging = *DRAGGING_NODE.read().unwrap();
    let is_dragging = dragging.is_some();

    let distance_max = *PARAM_REPULSION_RADIUS.read().unwrap();
    let charge = -*PARAM_REPULSION_K.read().unwrap(); // signed charge (Logseq -140)
    let link_distance = D3_LINK_DISTANCE;
    let link_strength = *PARAM_SPRING_K.read().unwrap();
    let velocity_decay = *PARAM_DAMPING.read().unwrap();
    let center_strength = *PARAM_GRAVITY_K.read().unwrap();
    let collide_pad = *PARAM_COLLIDE_PAD.read().unwrap();
    let alpha_decay = *PARAM_ALPHA_DECAY.read().unwrap();
    let alpha_cooling_enabled = *ALPHA_COOLING_ENABLED.read().unwrap();

    // d3 alpha model: each tick alpha moves toward ALPHA_TARGET by alphaDecay.
    // While a node is dragged the alpha is held at ALPHA_REHEAT so the layout
    // keeps following the pointer. Cooldown off pins alpha hot forever.
    let mut alpha = *SIM_ALPHA.read().unwrap();
    if alpha_cooling_enabled {
        alpha += (ALPHA_TARGET - alpha) * alpha_decay;
        if is_dragging {
            alpha = alpha.max(ALPHA_REHEAT);
        }
    } else {
        alpha = 1.0;
    }
    *SIM_ALPHA.write().unwrap() = alpha;

    // One d3-force tick, in d3's force order (link, charge, collide, center),
    // then d3's velocity-Verlet integration.
    let meta = build_link_meta(node_count, &edges, link_distance, link_strength);
    apply_link(&mut nodes, &edges, &meta, alpha);
    apply_charge(&mut nodes, alpha, distance_max, charge);
    let radii: Vec<f32> = nodes.iter().map(|n| n.radius).collect();
    apply_collide(
        &mut nodes,
        &radii,
        collide_pad,
        D3_COLLIDE_STRENGTH,
        D3_COLLIDE_ITERATIONS,
    );
    let center = Vector2::new(config::width() as f32 / 2.0, config::height() as f32 / 2.0);
    apply_center(&mut nodes, center, center_strength);
    for (i, node) in nodes.iter_mut().enumerate() {
        if Some(i) == dragging {
            node.velocity = Vector2::zero();
            continue;
        }
        node.velocity = node.velocity * velocity_decay;
        node.position += node.velocity;
    }
    drop(nodes);
    drop(edges);

    // Freeze when Logseq's budget is spent (a drag keeps the sim hot and does
    // not consume budget). With the cooldown switched off the graph churns
    // forever at full alpha.
    if !is_dragging {
        SIM_TICK.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    if alpha_cooling_enabled
        && SIM_TICK.load(std::sync::atomic::Ordering::Relaxed) >= layout_tick_count(node_count)
    {
        SIM_SETTLED.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

// Tests across modules (graph, editor tabs, ...) drive the same process-global
// NODES/EDGES/DIR_PATH statics, so cargo's parallel test threads would stomp on
// each other. Serialize every test that touches that state under this one
// lock; it is shared (pub, cfg(test)) so the editor module's tests use it too.
#[cfg(test)]
pub static TEST_NAV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    // Real d3 pipeline replica: the exact force order, alpha model, collide
    // iterations, velocity integration, and Logseq tick budget that
    // update_forces runs - with Logseq's constants baked in (this is the
    // production default, not sliders). Reuses the production apply_*
    // functions directly. Returns (positions, velocities).
    fn d3_settle(n: usize, edges: &[(usize, usize)], radii: &[f32]) -> (Vec<Vector2>, Vec<Vector2>) {
        let edge_structs: Vec<Edge> = edges.iter().map(|&(a, b)| Edge { n1: a, n2: b }).collect();
        let mut sim: Vec<Node> = (0..n)
            .map(|i| {
                let angle = D3_INITIAL_ANGLE * i as f32;
                let r = D3_INITIAL_RADIUS * (i as f32 + 0.5).sqrt();
                Node {
                    radius: radii[i],
                    color: Color::WHITE,
                    position: Vector2::new(r * angle.cos(), r * angle.sin()),
                    velocity: Vector2::zero(),
                    name: format!("n{i}"),
                    file_name: format!("n{i}.md"),
                    path: PathBuf::from(format!("n{i}.md")),
                    header: None,
                    has_subgraph: false,
                    folder_backed: false,
                }
            })
            .collect();
        let mut alpha = 1.0_f32;
        let budget = layout_tick_count(n);
        for _ in 0..budget {
            alpha += (ALPHA_TARGET - alpha) * D3_ALPHA_DECAY;
            let meta = build_link_meta(n, &edge_structs, D3_LINK_DISTANCE, D3_LINK_STRENGTH);
            apply_link(&mut sim, &edge_structs, &meta, alpha);
            apply_charge(&mut sim, alpha, D3_DISTANCE_MAX, D3_CHARGE_STRENGTH);
            let rr: Vec<f32> = sim.iter().map(|nd| nd.radius).collect();
            apply_collide(
                &mut sim,
                &rr,
                D3_COLLIDE_PAD,
                D3_COLLIDE_STRENGTH,
                D3_COLLIDE_ITERATIONS,
            );
            let center = Vector2::new(config::width() as f32 / 2.0, config::height() as f32 / 2.0);
            apply_center(&mut sim, center, 1.0);
            for nd in sim.iter_mut() {
                nd.velocity = nd.velocity * D3_VELOCITY_DECAY;
                nd.position += nd.velocity;
            }
        }
        (
            sim.iter().map(|nd| nd.position).collect(),
            sim.iter().map(|nd| nd.velocity).collect(),
        )
    }

    // Minimal node for single-pass force tests (charge/collide impulses).
    fn test_node(x: f32, y: f32) -> Node {
        Node {
            radius: NODE_BASE_RADIUS,
            color: Color::WHITE,
            position: Vector2::new(x, y),
            velocity: Vector2::zero(),
            name: "n".into(),
            file_name: "n.md".into(),
            path: PathBuf::from("n.md"),
            header: None,
            has_subgraph: false,
            folder_backed: false,
        }
    }

    fn layout_stats(positions: &[Vector2], radii: &[f32]) -> (usize, f32, f32) {
        let mut overlaps = 0usize;
        let mut min_pair = f32::MAX;
        let (mut lo_x, mut hi_x, mut lo_y, mut hi_y) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
        for p in positions {
            lo_x = lo_x.min(p.x); hi_x = hi_x.max(p.x);
            lo_y = lo_y.min(p.y); hi_y = hi_y.max(p.y);
        }
        for a in 0..positions.len() {
            for b in (a + 1)..positions.len() {
                let d = (positions[a] - positions[b]).length();
                min_pair = min_pair.min(d);
                if d < radii[a] + radii[b] {
                    overlaps += 1;
                }
            }
        }
        (overlaps, min_pair, (hi_x - lo_x).max(hi_y - lo_y))
    }

    fn edge_stats(positions: &[Vector2], edges: &[(usize, usize)]) -> (f32, f32, usize) {
        let mut sum = 0.0f32;
        let mut max = 0.0f32;
        let mut count = 0usize;
        for &(a, b) in edges {
            if a == b { continue; }
            let d = (positions[a] - positions[b]).length();
            sum += d;
            max = max.max(d);
            count += 1;
        }
        (sum / count as f32, max, count)
    }

#[test]
    fn link_rest_length_matches_logseq_82() {
        // A lone connected pair must settle near the 82-unit rest length that
        // Logseq's forceLink uses (charge widens it slightly; collide only
        // enforces a far-lower floor).
        let n = 2usize;
        let radii = vec![NODE_BASE_RADIUS; n];
        let edges = vec![(0usize, 1usize)];
        let (positions, _) = d3_settle(n, &edges, &radii);
        let sep = (positions[1] - positions[0]).length();
        assert!(
            (70.0..=115.0).contains(&sep),
            "two linked discs should rest near Logseq's 82, got {sep:.1}"
        );
    }

    #[test]
    fn charge_repels_within_radius_and_is_silent_beyond() {
        // Two nodes 100 apart feel the -140 charge as a repulsion; the
        // closed-form impulse is x * strength * alpha / l = 100 * -140 / 1e4.
        let mut nodes_ = vec![test_node(0.0, 0.0), test_node(100.0, 0.0)];
        apply_charge(&mut nodes_, 1.0, D3_DISTANCE_MAX, D3_CHARGE_STRENGTH);
        assert!(
            (nodes_[0].velocity.x + 1.4).abs() < 1e-3,
            "left node pushed left, got {}",
            nodes_[0].velocity.x
        );
        assert!(
            (nodes_[1].velocity.x - 1.4).abs() < 1e-3,
            "right node pushed right, got {}",
            nodes_[1].velocity.x
        );

        // A pair 1000 apart (beyond distanceMax 420) feels nothing at all.
        nodes_[1].position = Vector2::new(1000.0, 0.0);
        nodes_[0].velocity = Vector2::zero();
        nodes_[1].velocity = Vector2::zero();
        apply_charge(&mut nodes_, 1.0, D3_DISTANCE_MAX, D3_CHARGE_STRENGTH);
        assert_eq!(nodes_[0].velocity.length(), 0.0, "no force beyond the cutoff");
        assert_eq!(nodes_[1].velocity.length(), 0.0);
    }

    #[test]
    fn collide_separates_overlapping_discs_per_d3_weights() {
        // Two equal discs 5 apart: collide radius (3.8+10)*2 = 27.6, strength
        // 0.86. Each disc takes the weight rj^2/(ri^2+rj^2) = 0.5 of the scaled
        // overlap (ri == rj), so |impulse| = (27.6-5)*0.86*0.5.
        let mut nodes_ = vec![test_node(0.0, 0.0), test_node(5.0, 0.0)];
        let radii = vec![3.8f32, 3.8];
        apply_collide(&mut nodes_, &radii, 10.0, 0.86, 1);
        let expected = (27.6 - 5.0) * 0.86 * 0.5;
        assert!(
            (nodes_[0].velocity.x + expected).abs() < 1e-3,
            "query node takes half the impulse, got {} want {}",
            nodes_[0].velocity.x,
            -expected
        );
        assert!(
            (nodes_[1].velocity.x - expected).abs() < 1e-3,
            "head node takes the other half, got {}",
            nodes_[1].velocity.x
        );
        assert!(
            nodes_[0].velocity.y.abs() < 1e-3 && nodes_[1].velocity.y.abs() < 1e-3,
            "jiggle keeps the pair perfectly axial, got y {} / {}",
            nodes_[0].velocity.y,
            nodes_[1].velocity.y
        );

        // A clear pair (40 apart > 27.6) feels nothing.
        nodes_[1].position = Vector2::new(40.0, 0.0);
        nodes_[0].velocity = Vector2::zero();
        nodes_[1].velocity = Vector2::zero();
        apply_collide(&mut nodes_, &radii, 10.0, 0.86, 1);
        assert_eq!(nodes_[0].velocity.length(), 0.0);
        assert_eq!(nodes_[1].velocity.length(), 0.0);
    }

    #[test]
    fn dense_graph_force_layout_freezes_without_overlapping_discs() {
        // Regression guard for the "big graphs pile up in the centre" bug, now
        // against the real d3 pipeline. Collide guarantees every pair sits at
        // least ri+pad+rj+pad apart after the budget, so no disc can touch.
        let n = 64usize;
        let mut edges: Vec<(usize, usize)> = Vec::new();
        for i in 0..n {
            for &step in &[1usize, 3] {
                edges.push((i, (i + step) % n));
            }
            edges.push((i, (i * 7 + 13) % n));
            edges.push((i, (i * 5 + 11) % n));
        }
        let mut degree = vec![0u32; n];
        for &(a, b) in &edges {
            if a != b {
                degree[a] += 1;
                degree[b] += 1;
            }
        }
        let radii: Vec<f32> = degree.iter().map(|&d| radius_for(d)).collect();
        let (positions, _) = d3_settle(n, &edges, &radii);
        let (overlaps, min_pair, span) = layout_stats(&positions, &radii);
        assert_eq!(
            overlaps, 0,
            "a settled dense graph must have zero overlapping discs (min_pair {min_pair:.1})"
        );
        assert!(min_pair > 20.0, "no two discs may touch, got {min_pair:.1}");
        assert!(span > 250.0, "a 64-node graph should spread out, span {span:.0}");
    }

    #[test]
    fn sparse_tree_layout_converges_to_short_edges() {
        // Trees are the long-link regime (the current bug report): with the
        // real d3 link impulse each edge must actually reach near its 82-unit
        // rest, not span hundreds of units.
        let n = 63usize;
        let edges: Vec<(usize, usize)> = (1..n).map(|i| ((i - 1) / 3, i)).collect();
        let mut degree = vec![0u32; n];
        for &(a, b) in &edges {
            degree[a] += 1;
            degree[b] += 1;
        }
        let radii: Vec<f32> = degree.iter().map(|&d| radius_for(d)).collect();
        let (positions, _) = d3_settle(n, &edges, &radii);
        let (overlaps, _, span) = layout_stats(&positions, &radii);
        let (mean_edge, max_edge, _) = edge_stats(&positions, &edges);
        assert_eq!(overlaps, 0, "a settled tree must not overlap either");
        assert!(
            mean_edge < 250.0,
            "tree edges must contract toward 82, got mean {mean_edge:.0}"
        );
        assert!(
            max_edge < 350.0,
            "no single tree edge may span the graph, got max {max_edge:.0}"
        );
        assert!(span < 1000.0, "a converged tree stays compact, span {span:.0}");
    }

    #[test]
    fn settled_pairs_stay_clear_regardless_of_size() {
        // A connected leaf+hub pair (d3 radii): the link + collide must leave a
        // clear gap beyond r1 + r2 + 2*pad = 39.6. A disconnected pair lets
        // only charge spread them, still collide-separated.
        let (pos, _) = d3_settle(2, &[(0usize, 1usize)], &[NODE_BASE_RADIUS, NODE_MAX_RADIUS]);
        let sep = (pos[1] - pos[0]).length();
        assert!(sep >= 40.0, "connected leaf+hub must sit clearly apart, got {sep:.1}");

        let (pos2, _) = d3_settle(2, &[], &[NODE_BASE_RADIUS, NODE_BASE_RADIUS]);
        let sep2 = (pos2[1] - pos2[0]).length();
        assert!(sep2 >= 27.0, "disconnected pair must stay collide-separated, got {sep2:.1}");
    }

    #[test]
    fn d3_simulation_rests_within_tick_budget() {
        // A cramped random tree jostles under full-strength forces; Logseq's
        // tick budget + velocity decay must leave it (nearly) at rest.
        let mut rng = rand::rng();
        let n = 40usize;
        let edges: Vec<(usize, usize)> = (1..n).map(|i| (i, rng.random_range(0..i))).collect();
        let radii = vec![NODE_BASE_RADIUS; n];
        let (_, velocities) = d3_settle(n, &edges, &radii);
        let max_speed = velocities.iter().map(|v| v.length()).fold(0.0_f32, f32::max);
        assert!(
            max_speed < 3.0,
            "residual motion after Logseq's budget should be small, got {max_speed:.2}"
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

        // The bottom row below the last slider holds Reset Tweaks (left half)
        // and Respawn (right half); neither must resolve to any slider.
        let row_y = respawn_button_y() as f32;
        let px = PANEL_X as f32;
        let pw = config::scaled_size(PANEL_W) as f32;
        let reset_probe = px + pw / 2.0 - 8.0;
        let respawn_probe = px + pw / 2.0 + 8.0;
        for (probe, reset, respawn) in [
            (reset_probe, true, false),
            (respawn_probe, false, true),
        ] {
            assert_eq!(
                hit_test_reset_button(probe, row_y + row_h / 2.0),
                reset,
                "bottom row split wrong for x={probe}"
            );
            assert_eq!(
                hit_test_respawn_button(probe, row_y + row_h / 2.0),
                respawn,
                "bottom row split wrong for x={probe}"
            );
        }
        assert!(
            hit_test_slider(px + 50.0, row_y + row_h / 2.0).is_none(),
            "bottom row must not hit a slider"
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

        // A freshly typed link to a missing note materializes on save: the
        // ghost appears and a's edge set gains it, without any file being
        // written yet.
        filesystem::write_file(&a, "# A\n\n[[c]]\n[[draft]]\n");
        refresh_saved_node(&a);
        {
            let nodes = NODES.read().unwrap();
            let edges = EDGES.read().unwrap();
            assert_eq!(
                outgoing(&nodes, &edges, "a"),
                vec!["c".to_string(), "draft".to_string()]
            );
            let draft = nodes.iter().find(|n| n.name == "draft").expect("ghost");
            assert_eq!(draft.path, dir.join("draft.md"));
            assert!(!draft.path.exists());
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wikilink_ghosts_materialize_missing_notes_in_app_only() {
        let _guard = TEST_NAV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join("rg_ghost_test");
        let _ = std::fs::remove_dir_all(&dir);
        filesystem::create_dir(&dir);
        // `existing` has a real file; `ghost` is referenced but missing;
        // `pic.png` is an asset link and must NOT become a node.
        filesystem::write_file(&dir.join("real.md"), "# Real\n\n[[existing]]\n[[ghost]]\n[[pic.png]]\n");
        filesystem::write_file(&dir.join("existing.md"), "# Existing\n");

        *DIR_PATH.write().unwrap() = dir.clone();
        NAV_STACK.write().unwrap().clear();
        generate_nodes_from_directory(&dir);

        let ghost = {
            let nodes = NODES.read().unwrap();
            assert!(
                !nodes.iter().any(|n| n.name == "pic.png"),
                "image links must not turn into nodes"
            );
            nodes
                .iter()
                .find(|n| n.name == "ghost")
                .expect("missing [[ghost]] must produce a ghost node")
                .path
                .clone()
        };
        assert_eq!(ghost, dir.join("ghost.md"));
        assert!(!ghost.exists(), "a ghost note has no file on disk until saved");

        // The referring note has an edge to the ghost.
        {
            let nodes = NODES.read().unwrap();
            let edges = EDGES.read().unwrap();
            let real = nodes.iter().position(|n| n.name == "real").unwrap();
            let g = nodes.iter().position(|n| n.name == "ghost").unwrap();
            assert!(edges.iter().any(|e| e.n1 == real && e.n2 == g));
        }

        // Opening the ghost resolves to its (not-yet-existing) path, and
        // saving it materializes the .md so it becomes an ordinary note.
        let (p, name) = resolve_wikilink("ghost").unwrap();
        assert_eq!(p, ghost);
        assert_eq!(name, "ghost");
        filesystem::write_file(&ghost, "# Ghost\n\n[[real]]\n");
        assert!(ghost.exists());

        // A full re-scan still knows the note (now as a real file).
        generate_nodes_from_directory(&dir);
        {
            let nodes = NODES.read().unwrap();
            let g = nodes.iter().find(|n| n.name == "ghost").expect("ghost note");
            assert_eq!(g.path, ghost);
            // Both directions are linked now.
            let edges = EDGES.read().unwrap();
            let real = nodes.iter().position(|n| n.name == "real").unwrap();
            let gi = nodes.iter().position(|n| n.name == "ghost").unwrap();
            assert!(edges.iter().any(|e| e.n1 == real && e.n2 == gi));
            assert!(edges.iter().any(|e| e.n1 == gi && e.n2 == real));
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_wikilink_folder_first_then_tree() {
        let _guard = TEST_NAV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join("rg_resolve_link_test");
        let _ = std::fs::remove_dir_all(&dir);
        filesystem::create_dir(&dir);
        filesystem::create_dir(&dir.join("sub"));
        filesystem::write_file(&dir.join("alpha.md"), "# Alpha\n");
        filesystem::write_file(&dir.join("sub").join("beta.md"), "# Beta\n");
        filesystem::write_file(&dir.join("sub").join("also.md"), "# Also\n");

        *DIR_PATH.write().unwrap() = dir.clone();
        NAV_STACK.write().unwrap().clear();
        generate_nodes_from_directory(&dir);

        // Folder-first bare stem (matches the graph's own node).
        let (p, name) = resolve_wikilink("alpha").unwrap();
        assert_eq!(p, dir.join("alpha.md"));
        assert_eq!(name, "alpha");

        // .md spelling resolves identically.
        let (p, _) = resolve_wikilink("alpha.md").unwrap();
        assert_eq!(p, dir.join("alpha.md"));

        // Path-like target under the project root, even though the node from
        // the current folder is absent.
        let (p, name) = resolve_wikilink("sub/beta").unwrap();
        assert_eq!(p, dir.join("sub").join("beta.md"));
        assert_eq!(name, "beta");

        // Bare stem fallback into the tree.
        let (p, name) = resolve_wikilink("also").unwrap();
        assert_eq!(p, dir.join("sub").join("also.md"));
        assert_eq!(name, "also");

        // Assets and missing targets never resolve.
        assert!(resolve_wikilink("beta.png").is_none());
        assert!(resolve_wikilink("nosuch").is_none());
        assert!(resolve_wikilink("sub/missing").is_none());
        assert!(resolve_wikilink("").is_none());
        assert!(resolve_wikilink("   ").is_none());

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
    fn fold_packs_direct_link_children_only() {
        let _guard = TEST_NAV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join("rg_fold_test");
        let _ = std::fs::remove_dir_all(&dir);
        filesystem::create_dir(&dir);
        // a -> b, a -> c, b -> c: every edge stays inside the fold set, so the
        // one-way boundary holds. d is not linked at all and stays behind.
        filesystem::write_file(&dir.join("a.md"), "# A\n\n[[b]]\n[[c]]\n");
        filesystem::write_file(&dir.join("b.md"), "# B\n\n[[c]]\n");
        filesystem::write_file(&dir.join("c.md"), "# C\n");
        filesystem::write_file(&dir.join("d.md"), "# D\n");

        *DIR_PATH.write().unwrap() = dir.clone();
        NAV_STACK.write().unwrap().clear();
        generate_nodes_from_directory(&dir);

        let a_idx = NODES
            .read()
            .unwrap()
            .iter()
            .position(|n| n.name == "a")
            .unwrap();
        assert!(fold_node(&dir, a_idx));

        // The main note moved inside its own folder, direct children packed.
        assert!(dir.join("a").join("a.md").exists());
        assert!(dir.join("a").join("b.md").exists());
        assert!(dir.join("a").join("c.md").exists());
        assert!(!dir.join("a.md").exists());
        assert!(dir.join("d.md").exists());

        // The parent is now the folder-backed main node.
        {
            let nodes = NODES.read().unwrap();
            let a = nodes.iter().find(|n| n.name == "a").unwrap();
            assert!(a.has_subgraph);
            assert!(a.folder_backed);
            // The packed children left the current view entirely.
            assert!(nodes.iter().all(|n| n.name != "b" && n.name != "c"));
            assert!(nodes.iter().any(|n| n.name == "d"));
        }

        // The sub-graph folder holds the main note plus the packed children.
        let sub_files = filesystem::scan_directory(&dir.join("a"));
        let names: Vec<String> = sub_files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec!["a.md", "b.md", "c.md"]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn assets_follow_their_notes_into_the_subgraph_and_back() {
        let _guard = TEST_NAV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join("rg_fold_assets_test");
        let _ = std::fs::remove_dir_all(&dir);
        filesystem::create_dir(&dir);
        filesystem::create_dir(&dir.join("assets"));
        filesystem::create_dir(&dir.join("assets").join("sub"));
        filesystem::write_file(&dir.join("assets").join("pic.png"), "png-bytes");
        filesystem::write_file(&dir.join("assets").join("sub").join("20240513.png"), "nested");
        filesystem::write_file(&dir.join("a.md"), "# A\n\n[[b]]\n");
        filesystem::write_file(
            &dir.join("b.md"),
            "---\nheader: [[assets/pic.png]]\n---\n# B\n\n[[assets/pic.png]]\n[[assets/sub/20240513.png]]\n",
        );

        *DIR_PATH.write().unwrap() = dir.clone();
        NAV_STACK.write().unwrap().clear();
        generate_nodes_from_directory(&dir);

        let a_idx = NODES
            .read()
            .unwrap()
            .iter()
            .position(|n| n.name == "a")
            .unwrap();
        assert!(fold_node(&dir, a_idx));

        // The main note moved inside its folder; the referenced assets moved
        // into the sub-graph's own assets/ tree (header + inline links, nested
        // paths preserved, root copy gone).
        assert!(dir.join("a").join("a.md").is_file());
        assert!(dir.join("a").join("assets").join("pic.png").is_file());
        assert!(dir.join("a").join("assets").join("sub").join("20240513.png").is_file());
        assert!(!dir.join("assets").join("pic.png").exists());

        // Unwrap returns them to the project assets/ tree.
        let a_idx = NODES
            .read()
            .unwrap()
            .iter()
            .position(|n| n.name == "a")
            .unwrap();
        assert!(unwrap_node(&dir, a_idx));
        assert!(dir.join("b.md").exists());
        assert!(dir.join("assets").join("pic.png").is_file());
        assert!(dir.join("assets").join("sub").join("20240513.png").is_file());
        assert!(!dir.join("a").exists(), "empty sub-graph folder removed");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fold_then_unwrap_restores_the_disk_set() {
        let _guard = TEST_NAV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join("rg_fold_roundtrip_test");
        let _ = std::fs::remove_dir_all(&dir);
        filesystem::create_dir(&dir);
        filesystem::write_file(&dir.join("a.md"), "# A\n[[b]]\n");
        filesystem::write_file(&dir.join("b.md"), "# B\n");
        // b owns a companion folder (its own assets/sub-graph); it travels
        // with the note and comes back whole.
        filesystem::create_dir(&dir.join("b"));
        filesystem::write_file(&dir.join("b").join("inner.md"), "# Inner\n");

        *DIR_PATH.write().unwrap() = dir.clone();
        NAV_STACK.write().unwrap().clear();
        generate_nodes_from_directory(&dir);

        let a_idx = NODES
            .read()
            .unwrap()
            .iter()
            .position(|n| n.name == "a")
            .unwrap();
        assert!(fold_node(&dir, a_idx));
        assert!(dir.join("a").join("a.md").exists());
        assert!(dir.join("a").join("b.md").exists());
        assert!(dir.join("a").join("b").join("inner.md").exists());

        let a_idx = NODES
            .read()
            .unwrap()
            .iter()
            .position(|n| n.name == "a")
            .unwrap();
        assert!(unwrap_node(&dir, a_idx));
        assert!(dir.join("a.md").exists());
        assert!(dir.join("b.md").exists());
        assert!(dir.join("b").join("inner.md").exists());
        assert!(!dir.join("a").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fold_skips_ghosts_and_unwritten_targets() {
        let _guard = TEST_NAV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join("rg_fold_ghost_test");
        let _ = std::fs::remove_dir_all(&dir);
        filesystem::create_dir(&dir);
        filesystem::write_file(&dir.join("a.md"), "# A\n[[b]]\n[[draft]]\n");
        filesystem::write_file(&dir.join("b.md"), "# B\n");

        *DIR_PATH.write().unwrap() = dir.clone();
        NAV_STACK.write().unwrap().clear();
        generate_nodes_from_directory(&dir);

        let a_idx = NODES
            .read()
            .unwrap()
            .iter()
            .position(|n| n.name == "a")
            .unwrap();
        assert!(fold_node(&dir, a_idx));

        // The real child moved; the ghost has no file to move. Once the main
        // note is folder-backed its own links (including the [[draft]] ghost)
        // no longer render at the parent level, so no ghost appears there.
        assert!(dir.join("a").join("b.md").exists());
        assert!(!dir.join("a").join("draft.md").exists());
        {
            let nodes = NODES.read().unwrap();
            assert!(!nodes.iter().any(|n| n.name == "draft"));
            assert!(
                !nodes.iter().any(|n| n.name == "b"),
                "packed children leave the parent view, no ghosts"
            );
            assert!(nodes.iter().any(|n| n.name == "a"));
        }

        // Opening the folder re-materializes the ghost next to its referrer.
        NAV_STACK.write().unwrap().clear();
        *DIR_PATH.write().unwrap() = dir.clone();
        navigate_into("a");
        generate_nodes_from_directory(&dir.join("a"));
        {
            let nodes = NODES.read().unwrap();
            let draft = nodes
                .iter()
                .find(|n| n.name == "draft")
                .expect("ghost kept inside the folder");
            assert_eq!(draft.path, dir.join("a").join("draft.md"));
        }

        NAV_STACK.write().unwrap().clear();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn legacy_companion_folder_is_foldable_and_migrates() {
        let _guard = TEST_NAV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join("rg_legacy_fold_test");
        let _ = std::fs::remove_dir_all(&dir);
        filesystem::create_dir(&dir);
        // Legacy layout: the note is at the top level and its folder already
        // exists, but there is no `a/a.md`. Disk says "foldable".
        filesystem::write_file(&dir.join("a.md"), "# A\n");
        filesystem::create_dir(&dir.join("a"));
        filesystem::write_file(&dir.join("a").join("legacy.md"), "# Legacy\n");

        *DIR_PATH.write().unwrap() = dir.clone();
        NAV_STACK.write().unwrap().clear();
        generate_nodes_from_directory(&dir);

        let a_idx = NODES
            .read()
            .unwrap()
            .iter()
            .position(|n| n.name == "a")
            .unwrap();
        {
            let nodes = NODES.read().unwrap();
            let a = &nodes[a_idx];
            assert!(!a.folder_backed);
            assert!(a.has_subgraph, "legacy companion folder");
            let rows = context_menu_rows(Some(a_idx), &nodes);
            assert!(rows.contains(&ContextRow::FoldSubGraph));
            assert!(rows.contains(&ContextRow::OpenSubGraph));
            assert!(!rows.contains(&ContextRow::UnwrapSubGraph));
        }

        // Re-folding migrates to the folder-backed layout.
        assert!(fold_node(&dir, a_idx));
        assert!(dir.join("a").join("a.md").exists());
        assert!(dir.join("a").join("legacy.md").exists());
        assert!(!dir.join("a.md").exists());

        let nodes = NODES.read().unwrap();
        let a_pos = nodes.iter().position(|n| n.name == "a").unwrap();
        assert!(nodes[a_pos].folder_backed);
        let rows = context_menu_rows(Some(a_pos), &nodes);
        assert!(rows.contains(&ContextRow::UnwrapSubGraph));
        drop(nodes);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn folder_backed_unwrap_needs_no_history() {
        let _guard = TEST_NAV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join("rg_backed_unwrap_test");
        let _ = std::fs::remove_dir_all(&dir);
        filesystem::create_dir(&dir);
        filesystem::create_dir(&dir.join("a"));
        filesystem::create_dir(&dir.join("a").join("assets"));
        filesystem::write_file(&dir.join("a").join("a.md"), "# A\n");
        filesystem::write_file(&dir.join("a").join("b.md"), "# B\n");
        filesystem::write_file(&dir.join("a").join("assets").join("x.png"), "x");

        *DIR_PATH.write().unwrap() = dir.clone();
        NAV_STACK.write().unwrap().clear();
        generate_nodes_from_directory(&dir);

        let a_idx = NODES
            .read()
            .unwrap()
            .iter()
            .position(|n| n.name == "a")
            .unwrap();
        {
            let nodes = NODES.read().unwrap();
            let a = &nodes[a_idx];
            assert!(a.folder_backed);
            assert!(a.has_subgraph);
            let rows = context_menu_rows(Some(a_idx), &nodes);
            assert!(rows.contains(&ContextRow::OpenSubGraph));
            assert!(rows.contains(&ContextRow::UnwrapSubGraph));
            assert!(!rows.contains(&ContextRow::FoldSubGraph));
        }

        // Disk-only eligibility: unwrap flattens even with no fold history.
        assert!(unwrap_node(&dir, a_idx));
        assert!(dir.join("a.md").exists());
        assert!(dir.join("b.md").exists());
        assert!(dir.join("assets").join("x.png").is_file());
        assert!(!dir.join("a").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fold_guards_block_boundary_crossings() {
        let _guard = TEST_NAV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join("rg_fold_guard_test");
        let _ = std::fs::remove_dir_all(&dir);
        filesystem::create_dir(&dir);

        // Outward: a packed child links to a real note staying behind.
        filesystem::write_file(&dir.join("a.md"), "# A\n[[b]]\n");
        filesystem::write_file(&dir.join("b.md"), "# B\n[[c]]\n");
        filesystem::write_file(&dir.join("c.md"), "# C\n");

        *DIR_PATH.write().unwrap() = dir.clone();
        NAV_STACK.write().unwrap().clear();
        generate_nodes_from_directory(&dir);
        let a_idx = NODES.read().unwrap().iter().position(|n| n.name == "a").unwrap();
        {
            let nodes = NODES.read().unwrap();
            let rows = context_menu_rows(Some(a_idx), &nodes);
            assert!(!rows.contains(&ContextRow::FoldSubGraph), "outward link blocks");
        }
        assert!(!fold_node(&dir, a_idx), "outward link blocks");
        assert!(dir.join("a.md").exists() && dir.join("b.md").exists());

        // Inbound: a note outside the fold set links to a to-be-packed child.
        filesystem::write_file(&dir.join("b.md"), "# B\n"); // drop the outward link
        filesystem::write_file(&dir.join("d.md"), "# D\n[[b]]\n");
        generate_nodes_from_directory(&dir);
        let a_idx = NODES.read().unwrap().iter().position(|n| n.name == "a").unwrap();
        {
            let nodes = NODES.read().unwrap();
            let rows = context_menu_rows(Some(a_idx), &nodes);
            assert!(!rows.contains(&ContextRow::FoldSubGraph), "external inbound blocks");
        }
        assert!(!fold_node(&dir, a_idx), "external inbound blocks");
        assert!(dir.join("b.md").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn folder_backed_links_render_only_when_open() {
        let _guard = TEST_NAV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join("rg_backed_render_test");
        let _ = std::fs::remove_dir_all(&dir);
        filesystem::create_dir(&dir);
        filesystem::create_dir(&dir.join("a"));
        filesystem::write_file(&dir.join("a").join("a.md"), "# A\n[[outside]]\n");
        filesystem::write_file(&dir.join("a").join("b.md"), "# B\n");
        filesystem::write_file(&dir.join("outside.md"), "# Outside\n");
        // d references the packed child b (sub-graph content -> no ghost) and
        // the main node a (referenceable -> real edge).
        filesystem::write_file(&dir.join("d.md"), "# D\n[[b]]\n[[a]]\n");

        *DIR_PATH.write().unwrap() = dir.clone();
        NAV_STACK.write().unwrap().clear();
        generate_nodes_from_directory(&dir);
        {
            let nodes = NODES.read().unwrap();
            let edges = EDGES.read().unwrap();
            let d = nodes.iter().position(|n| n.name == "d").unwrap();
            let a = nodes.iter().position(|n| n.name == "a").unwrap();
            // The main node's own link to `outside` is not rendered while closed.
            assert!(!edges.iter().any(|e| e.n1 == a));
            // External references to the main node resolve normally.
            assert!(edges.iter().any(|e| e.n1 == d && e.n2 == a));
            // References to packed content do not materialize a ghost.
            assert!(nodes.iter().all(|n| n.name != "b"));
        }

        // Opening the folder renders the main note's links again.
        NAV_STACK.write().unwrap().clear();
        *DIR_PATH.write().unwrap() = dir.clone();
        navigate_into("a");
        generate_nodes_from_directory(&dir.join("a"));
        {
            let nodes = NODES.read().unwrap();
            let a = nodes.iter().position(|n| n.name == "a").unwrap();
            // `outside` has no file inside a/, so it appears as a ghost here.
            let outside = nodes
                .iter()
                .find(|n| n.name == "outside")
                .expect("ghost when open");
            assert_eq!(outside.path, dir.join("a").join("outside.md"));
            let o = nodes.iter().position(|n| n.name == "outside").unwrap();
            let edges = EDGES.read().unwrap();
            assert!(edges.iter().any(|e| e.n1 == a && e.n2 == o));
        }

        NAV_STACK.write().unwrap().clear();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn childless_node_folds_to_an_empty_nest() {
        let _guard = TEST_NAV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join("rg_fold_empty_test");
        let _ = std::fs::remove_dir_all(&dir);
        filesystem::create_dir(&dir);
        filesystem::write_file(&dir.join("solo.md"), "# Solo\n");

        *DIR_PATH.write().unwrap() = dir.clone();
        NAV_STACK.write().unwrap().clear();
        generate_nodes_from_directory(&dir);
        let idx = NODES.read().unwrap().iter().position(|n| n.name == "solo").unwrap();
        assert!(fold_node(&dir, idx));
        assert!(dir.join("solo").join("solo.md").exists());
        assert!(!dir.join("solo.md").exists());
        let nodes = NODES.read().unwrap();
        assert!(nodes.iter().find(|n| n.name == "solo").unwrap().folder_backed);
        drop(nodes);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rename_folder_backed_node_renames_the_folder() {
        let _guard = TEST_NAV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join("rg_rename_backed_test");
        let _ = std::fs::remove_dir_all(&dir);
        filesystem::create_dir(&dir);
        filesystem::create_dir(&dir.join("a"));
        filesystem::write_file(&dir.join("a").join("a.md"), "# A\n");
        filesystem::write_file(&dir.join("a").join("b.md"), "# B\n");

        *DIR_PATH.write().unwrap() = dir.clone();
        NAV_STACK.write().unwrap().clear();
        generate_nodes_from_directory(&dir);
        let idx = NODES.read().unwrap().iter().position(|n| n.name == "a").unwrap();
        assert!(rename_node(idx, "z"));

        // The folder and its main note are renamed together: z/z.md.
        assert!(dir.join("z").join("z.md").exists());
        assert!(dir.join("z").join("b.md").exists());
        assert!(!dir.join("a").exists());
        assert!(!dir.join("z.md").exists());
        let nodes = NODES.read().unwrap();
        assert!(nodes.iter().find(|n| n.name == "z").unwrap().folder_backed);
        assert!(nodes.iter().all(|n| n.name != "b"), "b stays packed");
        drop(nodes);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn context_menu_offers_fold_then_unwrap() {
        let _guard = TEST_NAV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join("rg_menu_rows_test");
        let _ = std::fs::remove_dir_all(&dir);
        filesystem::create_dir(&dir);
        filesystem::write_file(&dir.join("a.md"), "# A\n[[b]]\n");
        filesystem::write_file(&dir.join("b.md"), "# B\n");
        filesystem::write_file(&dir.join("c.md"), "# C\n");

        *DIR_PATH.write().unwrap() = dir.clone();
        NAV_STACK.write().unwrap().clear();
        generate_nodes_from_directory(&dir);

        {
            let nodes = NODES.read().unwrap();
            let a_idx = nodes.iter().position(|n| n.name == "a").unwrap();
            let rows = context_menu_rows(Some(a_idx), &nodes);
            assert!(rows.contains(&ContextRow::FoldSubGraph));
            assert!(!rows.contains(&ContextRow::UnwrapSubGraph));
            // A childless note folds into an empty nest now that Fold and
            // "create sub-graph" are one action.
            let c_idx = nodes.iter().position(|n| n.name == "c").unwrap();
            let c_rows = context_menu_rows(Some(c_idx), &nodes);
            assert!(c_rows.contains(&ContextRow::FoldSubGraph));
        }

        let a_idx = NODES
            .read()
            .unwrap()
            .iter()
            .position(|n| n.name == "a")
            .unwrap();
        assert!(fold_node(&dir, a_idx));

        {
            let nodes = NODES.read().unwrap();
            let a_idx = nodes.iter().position(|n| n.name == "a").unwrap();
            assert!(nodes[a_idx].folder_backed);
            let rows = context_menu_rows(Some(a_idx), &nodes);
            assert!(rows.contains(&ContextRow::OpenSubGraph));
            assert!(rows.contains(&ContextRow::UnwrapSubGraph));
            assert!(!rows.contains(&ContextRow::FoldSubGraph));
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unwrap_renames_colliding_notes_instead_of_overwriting() {
        let _guard = TEST_NAV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join("rg_unwrap_collide_test");
        let _ = std::fs::remove_dir_all(&dir);
        filesystem::create_dir(&dir);
        filesystem::write_file(&dir.join("a.md"), "# A\n[[b]]\n");
        filesystem::write_file(&dir.join("b.md"), "# B\n");

        *DIR_PATH.write().unwrap() = dir.clone();
        NAV_STACK.write().unwrap().clear();
        generate_nodes_from_directory(&dir);

        let a_idx = NODES
            .read()
            .unwrap()
            .iter()
            .position(|n| n.name == "a")
            .unwrap();
        assert!(fold_node(&dir, a_idx));

        // The user recreates b at the parent level while it is packed.
        filesystem::write_file(&dir.join("b.md"), "# NEW B\n");

        let a_idx = NODES
            .read()
            .unwrap()
            .iter()
            .position(|n| n.name == "a")
            .unwrap();
        assert!(unwrap_node(&dir, a_idx));

        // Neither note is lost: the user's b.md stays, the unwrapped one comes
        // back renamed next to it.
        assert_eq!(filesystem::read_file(&dir.join("b.md")), "# NEW B\n");
        assert!(dir.join("b (2).md").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_header_path_prefers_the_notes_own_directory() {
        let _guard = TEST_NAV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join("rg_header_relative_test");
        let _ = std::fs::remove_dir_all(&dir);
        filesystem::create_dir(&dir);
        filesystem::create_dir(&dir.join("a"));
        filesystem::create_dir(&dir.join("a").join("assets"));
        filesystem::create_dir(&dir.join("assets"));
        filesystem::write_file(&dir.join("assets").join("old.png"), "root");
        filesystem::write_file(&dir.join("a").join("assets").join("new.png"), "local");

        // A packed note (moved into a/) resolves its assets locally first.
        let packed = dir.join("a").join("b.md");
        assert_eq!(
            resolve_header_path(&packed, "assets/new.png"),
            Some(dir.join("a").join("assets").join("new.png"))
        );
        // The local copy takes precedence over a root copy with the same name.
        filesystem::write_file(&dir.join("assets").join("new.png"), "root-dup");
        assert_eq!(
            resolve_header_path(&packed, "assets/new.png"),
            Some(dir.join("a").join("assets").join("new.png"))
        );
        // A root note still resolves from the project root.
        let root_note = dir.join("a.md");
        assert_eq!(
            resolve_header_path(&root_note, "assets/old.png"),
            Some(dir.join("assets").join("old.png"))
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unwrap_skips_same_named_uid_assets() {
        let _guard = TEST_NAV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join("rg_unwrap_asset_collide_test");
        let _ = std::fs::remove_dir_all(&dir);
        filesystem::create_dir(&dir);
        filesystem::create_dir(&dir.join("assets"));
        filesystem::write_file(&dir.join("assets").join("pic.png"), "png-bytes");
        filesystem::write_file(&dir.join("assets").join("u.png"), "uid-bytes");
        filesystem::write_file(&dir.join("a.md"), "# A\n\n[[b]]\n");
        filesystem::write_file(
            &dir.join("b.md"),
            "---\nheader: [[assets/pic.png]]\n---\n# B\n\n[[assets/pic.png]]\n[[assets/u.png]]\n",
        );

        *DIR_PATH.write().unwrap() = dir.clone();
        NAV_STACK.write().unwrap().clear();
        generate_nodes_from_directory(&dir);

        let a_idx = NODES
            .read()
            .unwrap()
            .iter()
            .position(|n| n.name == "a")
            .unwrap();
        assert!(fold_node(&dir, a_idx));
        // Both referenced assets moved into the sub-graph's own assets/.
        assert!(dir.join("a").join("assets").join("pic.png").is_file());
        assert!(dir.join("a").join("assets").join("u.png").is_file());
        assert!(!dir.join("assets").join("pic.png").exists());

        // While packed, the same uid-named file reappears at the project level
        // (another note restored it). Unwrap must not clobber the user's
        // copy: uid names mean the same file, so the destination wins and the
        // packed duplicate is dropped, leaving the folder empty to remove.
        filesystem::write_file(&dir.join("assets").join("pic.png"), "png-bytes");

        let a_idx = NODES
            .read()
            .unwrap()
            .iter()
            .position(|n| n.name == "a")
            .unwrap();
        assert!(unwrap_node(&dir, a_idx));
        assert_eq!(
            filesystem::read_file(&dir.join("assets").join("pic.png")),
            "png-bytes",
            "user's copy survives unwrap"
        );
        assert!(dir.join("assets").join("u.png").is_file(), "other asset returned");
        assert!(dir.join("b.md").exists());
        assert!(!dir.join("a").exists(), "empty sub-graph folder removed");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn folded_folder_is_navigable_and_shows_packed_children() {
        let _guard = TEST_NAV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join("rg_fold_navigate_test");
        let _ = std::fs::remove_dir_all(&dir);
        filesystem::create_dir(&dir);
        // A -> B -> D, where D is an unwritten ghost: folding A packs B, and
        // B's link re-materializes D inside the folder. A real D staying behind
        // would block the fold (see fold_guards_block_boundary_crossings).
        filesystem::write_file(&dir.join("a.md"), "# A\n\n[[b]]\n");
        filesystem::write_file(&dir.join("b.md"), "# B\n\n[[d]]\n");

        *DIR_PATH.write().unwrap() = dir.clone();
        NAV_STACK.write().unwrap().clear();
        generate_nodes_from_directory(&dir);

        let a_idx = NODES
            .read()
            .unwrap()
            .iter()
            .position(|n| n.name == "a")
            .unwrap();
        assert!(fold_node(&dir, a_idx));
        assert!(dir.join("a").join("a.md").exists());
        assert!(dir.join("a").join("b.md").exists());
        assert!(!dir.join("a").join("d.md").exists());

        // Navigate into the folded folder: the main note and the packed child
        // become nodes there and B's outgoing link re-materializes. D's file is
        // outside this scan, so D appears as an in-app ghost beside B.
        NAV_STACK.write().unwrap().clear();
        *DIR_PATH.write().unwrap() = dir.clone();
        navigate_into("a");
        generate_nodes_from_directory(&dir.join("a"));

        {
            let nodes = NODES.read().unwrap();
            assert!(nodes.iter().any(|n| n.name == "a"));
            assert!(nodes.iter().any(|n| n.name == "b"));
            let d = nodes
                .iter()
                .find(|n| n.name == "d")
                .expect("grandchild ghost next to its referrer");
            assert_eq!(d.path, dir.join("a").join("d.md"));
            let edges = EDGES.read().unwrap();
            let b = nodes.iter().position(|n| n.name == "b").unwrap();
            let d_idx = nodes.iter().position(|n| n.name == "d").unwrap();
            assert!(edges.iter().any(|e| e.n1 == b && e.n2 == d_idx));
        }

        NAV_STACK.write().unwrap().clear();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn logseq_constants_and_radii() {
        // "Down to the node radius": Logseq's exact page-node formula and force
        // defaults.
        assert_eq!(NODE_BASE_RADIUS, 3.8);
        assert_eq!(NODE_RADIUS_GROWTH, 3.4);
        assert_eq!(NODE_MAX_RADIUS, 15.8);
        assert!((radius_for(0) - 3.8).abs() < 1e-4, "leaf at 3.8");
        assert!((radius_for(1) - 7.2).abs() < 1e-4, "single link at 7.2");
        assert!((radius_for(100) - 15.8).abs() < 1e-4, "hub capped at 15.8");
        assert_eq!(D3_LINK_DISTANCE, 82.0);
        assert!((*PARAM_SPRING_K.read().unwrap() - 0.82).abs() < 1e-3);
        assert!((*PARAM_DAMPING.read().unwrap() - 0.6).abs() < 1e-3);
        assert!((*PARAM_REPULSION_K.read().unwrap() - 140.0).abs() < 1e-3);
        assert!((*PARAM_REPULSION_RADIUS.read().unwrap() - 420.0).abs() < 1e-3);
        assert!((*PARAM_COLLIDE_PAD.read().unwrap() - 10.0).abs() < 1e-3);
    }

}
