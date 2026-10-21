//! Where the pipes and conduits of a technical project run, and how much of
//! them it takes.
//!
//! A run is not a line drawn across a room: a conduit leaves its panel in the
//! slab, crosses above the ceiling and drops inside a wall to each box; a
//! water pipe runs in the wall or under the floor and rises to each point.
//! So a run is laid out by a premise — where it passes — and measured the way
//! it is bought: the horizontal length along its path, the vertical drops to
//! every box, the bends and the branches.
//!
//! The points are joined to their source by the shortest tree through them,
//! which is how a run is chained from box to box on site. Along the walls the
//! path follows their centerlines, never crossing a room.

use serde::Serialize;

use crate::elements::Wall;
use crate::geometry::Point2;
use crate::home::Home;
use crate::ids::FurnitureId;

/// Where a run passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Via {
    /// In the slab or above the ceiling, dropping inside the walls to each box.
    Ceiling,
    /// Under the floor or in the screed, rising inside the walls to each point.
    Floor,
    /// Inside the walls, along them, at the points' height.
    Wall,
}

impl Via {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "ceiling" | "teto" | "forro" | "laje" => Some(Self::Ceiling),
            "floor" | "piso" => Some(Self::Floor),
            "wall" | "parede" => Some(Self::Wall),
            _ => None,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Ceiling => "ceiling",
            Self::Floor => "floor",
            Self::Wall => "wall",
        }
    }
}

/// An end of a run: a point of use, or the source it comes from.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Terminal {
    pub id: Option<FurnitureId>,
    pub at: Point2,
    /// Height of the box or the fitting above the floor, cm.
    pub z: f64,
}

/// A laid-out run.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Route {
    /// The plan paths, one per link of the tree, cm.
    pub paths: Vec<Vec<Point2>>,
    /// Length along the plan, cm.
    pub horizontal: f64,
    /// Length of the drops or rises to every terminal, cm.
    pub vertical: f64,
    /// Changes of direction of 30° or more along the paths, plus the turn
    /// from the slab or floor into the wall at every drop.
    pub bends: usize,
    /// Branches: a run that splits, or passes one point to reach another.
    pub branches: usize,
    /// Terminals, source first.
    pub terminals: Vec<Terminal>,
    /// How many links each terminal has in the tree.
    pub degrees: Vec<usize>,
    /// The length from the source to each terminal along the tree, drops
    /// and stubs included, cm (0 for the source): what a cable run in star
    /// — network, TV, one cable from the rack to each point — takes.
    pub reach: Vec<f64>,
    /// Terminals the premise cannot reach as asked: a point to be run along
    /// the walls that stands in no wall. A run through them would be drawn
    /// across a room — not a run anyone can build — so it is said instead.
    pub impossible: Vec<Terminal>,
}

impl Route {
    /// Total length, cm.
    pub fn length(&self) -> f64 {
        self.horizontal + self.vertical
    }
}

/// How close under the slab a point is on the ceiling, cm.
const ON_CEILING: f64 = 50.0;

/// How far a point may be from a wall and still be in it, cm.
const IN_WALL: f64 = 60.0;

/// The walls of the storey shown, as centerline segments.
fn wall_segments(home: &Home) -> Vec<(Point2, Point2)> {
    let view = home.level_view(home.current_level());
    view.walls
        .iter()
        .filter(|w: &&Wall| w.start.distance(w.end) > 1.0)
        .map(|w| (w.start, w.end))
        .collect()
}

fn project(a: Point2, b: Point2, p: Point2) -> (f64, Point2) {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len2 = (dx * dx + dy * dy).max(1e-9);
    let t = (((p.x - a.x) * dx + (p.y - a.y) * dy) / len2).clamp(0.0, 1.0);
    (t, Point2::new(a.x + t * dx, a.y + t * dy))
}

/// A graph along the walls, with every terminal attached where it meets its
/// wall.
struct WallGraph {
    nodes: Vec<Point2>,
    edges: Vec<Vec<(usize, f64)>>,
    /// Node of each terminal.
    attach: Vec<usize>,
    /// Whether each terminal meets a wall.
    in_wall: Vec<bool>,
}

impl WallGraph {
    fn new(segments: &[(Point2, Point2)], terminals: &[Terminal]) -> Self {
        let mut nodes: Vec<Point2> = Vec::new();
        let node = |p: Point2, nodes: &mut Vec<Point2>| -> usize {
            if let Some(i) = nodes.iter().position(|q| q.distance(p) <= 2.0) {
                return i;
            }
            nodes.push(p);
            nodes.len() - 1
        };
        // Stops along each wall: its ends, and where terminals meet it.
        let mut stops: Vec<Vec<(f64, usize)>> = segments
            .iter()
            .map(|(a, b)| vec![(0.0, node(*a, &mut nodes)), (1.0, node(*b, &mut nodes))])
            .collect();
        // Walls that cross or touch in their middles meet there too.
        for i in 0..segments.len() {
            for j in 0..segments.len() {
                if i == j {
                    continue;
                }
                for end in [segments[j].0, segments[j].1] {
                    let (t, foot) = project(segments[i].0, segments[i].1, end);
                    if foot.distance(end) <= 8.0 && t > 1e-3 && t < 1.0 - 1e-3 {
                        let n = node(foot, &mut nodes);
                        stops[i].push((t, n));
                    }
                }
            }
        }
        let mut attach = Vec::with_capacity(terminals.len());
        let mut in_wall = Vec::with_capacity(terminals.len());
        for terminal in terminals {
            let nearest = segments
                .iter()
                .enumerate()
                .map(|(i, (a, b))| {
                    let (t, foot) = project(*a, *b, terminal.at);
                    (i, t, foot, foot.distance(terminal.at))
                })
                .min_by(|x, y| x.3.total_cmp(&y.3));
            match nearest {
                Some((i, t, foot, d)) if d <= IN_WALL => {
                    let n = node(foot, &mut nodes);
                    stops[i].push((t, n));
                    attach.push(n);
                    in_wall.push(true);
                }
                _ => {
                    attach.push(node(terminal.at, &mut nodes));
                    in_wall.push(false);
                }
            }
        }
        let mut edges = vec![Vec::new(); nodes.len()];
        for list in &mut stops {
            list.sort_by(|a, b| a.0.total_cmp(&b.0));
            list.dedup_by_key(|s| s.1);
            for pair in list.windows(2) {
                let (u, v) = (pair[0].1, pair[1].1);
                if u != v {
                    let w = nodes[u].distance(nodes[v]);
                    edges[u].push((v, w));
                    edges[v].push((u, w));
                }
            }
        }
        // A terminal off every wall (a ceiling light in the middle of a room)
        // reaches the nearest wall node straight, in the slab.
        for (k, terminal) in terminals.iter().enumerate() {
            let n = attach[k];
            if edges[n].is_empty()
                && let Some((m, d)) = nodes
                    .iter()
                    .enumerate()
                    .filter(|(m, _)| *m != n && !edges[*m].is_empty())
                    .map(|(m, p)| (m, p.distance(terminal.at)))
                    .min_by(|a, b| a.1.total_cmp(&b.1))
            {
                edges[n].push((m, d));
                edges[m].push((n, d));
            }
        }
        Self {
            nodes,
            edges,
            attach,
            in_wall,
        }
    }

    /// Shortest path between two nodes along the walls: its length and points.
    fn path(&self, from: usize, to: usize) -> Option<(f64, Vec<Point2>)> {
        let n = self.nodes.len();
        let mut dist = vec![f64::INFINITY; n];
        let mut prev = vec![usize::MAX; n];
        let mut done = vec![false; n];
        dist[from] = 0.0;
        for _ in 0..n {
            let u = (0..n)
                .filter(|i| !done[*i])
                .min_by(|a, b| dist[*a].total_cmp(&dist[*b]))?;
            if dist[u].is_infinite() {
                break;
            }
            if u == to {
                break;
            }
            done[u] = true;
            for &(v, w) in &self.edges[u] {
                if dist[u] + w < dist[v] {
                    dist[v] = dist[u] + w;
                    prev[v] = u;
                }
            }
        }
        if dist[to].is_infinite() {
            return None;
        }
        let mut points = vec![self.nodes[to]];
        let mut at = to;
        while at != from {
            at = prev[at];
            points.push(self.nodes[at]);
        }
        points.reverse();
        Some((dist[to], points))
    }
}

/// Turns of 30° or more along a plan path.
pub fn bends_along(path: &[Point2]) -> usize {
    path.windows(3)
        .filter(|w| {
            let (a, b, c) = (w[0], w[1], w[2]);
            let (u, v) = ((b.x - a.x, b.y - a.y), (c.x - b.x, c.y - b.y));
            let (lu, lv) = (u.0.hypot(u.1), v.0.hypot(v.1));
            if lu < 1.0 || lv < 1.0 {
                return false;
            }
            let cos = (u.0 * v.0 + u.1 * v.1) / (lu * lv);
            cos < 30f64.to_radians().cos()
        })
        .count()
}

/// The premise that costs least for this run, with every premise's route.
///
/// Suggesting means choosing what is cheapest to build — the shortest total
/// of pipe or conduit, drops included — among the premises that can be built
/// at all: a wall run is left out when a point stands in no wall.
pub fn cheapest(
    home: &Home,
    source: Terminal,
    points: &[Terminal],
    storey: f64,
) -> (Via, Vec<(Via, Route)>) {
    let all: Vec<(Via, Route)> = [Via::Ceiling, Via::Floor, Via::Wall]
        .into_iter()
        .map(|via| (via, lay_out(home, source, points, via, storey)))
        .collect();
    let best = all
        .iter()
        .filter(|(_, r)| r.impossible.is_empty())
        .min_by(|a, b| a.1.length().total_cmp(&b.1.length()))
        .map_or(Via::Ceiling, |(via, _)| *via);
    (best, all)
}

/// Lays out a run from `source` to every one of `points`.
///
/// `storey` is the height from floor to slab, cm. Along the walls (`Wall`)
/// the path follows their centerlines; in the slab or the floor it goes
/// straight between where each point meets its wall, and turns into the wall
/// to drop or rise to the box.
pub fn lay_out(home: &Home, source: Terminal, points: &[Terminal], via: Via, storey: f64) -> Route {
    let mut terminals = vec![source];
    terminals.extend_from_slice(points);
    let n = terminals.len();
    let segments = wall_segments(home);
    let graph = WallGraph::new(&segments, &terminals);
    // Link costs between every pair of terminals, with their plan paths.
    let link = |i: usize, j: usize| -> (f64, Vec<Point2>) {
        let (a, b) = (graph.nodes[graph.attach[i]], graph.nodes[graph.attach[j]]);
        match via {
            Via::Wall => graph
                .path(graph.attach[i], graph.attach[j])
                .unwrap_or_else(|| (a.distance(b), vec![a, b])),
            Via::Ceiling | Via::Floor => (a.distance(b), vec![a, b]),
        }
    };
    let mut cost = vec![vec![(0.0, Vec::new()); n]; n];
    for (i, j) in (0..n).flat_map(|i| ((i + 1)..n).map(move |j| (i, j))) {
        let l = link(i, j);
        cost[j][i] = (l.0, l.1.iter().rev().copied().collect());
        cost[i][j] = l;
    }
    // Prim's tree from the source.
    let mut in_tree = vec![false; n];
    in_tree[0] = true;
    let mut degrees: Vec<usize> = vec![0; n];
    let mut along = vec![0.0; n];
    let mut paths = Vec::new();
    let mut horizontal = 0.0;
    let mut wall_rise = 0.0;
    let mut bends = 0;
    for _ in 1..n {
        let Some((i, j)) = (0..n)
            .filter(|i| in_tree[*i])
            .flat_map(|i| (0..n).filter(|j| !in_tree[*j]).map(move |j| (i, j)))
            .min_by(|a, b| cost[a.0][a.1].0.total_cmp(&cost[b.0][b.1].0))
        else {
            break;
        };
        in_tree[j] = true;
        along[j] = along[i] + cost[i][j].0;
        if via == Via::Wall {
            // Inside the wall the run climbs or drops between the boxes' heights.
            along[j] += (terminals[i].z - terminals[j].z).abs();
            wall_rise += (terminals[i].z - terminals[j].z).abs();
        }
        degrees[i] += 1;
        degrees[j] += 1;
        horizontal += cost[i][j].0;
        bends += bends_along(&cost[i][j].1);
        paths.push(cost[i][j].1.clone());
    }
    // The stub from each terminal into its wall, and its drop or rise.
    for (k, t) in terminals.iter().enumerate() {
        horizontal += graph.nodes[graph.attach[k]].distance(t.at);
    }
    let drop = |t: &Terminal| match via {
        Via::Ceiling => (storey - t.z).max(0.0),
        Via::Floor => t.z.max(0.0),
        Via::Wall => 0.0,
    };
    let vertical: f64 = terminals.iter().map(drop).sum::<f64>() + wall_rise;
    let stub = |k: usize| graph.nodes[graph.attach[k]].distance(terminals[k].at);
    let reach: Vec<f64> = (0..n)
        .map(|k| {
            if k == 0 {
                0.0
            } else {
                let cm = along[k] + stub(0) + stub(k) + drop(&terminals[0]) + drop(&terminals[k]);
                (cm * 10.0).round() / 10.0
            }
        })
        .collect();
    // Every drop or rise turns from the slab or the floor into the wall.
    if via != Via::Wall {
        bends += terminals.len();
    }
    // A point passed on the way to another is a branch; so is a split.
    let branches: usize = degrees.iter().map(|d| d.saturating_sub(1)).sum();
    // Inside a wall every point must be in one; from the ceiling a point
    // drops inside a wall unless it is on the ceiling itself (a light). Under
    // the floor a point rises anywhere: an island's sink, a floor outlet.
    let impossible = terminals
        .iter()
        .zip(&graph.in_wall)
        .filter(|(t, meets)| match via {
            Via::Wall => !**meets,
            Via::Ceiling => !**meets && t.z < storey - ON_CEILING,
            Via::Floor => false,
        })
        .map(|(t, _)| *t)
        .collect();
    Route {
        impossible,
        paths,
        horizontal: (horizontal * 10.0).round() / 10.0,
        vertical: (vertical * 10.0).round() / 10.0,
        bends,
        branches,
        terminals,
        degrees,
        reach,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::elements::Wall;
    use crate::ids::WallId;

    fn room() -> Home {
        let mut home = Home::default();
        let pts = [(0.0, 0.0), (400.0, 0.0), (400.0, 300.0), (0.0, 300.0)];
        for k in 0..4 {
            let (a, b) = (pts[k], pts[(k + 1) % 4]);
            home.walls.push(Wall::new(
                WallId(k as u64 + 1),
                Point2::new(a.0, a.1),
                Point2::new(b.0, b.1),
            ));
        }
        home
    }

    fn t(id: u64, x: f64, y: f64, z: f64) -> Terminal {
        Terminal {
            id: Some(FurnitureId(id)),
            at: Point2::new(x, y),
            z,
        }
    }

    #[test]
    fn along_the_walls_a_run_goes_round_the_room_not_across_it() {
        let home = room();
        // Panel on the top wall at x=0 corner side, outlet on the bottom wall.
        let source = t(1, 50.0, 0.0, 150.0);
        let outlet = t(2, 50.0, 300.0, 30.0);
        let wall = lay_out(&home, source, &[outlet], Via::Wall, 280.0);
        // Down the left wall: 50 + 300 + 50 = 400 cm, never the 300 cm across.
        assert!((wall.horizontal - 400.0).abs() < 0.5, "{wall:?}");
        assert!(wall.bends >= 2, "two corners: {wall:?}");
        assert!(
            (wall.vertical - 120.0).abs() < 1e-9,
            "inside the wall it drops from the panel to the outlet: {wall:?}"
        );

        // Through the ceiling: straight 300 cm, and the drops to both boxes.
        let ceiling = lay_out(&home, source, &[outlet], Via::Ceiling, 280.0);
        assert!((ceiling.horizontal - 300.0).abs() < 0.5, "{ceiling:?}");
        assert!(
            (ceiling.vertical - ((280.0 - 150.0) + (280.0 - 30.0))).abs() < 1e-9,
            "{ceiling:?}"
        );

        // Under the floor: the rises from the floor.
        let floor = lay_out(&home, source, &[outlet], Via::Floor, 280.0);
        assert!((floor.vertical - 180.0).abs() < 1e-9, "{floor:?}");
    }

    #[test]
    fn the_cheapest_premise_is_suggested_and_an_impossible_one_said() {
        let home = room();
        let source = t(1, 0.0, 150.0, 150.0);
        // A socket in a wall and a ceiling light in the middle of the room.
        let socket = t(2, 400.0, 150.0, 30.0);
        let light = t(3, 200.0, 150.0, 270.0);
        let wall = lay_out(&home, source, &[socket, light], Via::Wall, 280.0);
        assert_eq!(
            wall.impossible.len(),
            1,
            "the light is in no wall: {wall:?}"
        );
        assert_eq!(wall.impossible[0].id, Some(FurnitureId(3)));
        let (best, all) = cheapest(&home, source, &[socket, light], 280.0);
        assert_ne!(best, Via::Wall, "an impossible premise is never suggested");
        let chosen = all.iter().find(|(v, _)| *v == best).unwrap().1.length();
        assert!(
            all.iter()
                .filter(|(_, r)| r.impossible.is_empty())
                .all(|(_, r)| chosen <= r.length() + 1e-9),
            "{all:?}"
        );
    }

    #[test]
    fn points_are_chained_by_the_shortest_tree_and_branches_counted() {
        let home = room();
        let source = t(1, 0.0, 150.0, 150.0);
        let points = [
            t(2, 100.0, 0.0, 30.0),
            t(3, 300.0, 0.0, 30.0),
            t(4, 100.0, 300.0, 30.0),
        ];
        let route = lay_out(&home, source, &points, Via::Ceiling, 280.0);
        assert_eq!(
            route.paths.len(),
            3,
            "a tree over four ends has three links"
        );
        // A star from the source would be longer than the chain the tree picks.
        let star: f64 = points.iter().map(|p| p.at.distance(source.at)).sum();
        assert!(route.horizontal < star, "{route:?}");
        assert_eq!(route.degrees.iter().sum::<usize>(), 6);
    }

    #[test]
    fn from_the_ceiling_a_low_point_in_no_wall_cannot_be_reached_but_a_light_can() {
        let home = room();
        let source = t(1, 0.0, 150.0, 150.0);
        let island_sink = t(2, 200.0, 150.0, 60.0);
        let ceiling = lay_out(&home, source, &[island_sink], Via::Ceiling, 280.0);
        assert_eq!(
            ceiling.impossible.len(),
            1,
            "no wall to drop in: {ceiling:?}"
        );
        let floor = lay_out(&home, source, &[island_sink], Via::Floor, 280.0);
        assert!(
            floor.impossible.is_empty(),
            "it rises from the floor: {floor:?}"
        );
        let light = t(3, 200.0, 150.0, 270.0);
        assert_eq!(
            lay_out(&home, source, &[light], Via::Ceiling, 280.0).impossible,
            [] as [crate::routing::Terminal; 0]
        );
    }

    #[test]
    fn a_star_cable_runs_whole_to_each_point_and_a_wall_run_climbs_between_boxes() {
        let home = room();
        let source = t(1, 0.0, 150.0, 150.0);
        let near = t(2, 100.0, 0.0, 30.0);
        let far = t(3, 300.0, 0.0, 30.0);
        let route = lay_out(&home, source, &[near, far], Via::Ceiling, 280.0);
        assert!(route.reach[0].abs() < 1e-9);
        // The far point is reached through the near one: its cable is longer
        // than the tree's link to it, and than the near point's cable.
        assert!(route.reach[2] > route.reach[1], "{route:?}");
        let tree = route.length();
        let star: f64 = route.reach.iter().sum();
        assert!(
            star > tree,
            "two whole cables take more than the shared trunk: {route:?}"
        );

        let wall = lay_out(
            &home,
            t(1, 0.0, 150.0, 150.0),
            &[t(2, 0.0, 250.0, 30.0)],
            Via::Wall,
            280.0,
        );
        assert!(
            (wall.vertical - 120.0).abs() < 1e-9,
            "from 150 cm down to 30 cm: {wall:?}"
        );
    }
}
