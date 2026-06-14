# Pathfinding Architecture & Algorithm Pipeline

This document details the software architecture, geometric algorithms, and end-to-end data transformation pipeline used to service high-performance pathfinding requests. 

## 1. The World Representation Layers
To cleanly separate spatial mapping from mechanical traversal, the pathfinding engine relies on three distinct topological layers:

* **Grid Space (BitMap Topology):** A highly optimized, packed bit-array mapping static boundaries. Environmental geometry is discretized into individual binary bits where `1` denotes an impassable obstacle and `0` signifies walkable terrain.
* **Graph Space (Adjacency Topology):** A mathematical abstraction over Grid Space. It discards wall states completely, indexing only viable nodes and explicitly tracking directional travel relationships (edges) between adjacent free nodes via 1D array indexing: `ID = (y * MAP_WIDTH) + x`.
* **World Space (Simulation Vector Topology):** The continuous floating-point coordinate space where gameplay logic and entity movement occur.

---

## 2. The Core Algorithms

### A. Bresenham's Line Algorithm (Line-of-Sight Shortcut)
Before executing heavy pathfinding logic, the system uses Bresenham’s Line Algorithm as a high-performance optimization. It calculates the exact discrete grid tiles intersected by a perfectly straight line drawn between the Start and End coordinates.
* **Purpose:** If a player clicks an empty area across a room, this algorithm verifies that the direct path is unobstructed. 
* **Behavior:** It steps through the grid space. If every tile along the line exists in the walkable graph, it instantly returns the straight path and completely bypasses A* and the Funnel algorithm.

### B. A* Search (Topological Pathing)
If the line-of-sight is blocked, the engine falls back to A* (A-Star). This algorithm searches the Adjacency Graph to find the shortest sequence of orthogonal grid tiles around the obstacle.
* **The Heuristic:** It prioritizes which nodes to search using the Manhattan distance formula:
  `h(n) = |n_x - goal_x| + |n_y - goal_y|`
* **The Tie-Breaker:** In wide-open grid spaces, multiple paths can have the exact same cost, causing A* to generate unnatural "staircase" zig-zags. By multiplying the heuristic by a tiny fraction (`h_cost * 1.001`), the algorithm breaks ties and greedily favors paths that visually point directly toward the goal.
* **Output:** A raw, jagged list of grid node IDs forming a blocky path around the obstacles.

### C. Simple Stupid Funnel (String-Pulling)
Standard A* produces rigid, square tile steps. The Funnel algorithm (SSF) acts as a geometric smoother, pulling the jagged path tight like a string wrapped around pegs, transforming grid steps into smooth, direct lines of sight across open space.
* **Portals:** The algorithm converts the shared edges between adjacent A* tiles into "Portals" (a Left point and a Right point). A padding radius is applied to these portal points to ensure the final path doesn't mathematically scrape the exact pixel vertex of a wall.
* **The Math:** It maintains an `Apex` (the current pivot point) and tests consecutive portals using a 2D cross-product:
  `CrossProduct(Origin, A, B) = (A_x - Origin_x) * (B_y - Origin_y) - (A_y - Origin_y) * (B_x - Origin_x)`
* **Behavior:** If the cross-product indicates the new portal narrows the funnel, it tightens. If the Left and Right boundaries cross over one another, the funnel "snaps" shut onto the corner, dropping a permanent waypoint, and resetting the funnel for the next stretch.

---

## 3. The Interaction Flow (End-to-End Pipeline)

When an entity requests a path, the data flows sequentially through this transformation pipeline:

1. **Coordinate Localization (World -> Grid):** Continuous floating-point coordinates from the network request are offset to positive values and divided by `TILE_SIZE` to locate the exact starting and ending grid cells.
2. **Line of Sight Gatekeeper:** Bresenham's algorithm attempts a direct line. If successful, the pipeline terminates early and returns the start and end vectors.
3. **Topological Route Resolution:** A* navigates the Graph Space, outputting a sequence of discrete `usize` node IDs.
4. **Portal Construction (Graph -> Edge Geometries):** The system loops through the A* node sequence in pairs, determining the directional heading (Up, Down, Left, Right). It generates a physical `Portal` gate at each boundary, applying the left/right hand rules and corner padding.
5. **String-Pulling Optimization:** The Portals are fed into the Funnel Algorithm, which culls unnecessary zig-zags and snaps the trajectory tight around corners.
6. **Network Egress:** The finalized, smoothed vector waypoints are packed into a `PathResponsePayload` and published back to the client.