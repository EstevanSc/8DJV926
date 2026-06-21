use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

// --- DATA STRUCTURES FROM PREVIOUS STEPS ---

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Node {
    pub id: usize,
    pub x: usize,
    pub y: usize,
}

pub struct Graph {
    pub adjacency_list: HashMap<usize, Vec<usize>>,
    pub nodes: HashMap<usize, Node>,
}

impl Graph {
    pub fn new() -> Self {
        Self {
            adjacency_list: std::collections::HashMap::new(),
            nodes: std::collections::HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Portal {
    pub left: [f32; 2],
    pub right: [f32; 2],
}

// --- A* STRUCTURES ---

#[derive(Copy, Clone, PartialEq)]
struct AStarNodeState {
    node_id: usize,
    f_cost: f32,
    g_cost: f32,
}

// Invert Ordering to turn Rust's Max-Heap BinaryHeap into a Min-Heap
impl Eq for AStarNodeState {}

impl Ord for AStarNodeState {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .f_cost
            .partial_cmp(&self.f_cost)
            .unwrap_or(Ordering::Equal)
    }
}

impl PartialOrd for AStarNodeState {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

// --- 2D VECTOR UTILITIES FOR THE FUNNEL ---

fn cross_product_2d(origin: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let ax = a[0] - origin[0];
    let ay = a[1] - origin[1];
    let bx = b[0] - origin[0];
    let by = b[1] - origin[1];
    (ax * by) - (ay * bx)
}

// --- ALGORITHM IMPLEMENTATIONS ---

/// 1. Core A* Grid Pathfinding
pub fn run_a_star(graph: &Graph, start_id: usize, end_id: usize) -> Option<Vec<usize>> {
    let mut open_set = BinaryHeap::new();
    let mut came_from: HashMap<usize, usize> = HashMap::new();

    let mut g_score: HashMap<usize, f32> = HashMap::new();
    g_score.insert(start_id, 0.0);

    let goal_node = graph.nodes.get(&end_id)?;

    open_set.push(AStarNodeState {
        node_id: start_id,
        g_cost: 0.0,
        f_cost: 0.0,
    });

    while let Some(AStarNodeState {
        node_id, g_cost, ..
    }) = open_set.pop()
    {
        if node_id == end_id {
            // Reconstruct the path backwards
            let mut path = vec![end_id];
            let mut current = end_id;
            while let Some(&parent) = came_from.get(&current) {
                path.push(parent);
                current = parent;
            }
            path.reverse();
            return Some(path);
        }

        // If we found a shorter way to this node already, skip processing
        if g_cost > *g_score.get(&node_id).unwrap_or(&f32::INFINITY) {
            continue;
        }

        if let Some(neighbors) = graph.adjacency_list.get(&node_id) {
            for &neighbor_id in neighbors {
                let current_g = g_score.get(&node_id).unwrap_or(&f32::INFINITY);
                let tentative_g = current_g + 1.0; // Uniform step cost between adjacent tiles

                if tentative_g < *g_score.get(&neighbor_id).unwrap_or(&f32::INFINITY) {
                    came_from.insert(neighbor_id, node_id);
                    g_score.insert(neighbor_id, tentative_g);

                    let neighbor_node = graph.nodes.get(&neighbor_id)?;
                    // Manhattan distance heuristic
                    let mut h_cost = (neighbor_node.x as f32 - goal_node.x as f32).abs()
                        + (neighbor_node.y as f32 - goal_node.y as f32).abs();

                    // Multiply by a tiny fraction. This forces A* to pick the path that is most
                    // directly pointing at the goal, heavily reducing staircasing in open corridors.
                    h_cost *= 1.001;

                    open_set.push(AStarNodeState {
                        node_id: neighbor_id,
                        g_cost: tentative_g,
                        f_cost: tentative_g + h_cost,
                    });
                }
            }
        }
    }

    None // No path found
}

/// 2. Simple Stupid Funnel (String-Pulling) Algorithm
pub fn run_funnel(start: [f32; 2], end: [f32; 2], portals: &[Portal]) -> Vec<[f32; 2]> {
    if portals.is_empty() {
        return vec![start, end];
    }

    let mut path = vec![start];

    let mut apex = start;
    let mut left_ptr = portals[0].left;
    let mut right_ptr = portals[0].right;

    let mut left_index = 0;
    let mut right_index = 0;

    let mut i = 1;
    while i < portals.len() {
        let portal = portals[i];

        // --- Process the Right Side of the Funnel ---
        // Does the new right point tighten the funnel? (Cross product check)
        if cross_product_2d(apex, right_ptr, portal.right) <= 0.0 {
            // Did it cross over the left boundary?
            if apex == left_ptr || cross_product_2d(apex, left_ptr, portal.right) > 0.0 {
                // Tighten the right side
                right_ptr = portal.right;
                right_index = i;
            } else {
                // Crossover! The funnel snapped shut onto the left boundary vertex.
                apex = left_ptr;
                path.push(apex);

                // Restart from the last snapped index point
                i = left_index;
                if i + 1 < portals.len() {
                    left_ptr = portals[i + 1].left;
                    right_ptr = portals[i + 1].right;
                }
                left_index = i;
                right_index = i;
                i += 1;
                continue;
            }
        }

        // --- Process the Left Side of the Funnel ---
        // Does the new left point tighten the funnel?
        if cross_product_2d(apex, left_ptr, portal.left) >= 0.0 {
            // Did it cross over the right boundary?
            if apex == right_ptr || cross_product_2d(apex, right_ptr, portal.left) < 0.0 {
                // Tighten the left side
                left_ptr = portal.left;
                left_index = i;
            } else {
                // Crossover! The funnel snapped shut onto the right boundary vertex.
                apex = right_ptr;
                path.push(apex);

                // Restart from the last snapped index point
                i = right_index;
                if i + 1 < portals.len() {
                    left_ptr = portals[i + 1].left;
                    right_ptr = portals[i + 1].right;
                }
                left_index = i;
                right_index = i;
                i += 1;
                continue;
            }
        }

        i += 1;
    }

    // Append the final destination
    path.push(end);
    path
}

pub fn build_portals_from_nodes(
    node_path: &[usize],
    graph: &Graph,
    tile_size: f32,
    offset_x: f32,
    offset_y: f32,
) -> Vec<Portal> {
    let mut portals = Vec::new();

    // REDUCED PADDING: Must be less than half of tile_size! (e.g., 8.0 or 10.0)
    let padding = 10.0;

    for window in node_path.windows(2) {
        let n1 = graph.nodes.get(&window[0]).unwrap();
        let n2 = graph.nodes.get(&window[1]).unwrap();

        // Calculate grid coordinates, then subtract the offset to return to Bevy world space
        let n1_x = (n1.x as f32 * tile_size) - offset_x;
        let n1_y = (n1.y as f32 * tile_size) - offset_y;

        if n2.x > n1.x {
            // Moving Right
            portals.push(Portal {
                left: [n1_x + tile_size, n1_y + tile_size - padding],
                right: [n1_x + tile_size, n1_y + padding],
            });
        } else if n2.x < n1.x {
            // Moving Left
            portals.push(Portal {
                left: [n1_x, n1_y + padding],
                right: [n1_x, n1_y + tile_size - padding],
            });
        } else if n2.y > n1.y {
            // Moving Up
            portals.push(Portal {
                left: [n1_x + padding, n1_y + tile_size],
                right: [n1_x + tile_size - padding, n1_y + tile_size],
            });
        } else if n2.y < n1.y {
            // Moving Down
            portals.push(Portal {
                left: [n1_x + tile_size - padding, n1_y],
                right: [n1_x + padding, n1_y],
            });
        }
    }

    portals
}

pub fn has_line_of_sight(
    graph: &Graph,
    map_width: usize,
    start_x: usize,
    start_y: usize,
    end_x: usize,
    end_y: usize,
) -> bool {
    let mut x0 = start_x as i32;
    let mut y0 = start_y as i32;
    let x1 = end_x as i32;
    let y1 = end_y as i32;

    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;

    loop {
        // Calculate the flat ID for the current grid tile
        let node_id = (y0 as usize) * map_width + (x0 as usize);

        // If the graph does NOT contain this node, it must be a wall!
        // Therefore, the straight line of sight is blocked.
        if !graph.nodes.contains_key(&node_id) {
            return false;
        }

        // We reached the target successfully
        if x0 == x1 && y0 == y1 {
            break;
        }

        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x0 += sx;
        }
        if e2 <= dx {
            err += dx;
            y0 += sy;
        }
    }

    true // The straight line is completely clear!
}
