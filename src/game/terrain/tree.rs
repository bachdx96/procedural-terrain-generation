use crate::game::base::{Region, WorldSpace};
use euclid::{point2, point3, size2, Box2D, Box3D, Point2D};
use std::collections::HashMap;

const MAX_LEVEL: u32 = 8;
const ROOT_LEVEL_SIZE: i32 = 1 << MAX_LEVEL as i32;
const MIN_Z: i32 = -1;
const MAX_Z: i32 = 1;

pub struct Tree {
    sub_nodes: HashMap<Point2D<i32, WorldSpace>, Node>,
}

pub struct Node {
    bounds: Box3D<i32, WorldSpace>,
    sub_nodes: Option<Vec<Node>>,
    level: u32,
    remove_sub_nodes: bool,
}

impl Tree {
    pub fn new() -> Self {
        Self {
            sub_nodes: HashMap::new(),
        }
    }

    pub fn add_node(&mut self, point: &Point2D<i32, WorldSpace>) {
        if !self.sub_nodes.contains_key(point) {
            self.sub_nodes.insert(
                *point,
                Node::new(
                    Box3D::new(
                        point.extend(MIN_Z),
                        point
                            .add_size(&size2(ROOT_LEVEL_SIZE, ROOT_LEVEL_SIZE))
                            .extend(MAX_Z),
                    ),
                    0,
                ),
            );
        }
    }

    pub fn ensure_node_in_region(&mut self, region: &Region) {
        let bounding_box = Box2D::from_points(region.points()).round_out().to_i32();
        let min_x = round_down_to_multiple_of(bounding_box.min.x, ROOT_LEVEL_SIZE);
        let min_y = round_down_to_multiple_of(bounding_box.min.y, ROOT_LEVEL_SIZE);
        let mut max_x = round_up_to_multiple_of(bounding_box.max.x, ROOT_LEVEL_SIZE);
        let mut max_y = round_up_to_multiple_of(bounding_box.max.y, ROOT_LEVEL_SIZE);
        if min_x == max_x {
            max_x += ROOT_LEVEL_SIZE;
        }
        if min_y == max_y {
            max_y += ROOT_LEVEL_SIZE;
        }
        for x in (min_x..max_x).step_by(ROOT_LEVEL_SIZE as _) {
            for y in (min_y..max_y).step_by(ROOT_LEVEL_SIZE as _) {
                let point = point2(x, y);
                if self.sub_nodes.contains_key(&point) {
                    continue;
                } else {
                    let the_box =
                        Box2D::new(point, point2(x + ROOT_LEVEL_SIZE, y + ROOT_LEVEL_SIZE))
                            .to_f32();
                    if region.intersects_box(&the_box) {
                        self.add_node(&point);
                    }
                }
            }
        }
    }

    pub fn set_level_in_region(&mut self, region: &Region, level: u32) {
        for sub_node in self.sub_nodes.values_mut() {
            sub_node.set_level_in_region(region, level);
        }
    }

    pub fn leaf_iter(&self) -> LeafIter {
        LeafIter::new(self.sub_nodes.values(), &[], true, true)
    }

    pub fn leaf_iter_mut(&mut self) -> LeafIterMut {
        LeafIterMut::new(self.sub_nodes.values_mut(), &[], true, true)
    }

    pub fn leaf_intersect_regions_iter<'b>(&self, regions: &'b [Region]) -> LeafIter<'_, 'b> {
        LeafIter::new(self.sub_nodes.values(), regions, true, false)
    }

    pub fn leaf_intersect_regions_iter_if<'b, F>(
        &self,
        regions: &'b [Region],
        is_leaf: F,
    ) -> LeafIterIf<'_, 'b, F>
    where
        F: Fn(&Node, &[Node]) -> bool,
    {
        LeafIterIf::new(self.sub_nodes.values(), regions, true, false, is_leaf)
    }

    pub fn leaf_outside_regions_iter<'b>(&self, regions: &'b [Region]) -> LeafIter<'_, 'b> {
        LeafIter::new(self.sub_nodes.values(), regions, false, true)
    }

    pub fn leaf_intersect_regions_iter_mut<'b>(
        &mut self,
        regions: &'b [Region],
    ) -> LeafIterMut<'_, 'b> {
        LeafIterMut::new(self.sub_nodes.values_mut(), regions, true, false)
    }

    pub fn leaf_outside_regions_iter_mut<'b>(
        &mut self,
        regions: &'b [Region],
    ) -> LeafIterMut<'_, 'b> {
        LeafIterMut::new(self.sub_nodes.values_mut(), regions, false, true)
    }

    pub fn rebuild_tree(&mut self) {
        for sub_node in self.sub_nodes.values_mut() {
            sub_node.rebuild_tree();
        }
    }

    pub fn root_nodes(&self) -> std::collections::hash_map::Values<Point2D<i32, WorldSpace>, Node> {
        self.sub_nodes.values()
    }
}

impl Node {
    pub fn new(bounds: Box3D<i32, WorldSpace>, level: u32) -> Self {
        assert!(level <= MAX_LEVEL);
        Self {
            bounds,
            sub_nodes: None,
            level,
            remove_sub_nodes: false,
        }
    }

    pub fn intersects_region(&self, region: &Region) -> bool {
        let bounds = self.bounds;
        let the_box = Box2D::new(bounds.min.xy(), bounds.max.xy());
        region.intersects_box(&the_box.to_f32())
    }

    pub fn subdivide(&mut self) {
        if self.sub_nodes.is_some() {
            return;
        }
        let center = self.bounds.center();
        let top_left_node = Self::new(
            Box3D::new(self.bounds.min, center.xy().extend(self.bounds.max.z)),
            self.level + 1,
        );
        let top_right_node = Self::new(
            Box3D::new(
                point3(center.x, self.bounds.min.y, self.bounds.min.z),
                point3(self.bounds.max.x, center.y, self.bounds.max.z),
            ),
            self.level + 1,
        );
        let bottom_left_node = Self::new(
            Box3D::new(
                point3(self.bounds.min.x, center.y, self.bounds.min.z),
                point3(center.x, self.bounds.max.y, self.bounds.max.z),
            ),
            self.level + 1,
        );
        let bottom_right_node = Self::new(
            Box3D::new(center.xy().extend(self.bounds.min.z), self.bounds.max),
            self.level + 1,
        );
        self.sub_nodes = Some(vec![
            top_left_node,
            top_right_node,
            bottom_left_node,
            bottom_right_node,
        ]);
    }

    pub fn set_level_in_region(&mut self, region: &Region, level: u32) {
        if self.intersects_region(region) {
            if self.level >= level {
                // self.sub_nodes = None;
                self.remove_sub_nodes = true;
            } else {
                if self.sub_nodes.is_none() {
                    self.subdivide();
                }
                self.remove_sub_nodes = false;
                for sub_node in self.sub_nodes.as_mut().unwrap() {
                    sub_node.set_level_in_region(region, level);
                }
            }
        }
    }

    pub fn rebuild_tree(&mut self) {
        if self.remove_sub_nodes {
            self.sub_nodes = None;
            self.remove_sub_nodes = false;
        } else if self.sub_nodes.is_some() {
            for sub_node in self.sub_nodes.as_mut().unwrap() {
                sub_node.rebuild_tree();
            }
        }
    }

    pub fn bounds(&self) -> Box3D<i32, WorldSpace> {
        self.bounds
    }

    pub fn level(&self) -> u32 {
        self.level
    }

    pub fn sub_nodes(&self) -> Option<&Vec<Node>> {
        self.sub_nodes.as_ref()
    }
}

// TODO: Use a single function to check if node is in middle, is leaf or should skip
// instead of checking for regions and is_leaf function
pub struct LeafIter<'a, 'b> {
    // the second element of the tuple is true if it need
    // to check for collision with regions
    stack: Vec<(&'a Node, bool)>,
    regions: &'b [Region],
    intersect: bool,
    outside: bool,
}

impl<'a, 'b> LeafIter<'a, 'b> {
    pub fn new<T>(initial_nodes: T, regions: &'b [Region], intersect: bool, outside: bool) -> Self
    where
        T: IntoIterator<Item = &'a Node>,
    {
        Self {
            stack: initial_nodes
                .into_iter()
                .zip(std::iter::repeat(true))
                .collect(),
            regions,
            intersect,
            outside,
        }
    }
}

impl<'a, 'b> Iterator for LeafIter<'a, 'b> {
    type Item = &'a Node;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some((node, should_check)) = self.stack.pop() {
            let collide = should_check && self.regions.iter().any(|x| node.intersects_region(x));
            if let Some(sub_nodes) = &node.sub_nodes {
                if (self.intersect && collide) || (self.outside && (!collide || should_check)) {
                    for x in sub_nodes {
                        self.stack.push((x, collide));
                    }
                }
                self.next()
            } else if (self.intersect && collide) || (self.outside && !collide) {
                Some(node)
            } else {
                self.next()
            }
        } else {
            None
        }
    }
}

pub struct LeafIterMut<'a, 'b> {
    // the second element of the tuple is true if it need
    // to check for collision with regions
    stack: Vec<(&'a mut Node, bool)>,
    regions: &'b [Region],
    intersect: bool,
    outside: bool,
}

impl<'a, 'b> LeafIterMut<'a, 'b> {
    pub fn new<T>(initial_nodes: T, regions: &'b [Region], intersect: bool, outside: bool) -> Self
    where
        T: IntoIterator<Item = &'a mut Node>,
    {
        Self {
            stack: initial_nodes
                .into_iter()
                .zip(std::iter::repeat(true))
                .collect(),
            regions,
            intersect,
            outside,
        }
    }
}

impl<'a, 'b> Iterator for LeafIterMut<'a, 'b> {
    type Item = &'a mut Node;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some((node, should_check)) = self.stack.pop() {
            let collide = should_check && self.regions.iter().any(|x| node.intersects_region(x));
            if node.sub_nodes.is_none() {
                if (self.intersect && collide) || (self.outside && !collide) {
                    Some(node)
                } else {
                    self.next()
                }
            } else {
                let sub_nodes = node.sub_nodes.as_mut().unwrap();
                if (self.intersect && collide) || (self.outside && (!collide || should_check)) {
                    for x in sub_nodes {
                        self.stack.push((x, collide));
                    }
                }
                self.next()
            }
        } else {
            None
        }
    }
}

pub struct LeafIterIf<'a, 'b, F>
where
    F: Fn(&Node, &[Node]) -> bool,
{
    // the second element of the tuple is true if it need
    // to check for collision with regions
    stack: Vec<(&'a Node, bool)>,
    regions: &'b [Region],
    intersect: bool,
    outside: bool,
    is_leaf: F,
}

impl<'a, 'b, F> LeafIterIf<'a, 'b, F>
where
    F: Fn(&Node, &[Node]) -> bool,
{
    pub fn new<T>(
        initial_nodes: T,
        regions: &'b [Region],
        intersect: bool,
        outside: bool,
        is_leaf: F,
    ) -> Self
    where
        T: IntoIterator<Item = &'a Node>,
    {
        Self {
            stack: initial_nodes
                .into_iter()
                .zip(std::iter::repeat(true))
                .collect(),
            regions,
            intersect,
            outside,
            is_leaf,
        }
    }
}

impl<'a, 'b, F> Iterator for LeafIterIf<'a, 'b, F>
where
    F: Fn(&Node, &[Node]) -> bool,
{
    type Item = &'a Node;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some((node, should_check)) = self.stack.pop() {
            let collide = should_check && self.regions.iter().any(|x| node.intersects_region(x));
            if let Some(sub_nodes) = &node.sub_nodes {
                if (self.intersect && collide) || (self.outside && (!collide || should_check)) {
                    let is_leaf = &self.is_leaf;
                    if is_leaf(node, sub_nodes) {
                        return Some(node);
                    } else {
                        for x in sub_nodes {
                            self.stack.push((x, collide));
                        }
                    }
                }
                self.next()
            } else if (self.intersect && collide) || (self.outside && !collide) {
                Some(node)
            } else {
                self.next()
            }
        } else {
            None
        }
    }
}

fn round_down_to_multiple_of(n: i32, m: i32) -> i32 {
    if n >= 0 {
        (n / m) * m
    } else {
        ((n - m + 1) / m) * m
    }
}

fn round_up_to_multiple_of(n: i32, m: i32) -> i32 {
    if n >= 0 {
        ((n + m - 1) / m) * m
    } else {
        (n / m) * m
    }
}
