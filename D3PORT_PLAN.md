# d3-force 3.0.0 port (Logseq-exact layout) — implementation plan

Implement EXACTLY what Logseq's global graph does, down to the node radius, using
d3-force@3.0.0's own values and formulas. All force work lives in
`src/graph/processing.rs` (nothing else is force logic except renderer slider
labels + one static read in `src/graph/renderer.rs`).

Gate rules (standing): **do not commit without asking first**; do not judge
layout visually yourself (user gives verdicts); pixel-level probe checks are OK.

---

## 0. Frozen spec (from Logseq `logic.cljs` + d3-force@3.0.0 + d3-quadtree@3.0.1)

Force setup (global view), applied with axis = global → forceCenter(0,0):

| force | value |
|---|---|
| forceLink | distance **82**, strength **0.82**, iterations 1 |
| forceManyBody | strength **−140** (all nodes), distanceMax **420**, theta 0.9 (θ²=0.81), distanceMin 1 |
| forceCollide | radius = `node.radius + 10`, strength **0.86**, iterations **2** |
| forceCenter | (0,0), strength **1** (default) |
| forceY | strength 0 (inert in global view) → omit |

Simulation:
- velocityDecay **0.6**, alphaStart 1, alphaMin 0.001, `alphaDecay = 1 − 0.001^(1/300) ≈ 0.0227628`.
- Tick order (d3 `simulation.tick`): decay alpha → link(alpha) → charge(alpha) →
  collide (2×, no alpha) → center → integrate `x += (vx *= velocityDecay)`.
- Tick budget by node count (non-tags view): `n≤120 → 160`, `n≤400 → 110`,
  `n≤900 → 90`, `else → 70`. After the budget Logseq declares the layout done.
- Seed for position-less nodes (d3 `position(i)`): `r = 10·sqrt(0.5 + i)`,
  `angle = i·π(3−√5)`. (Deterministic; no rng wobble.)
- d3-force's default `lcg()` with an undefined seed evaluates to a CONSTANT 0,
  so `jiggle = (0 − 0.5)·1e-6 = −5e-7` — a fixed constant, not random.
- Node radius (Logseq `node-radius`, pages): `3.8 + min(12, 3.4·√degree)`.

Constants to add to processing.rs (all `pub const` for tests):

```rust
pub const D3_LINK_DISTANCE: f32 = 82.0;
pub const D3_LINK_STRENGTH: f32 = 0.82;
pub const D3_CHARGE_STRENGTH: f32 = -140.0;
pub const D3_DISTANCE_MAX: f32 = 420.0;
pub const D3_COLLIDE_PAD: f32 = 10.0;
pub const D3_COLLIDE_STRENGTH: f32 = 0.86;
pub const D3_COLLIDE_ITERATIONS: usize = 2;
pub const D3_VELOCITY_DECAY: f32 = 0.6;
/// d3: alphaDecay = 1 - alphaMin^(1/300), alphaMin = 0.001 (Math.pow ≈ 0.0227628).
pub const D3_ALPHA_DECAY: f32 = 0.0227628;
/// Barnes-Hut accuracy, theta 0.9 squared.
pub const D3_THETA2: f32 = 0.81;
pub const D3_DISTANCE_MIN2: f32 = 1.0;
/// d3 lcg() default → constant 0, so jiggle = (0 - 0.5)*1e-6.
pub const D3_JIGGLE: f32 = -5.0e-7;
/// d3 initial radius/angle for position-less nodes.
pub const D3_INITIAL_RADIUS: f32 = 10.0;
pub const D3_INITIAL_ANGLE: f32 = std::f32::consts::PI * (3.0 - 5.0_f32.sqrt());
```

---

## 1. Edits — `src/graph/processing.rs`

### 1.1 Imports (lines 7-8)

Add `AtomicUsize`:

```rust
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::RwLock;
```

(Keep `use rand::prelude::*;` — still used by `add_node`.)

### 1.2 Radius constants + `radius_for` (REPLACE lines 10-30)

```rust
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
```

`apply_radii` (lines 32-57) stays as-is.

### 1.3 Delete old force constants, add d3 constants (REPLACE lines 59-104)

Delete `SPRING_REST_GAP`, `REPULSION_RADIUS`, `REPULSION_K`, `COLLIDE_PADDING`,
`COLLIDE_STRENGTH`, `RADIUS_SCALE_DEFAULT`, `RADIUS_VARIATION_DEFAULT`,
`NONLINK_ATTRACTION_DEFAULT`. Replace with the constants block from section 0
(plus a header comment: Logseq lays its global graph out with d3-force@3.0.0 —
forceLink(82, 0.82) + forceManyBody(−140@420) + forceCollide(radius+10, 0.86, ×2)
+ forceCenter(0,0), velocityVerlet with velocityDecay 0.6 over a fixed tick
budget; the sim below is a straight port of d3-force@3.0.0
simulation/link/manyBody/collide/center and d3-quadtree@3.0.1).

### 1.4 PARAM statics (REPLACE lines 106-130)

```rust
// Live-tunable force parameters (d3-force terms; defaults = Logseq's recipe).
pub static PARAM_SPRING_K: RwLock<f32> = RwLock::new(D3_LINK_STRENGTH);   // forceLink strength
pub static PARAM_DAMPING: RwLock<f32> = RwLock::new(D3_VELOCITY_DECAY);  // velocityDecay
pub static PARAM_GRAVITY_K: RwLock<f32> = RwLock::new(1.0);              // forceCenter strength
pub static PARAM_REPULSION_RADIUS: RwLock<f32> = RwLock::new(D3_DISTANCE_MAX); // charge distanceMax
pub static PARAM_REPULSION_K: RwLock<f32> = RwLock::new(140.0);          // |charge strength| (Logseq -140)
pub static PARAM_ALPHA_DECAY: RwLock<f32> = RwLock::new(D3_ALPHA_DECAY);
pub static PARAM_RADIUS_SCALE: RwLock<f32> = RwLock::new(1.0);
pub static PARAM_RADIUS_VARIATION: RwLock<f32> = RwLock::new(1.0);       // degree-growth multiplier
pub static PARAM_COLLIDE_PAD: RwLock<f32> = RwLock::new(D3_COLLIDE_PAD); // forceCollide +pad per node
```

(`ALPHA_COOLING_ENABLED`, `SHOW_FORCE_PANEL`, `ACTIVE_SLIDER` at 135-137 unchanged.)

### 1.5 SLIDER_RANGES (REPLACE lines 151-164)

```rust
// (min, max) range of each slider, in the same order as the PARAM_* list.
// Defaults are Logseq's d3-force values (renderer labels below update).
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
];
```

### 1.6 comment-only fixes (lines 178, 288, 1316-1319 area)

- Line 178 hit_test_slider doc: `0 = Spring Tightness ... 8 = Attraction` →
  `0 = Link Strength ... 8 = Collide Pad`.
- Line 288 param_values doc: `(spring_tightness ... nonlink_attraction)` →
  `(link_strength ... collide_pad)`.

### 1.7 `update_slider_from_mouse` (REPLACE lines 243-267)

New rounding + match arm (rest of function unchanged):

```rust
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
        _ => {}
    }
```

### 1.8 param persistence (lines 289-300, 304-314, 318-328)

- `param_values` index 8: `*PARAM_NONLINK_ATTRACTION.read().unwrap()` → `*PARAM_COLLIDE_PAD.read().unwrap()`.
- `set_param_values` arm 8 likewise.
- `PARAM_KEYS[8]`: `"attraction"` → `"collide_pad"`.

(`app_config.rs` needs no code change: it iterates PARAM_KEYS/param_values;
old config keys simply stop being read, Logseq defaults apply.)

### 1.9 Alpha block (lines 335-356) — remove both obsolete consts, add SIM_TICK

Keep ALPHA_START=1.0, ALPHA_REHEAT=0.3, ALPHA_TARGET=0.0. DELETE the
`ALPHA_DECAY` and `ALPHA_MIN` consts and their comment (now handled by the
PARAM + freeze-budget). Add:

```rust
// Ticks the current run has executed since the last wake. The freeze point is
// Logseq's fixed per-size budget (see layout_tick_count), not an alpha floor.
static SIM_TICK: AtomicUsize = AtomicUsize::new(0);
```

Update the SIM_SETTLED comment block to explain that freezing happens at the
Logseq tick budget.

### 1.10 Seed in `generate_nodes_from_directory` (REPLACE lines 415, 422-447)

- Remove `let mut rng = rand::rng();` (line 415; now unused here).
- Replace the rubble (lines 422-431: `let n`, `spiral_radius`, `golden_angle`,
  `center`) with:

```rust
    // d3-force's seed (d3-force@3.0.0 `position`): node i starts at radius
    // initialRadius*sqrt(0.5+i) on the golden angle, exactly the state d3 hands
    // Logseq before running its forces. No wobble: like d3, the seed is
    // deterministic and the forces alone shape the layout. Centred on the
    // screen; the forceCenter pass keeps the centroid there.
    let center = Vector2::new(config::width() as f32 / 2.0, config::height() as f32 / 2.0);
```

- Replace the position math (lines 436-446) with:

```rust
        let t = (i as f32 + 0.5); // no longer needed as a ratio
```
  (delete `let t = ...; n` entirely) and:

```rust
        let angle = D3_INITIAL_ANGLE * i as f32;
        let radius = D3_INITIAL_RADIUS * (i as f32 + 0.5).sqrt();
        nodes.push(Node {
            radius: NODE_BASE_RADIUS,
            color: Color::WHITE,
            position: Vector2::new(center.x + radius * angle.cos(), center.y + radius * angle.sin()),
            velocity: Vector2::new(0.0, 0.0),
            name,
            file_name,
            path: file.clone(),
            header: None,
            has_subgraph: filesystem::is_dir(&filesystem::subgraph_dir(file)),
        });
```

### 1.11 THE CORE — d3 port (REPLACE lines 1047-1195 entirely)

Delete `repulsion_forces`, `collide_corrections`. Insert the following new code
in their place (keep the `wake_simulation`/`update_forces` region for wave 1.13
which replaces them too).

```rust
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
            - nodes[s].position.x - nodes[s].velocity.x;
        let mut y = nodes[t].position.y + nodes[t].velocity.y
            - nodes[s].position.y - nodes[s].velocity.y;
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
// accumulators, `r` the collide quadrant bound (seed/4 notes).
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

// d3-quadtree, built the same way d3's addAll does: extent → cover(min) →
// cover(max) → add each point. Used by the charge (positions) and collide
// (positions + velocities) passes.
struct Quadtree {
    root: Option<Box<Quad>>,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
}

impl Quadtree {
    // d3 cover(): double the extent away from (x, y) until it is covered.
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
            let mut z = x1 - x0;
            if z == 0.0 {
                z = 1.0;
            }
            let mut node = self.root.take();
            // d3 only re-roots the wrapper chain when the old root was internal.
            let root_was_internal = node.as_ref().is_some_and(|q| q.leaf.is_empty());
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
            // The top wrapper spans the final expanded extent.
            if let Some(top) = node.as_mut() {
                top.x0 = x0;
                top.y0 = y0;
                top.x1 = x1;
                top.y1 = y1;
            }
            if root_was_internal {
                self.root = node;
            }
        }
        self.x0 = x0;
        self.y0 = y0;
        self.x1 = x1;
        self.y1 = y1;
    }

    // d3 add(): the data index chain lives in the quadtree itself.
    fn add(&mut self, x: f32, y: f32, idx: usize) {
        if x.is_nan() || y.is_nan() {
            return;
        }
        match self.root.as_mut() {
            Some(root) => insert_into(root, x, y, idx),
            None => {
                let mut leaf = Quad::make_leaf(self.x0, self.y0, self.x1, self.y1, idx, x, y);
                leaf.leaf = vec![idx];
                self.root = Some(Box::new(leaf));
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

fn place_chain(node: &mut Quad, chain: Vec<usize>, xp: f32, yp: f32, x: f32, y: f32, idx: usize) {
    let xm = (node.x0 + node.x1) * 0.5;
    let ym = (node.y0 + node.y1) * 0.5;
    let qn = (((y >= ym) as usize) << 1) | (x >= xm) as usize;
    let qo = (((yp >= ym) as usize) << 1) | (xp >= xm) as usize;
    if qn == qo {
        let (x0, y0, x1, y1) = quadrant_bounds(node.x0, node.y0, node.x1, node.y1, qn);
        node.child[qn] = Some(Box::new(Quad::internal(x0, y0, x1, y1)));
        place_chain(
            node.child[qn].as_mut().unwrap(),
            chain,
            xp,
            yp,
            x,
            y,
            idx,
        );
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
fn apply_collide(nodes: &mut [Node], radii: &[f32], pad: f32, strength: f32, iterations: usize) {
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
```

### 1.12 `wake_simulation` + `layout_tick_count` (REPLACE lines 1197-1203)

```rust
// Kick the force simulation out of its settled (paused) state and reheat it to
// full strength. Call after any structural change (add/remove/rename/regenerate),
// a slider tweak, or a drag so the layout recomputes, then it runs Logseq's
// tick budget and cools back down to sleep.
pub fn wake_simulation() {
    SIM_SETTLED.store(false, std::sync::atomic::Ordering::Relaxed);
    *SIM_ALPHA.write().unwrap() = ALPHA_START;
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
```

### 1.13 `update_forces` (REPLACE lines 1205-1308)

```rust
pub fn update_forces(_rl: &mut RaylibHandle) {
    // While the graph is settled the layout is at rest: skip the whole force
    // pass. Woken by structural changes and drags; re-sleeps at the Logseq
    // tick budget below.
    if SIM_SETTLED.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }

    let mut nodes = NODES.write().unwrap();
    let node_count = nodes.len();
    if node_count == 0 {
        return;
    }
    let edges = EDGES.read().unwrap();
    let is_dragging = DRAGGING_NODE.read().unwrap().is_some();

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
    apply_collide(&mut nodes, &radii, collide_pad, D3_COLLIDE_STRENGTH, D3_COLLIDE_ITERATIONS);
    let center = Vector2::new(config::width() as f32 / 2.0, config::height() as f32 / 2.0);
    apply_center(&mut nodes, center, center_strength);
    for (i, node) in nodes.iter_mut().enumerate() {
        if Some(i) == *DRAGGING_NODE.read().unwrap() {
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
```

### 1.14 Tests (lines 1310-1837 + line 2196)

**Delete** the whole old force harness + tests:
`repulsion_is_local_to_the_radius`, `non_linked_pairs_attract_through_the_grid`,
`grid_repulsion_matches_brute_force`, `collision_pushes_overlapping_discs_apart_equally`,
`collision_grid_matches_brute_force`, `settle_layout`, `collide_adjust`,
`sparse_tree_layout_converges_to_short_edges`,
`dense_graph_force_layout_freezes_without_overlapping_discs`,
`settled_pairs_stay_clear_regardless_of_size`,
`alpha_cooling_guarantees_rest_for_cramped_graphs`, and the old
`spring_rest_matches_logseq_link_distance`.

**Keep as-is**: `force_panel_hit_testing_tracks_shared_geometry`,
`node_size_counts_connections_in_both_directions`, and every non-force test
(`refresh_saved_node_diffs_outgoing_edges`, `wikilink_ghosts_materialize_...`,
`resolve_wikilink_folder_first_then_tree`, `navigate_into_pushes_...`,
`navigate_to_level_truncates_stack`, `rename_rewrites_links_in_other_notes`,
`attach_header_writes_frontmatter_and_refreshes_cache`).

**Add** (all in `mod tests`):

```rust
    // Real d3 pipeline replica: the exact force order, alpha model, collide
    // iterations, velocity integration, and Logseq tick budget that
    // update_forces runs — with Logseq's constants baked in (this is the
    // production default, not sliders). Returns (positions, velocities).
    // Reuses the production apply_* functions directly.
    fn d3_settle(n: usize, edges: &[(usize, usize)], radii: &[f32]) -> (Vec<Vector2>, Vec<Vector2>) {
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
                }
            })
            .collect();
        let edge_structs: Vec<Edge> = edges.iter().map(|&(a, b)| Edge { n1: a, n2: b }).collect();
        let mut alpha = 1.0_f32;
        let budget = layout_tick_count(n);
        for _ in 0..budget {
            alpha += (ALPHA_TARGET - alpha) * D3_ALPHA_DECAY;
            let meta = build_link_meta(n, &edge_structs, D3_LINK_DISTANCE, D3_LINK_STRENGTH);
            apply_link(&mut sim, &edge_structs, &meta, alpha);
            apply_charge(&mut sim, alpha, D3_DISTANCE_MAX, D3_CHARGE_STRENGTH);
            let rr: Vec<f32> = sim.iter().map(|nd| nd.radius).collect();
            apply_collide(&mut sim, &rr, D3_COLLIDE_PAD, D3_COLLIDE_STRENGTH, D3_COLLIDE_ITERATIONS);
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

    fn layout_stats(positions: &[Vector2], radii: &[f32]) -> (usize, f32, f32) {
        let mut overlaps = 0usize;
        let mut min_pair = f32::MAX;
        let (mut lo_x, mut hi_x, mut lo_y, mut hi_y) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
        for p in positions {
            lo_x = lo_x.min(p.x);
            hi_x = hi_x.max(p.x);
            lo_y = lo_y.min(p.y);
            hi_y = hi_y.max(p.y);
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
            if a == b {
                continue;
            }
            let d = (positions[a] - positions[b]).length();
            sum += d;
            max = max.max(d);
            count += 1;
        }
        (sum / count as f32, max, count)
    }
```

New force tests:

```rust
    #[test]
    fn link_rest_length_matches_logseq_82() {
        // A lone connected pair must settle near the 82-unit rest length that
        // Logseq's forceLink uses (charge widens it slightly; collide only
        // enforces a floor).
        let n = 2usize;
        let radii = vec![NODE_BASE_RADIUS; n];
        let edges = vec![(0usize, 1usize)];
        let (positions, _) = d3_settle(n, &edges, &radii);
        let sep = (positions[1] - positions[0]).length();
        assert!(
            (70.0..=115.0).contains(&sep),
            "two linked discs should rest near 82, got {sep:.1}"
        );
    }

    #[test]
    fn charge_repels_within_radius_and_is_silent_beyond() {
        // Two nodes 100 apart repel each other through the -140 charge ...
        let mut nodes_ = vec![
            Node {
                radius: 3.8,
                color: Color::WHITE,
                position: Vector2::new(0.0, 0.0),
                velocity: Vector2::zero(),
                name: "a".into(),
                file_name: "a.md".into(),
                path: PathBuf::from("a.md"),
                header: None,
                has_subgraph: false,
            },
            Node {
                radius: 3.8,
                color: Color::WHITE,
                position: Vector2::new(100.0, 0.0),
                velocity: Vector2::zero(),
                name: "b".into(),
                file_name: "b.md".into(),
                path: PathBuf::from("b.md"),
                header: None,
                has_subgraph: false,
            },
        ];
        apply_charge(&mut nodes_, 1.0, D3_DISTANCE_MAX, D3_CHARGE_STRENGTH);
        assert!(nodes_[0].velocity.x < 0.0, "left node must be pushed left");
        assert!(nodes_[1].velocity.x > 0.0, "right node must be pushed right");

        // ... and a pair 1000 apart (beyond distanceMax 420) feels nothing.
        let mut far_ = nodes_.clone();
        far_[1].position.x = 1000.0;
        far_[0].velocity = Vector2::zero();
        far_[1].velocity = Vector2::zero();
        apply_charge(&mut far_, 1.0, D3_DISTANCE_MAX, D3_CHARGE_STRENGTH);
        assert_eq!(far_[0].velocity.length(), 0.0, "no force beyond the cutoff");
        assert_eq!(far_[1].velocity.length(), 0.0);
    }

    #[test]
    fn collide_separates_overlapping_discs_per_d3_weights() {
        // Two equal discs 5 apart: pad 10 each → collide radius 13.8+13.8,
        // strength 0.86. Query i=0 vs head 1 → impulse magnitude
        // (27.6-5)/5*0.86 = 3.8872, split 50/50 because ri == rj.
        let mut nodes_ = vec![
            Node {
                radius: 3.8,
                color: Color::WHITE,
                position: Vector2::new(0.0, 0.0),
                velocity: Vector2::zero(),
                name: "a".into(),
                file_name: "a.md".into(),
                path: PathBuf::from("a.md"),
                header: None,
                has_subgraph: false,
            },
            Node {
                radius: 3.8,
                color: Color::WHITE,
                position: Vector2::new(5.0, 0.0),
                velocity: Vector2::zero(),
                name: "b".into(),
                file_name: "b.md".into(),
                path: PathBuf::from("b.md"),
                header: None,
                has_subgraph: false,
            },
        ];
        let radii = vec![3.8f32, 3.8];
        apply_collide(&mut nodes_, &radii, 10.0, 0.86, 1);
        let expected = (27.6 - 5.0) / 5.0 * 0.86 * 0.5; // ≈ 1.9436
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
        assert!(nodes_[0].velocity.y == 0.0 && nodes_[1].velocity.y == 0.0);

        // A clear pair feels nothing (r = 27.6, but they're 40 apart).
        nodes_[1].position.x = 40.0;
        nodes_[1].velocity = Vector2::zero();
        nodes_[0].velocity = Vector2::zero();
        apply_collide(&mut nodes_, &radii, 10.0, 0.86, 1);
        assert_eq!(nodes_[0].velocity.length(), 0.0);
        assert_eq!(nodes_[1].velocity.length(), 0.0);
    }

    #[test]
    fn dense_graph_force_layout_freezes_without_overlapping_discs() {
        // Regression guard for the "big graphs pile up in the centre" bug,
        // now against the real d3 pipeline. Collide guarantees every pair sits
        // at least ri+pad+rj+pad apart after the budget, so no disc can touch.
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
        assert!(min_pair > 25.0, "no two discs may touch, got {min_pair:.1}");
        assert!(span > 250.0, "a 64-node graph should spread out, span {span:.0}");
    }

    #[test]
    fn sparse_tree_layout_converges_to_short_edges() {
        // Trees are the long-link regime (the current bug report): with the
        // real d3 link impulse each edge must actually reach near its 82-unit
        // rest, not span hundreds of units. (Bounds calibrated after first run —
        // tighten below if the port behaves as intended.)
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
        // clear gap ≥ r1 + r2 + 2*pad. A disconnected pair lets only charge
        // spread them, still collide-separated.
        let r1 = NODE_BASE_RADIUS; // 3.8
        let r2 = NODE_MAX_RADIUS; // 15.8
        let (pos, _) = d3_settle(
            2,
            &[(0usize, 1usize)],
            &[r1, r2],
        );
        let sep = (pos[1] - pos[0]).length();
        assert!(sep >= 36.0, "connected leaf+hub must sit clearly apart, got {sep:.1}");

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
```

### 1.15 CALIBRATION PASS (do after first `cargo test`)

The graph-level bounds above (link pair `70..=115`, `min_pair > 25`,
`mean_edge < 250`, `max_edge < 350`, `span < 1000`, `sep ≥ 36 / ≥ 27`,
`max_speed < 3.0`) are first-pass guesses. After the port compiles, run the
tests once; where they fail, print the observed values (the assert messages
already carry them) and tighten/loosen the bounds to observed ± conservative
margin — BUT keep them strict enough to still catch the original bugs (tree
edges were 300-600+ before, so 250 mean / 350 max remain valid guards).

---

## 2. Edits — `src/graph/renderer.rs`

### 2.1 Slider labels (REPLACE lines 662-672)

```rust
        let labels = [
            "Link Strength",
            "Velocity Decay",
            "Center Pull",
            "Charge Radius",
            "Rep K",
            "Alpha Decay",
            "Radius Scale",
            "Radius Var.",
            "Collide Pad",
        ];
```

### 2.2 `read_slider_value` (line 741)

`8 => *PARAM_NONLINK_ATTRACTION.read().unwrap(),` → `8 => *PARAM_COLLIDE_PAD.read().unwrap(),`

### 2.3 Doc comments

- Line 729 comment: `0=Spring Tightness ... 8=Attraction` → `0=Link Strength ... 8=Collide Pad`.
- `NODE_CULL_MARGIN` (line 71) needs no change — it reads `NODE_MAX_RADIUS`,
  which is now 15.8 automatically.

---

## 3. Verification workflow

1. `cargo test -- --test-threads=1` (serial — tests share process globals).
   Iterate 1.15 calibration until green.
2. `cargo build`.
3. Relaunch on the complaint vault (fresh XDG config, no stale force params):
   ```
   pkill -x rust-sandbox; rm -rf /tmp/opencode/xdg-d3tree
   setsid nohup env DISPLAY=:0 XDG_CONFIG_HOME=/tmp/opencode/xdg-d3tree \
     ./target/debug/rust-sandbox /tmp/opencode/treevault &
   ```
   Let it settle (~160 ticks ≈ 2.7s). Do NOT judge yourself; ask the user for
   the verdict on: no long span links, no overlaps, hierarchy visible.
4. Then dense: same pattern on `/tmp/opencode/bigvault`. Ask again.
5. Iterate sliders only per user feedback; **ask before committing anything.**

Presentation note (not layout — leave unchanged unless the user asks): the
default camera zoom formula `2.2/sqrt(n)` in `generate_nodes_from_directory`
(≈line 476) was tuned for 7.0-unit discs; with Logseq's 3.8-unit discs the same
zoom makes the settled graph read a bit small. If the user complains, bump the
factor (e.g. `2.2 → 3.2`, clamp `(0.3, 2.2) → (0.4, 3.2)`) — this does not touch
the layout algorithm.

---

## 4. Open items / notes

- `ALPHA_DECAY`/`ALPHA_MIN` consts are deleted; the freeze is the Logseq tick
  budget (`layout_tick_count`). Slider 5 still tunes `PARAM_ALPHA_DECAY`
  directly for users who want slower/faster cooling within a longer budget.
- `PARAM_RADIUS_VARIATION` semantic inverts (0 old = full spread, 0 new =
  uniform). A persisted config with `radius_variation: 0.0` now shows uniforms;
  user can re-dial. Acceptable for a debug panel.
- Ghost nodes / self-loops / drags flow through the same passes; self-loops are
  skipped in `apply_link`, coincident chains handled by the quadtree.
- Source of truth for formulas: Logseq `logic.cljs` (link/charge/collide/radius/
  tick counts) and d3-force@3.0.0 (`simulation.js`, `link.js`, `manyBody.js`,
  `collide.js`, `center.js`, `jiggle.js`) + d3-quadtree@3.0.1 (`quadtree.js`,
  `add.js`, `cover.js`, `visit.js`, `visitAfter.js`) — all already fetched and
  verified.