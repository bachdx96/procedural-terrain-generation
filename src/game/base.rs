use euclid::{point2, Box2D, Point2D};

#[derive(Debug)]
pub struct WorldSpace;

#[derive(Debug)]
pub struct ViewSpace;

#[derive(Debug)]
pub struct ScreenSpace;

#[derive(Debug)]
pub struct LocalSpace;

#[derive(Debug, Clone)]
pub struct Region(Vec<Point2D<f32, WorldSpace>>);

impl Region {
    pub fn new<T>(points: T) -> Self
    where
        T: IntoIterator<Item = Point2D<f32, WorldSpace>>,
    {
        Self(points.into_iter().collect())
    }

    pub fn contains_point(&self, point: &Point2D<f32, WorldSpace>) -> bool {
        if self.0.len() < 3 {
            return false;
        }
        // Keep track of cross product sign changes
        let mut pos = 0;
        let mut neg = 0;

        for i in 0..self.0.len() {
            if &self.0[i] == point {
                return true;
            }
            let x1 = self.0[i].x;
            let y1 = self.0[i].y;

            let i2 = (i + 1) % self.0.len();

            let x2 = self.0[i2].x;
            let y2 = self.0[i2].y;

            let x = point.x;
            let y = point.y;

            let d = (x - x1) * (y2 - y1) - (y - y1) * (x2 - x1);

            if d > 0.0 {
                pos += 1
            };
            if d < 0.0 {
                neg += 1
            };

            //If the sign changes, then point is outside
            if pos > 0 && neg > 0 {
                return false;
            }
        }
        true
    }

    pub fn intersects_line(
        &self,
        a: &Point2D<f32, WorldSpace>,
        b: &Point2D<f32, WorldSpace>,
    ) -> bool {
        if self.0.len() < 3 {
            return false;
        }

        for i in 0..self.0.len() {
            if &self.0[i] == a || &self.0[i] == b {
                return true;
            }
            let c = self.0[i];

            let i2 = (i + 1) % self.0.len();

            let d = self.0[i2];

            if line_intersects(a, b, &c, &d) {
                return true;
            }
        }
        false
    }

    pub fn intersects_box(&self, other: &Box2D<f32, WorldSpace>) -> bool {
        let bounding_box = Box2D::from_points(&self.0);
        let a = point2(other.min.x, other.min.y);
        let b = point2(other.max.x, other.min.y);
        let c = point2(other.max.x, other.max.y);
        let d = point2(other.min.x, other.max.y);
        self.intersects_line(&a, &b)
            || self.intersects_line(&b, &c)
            || self.intersects_line(&c, &d)
            || self.intersects_line(&d, &a)
            || other.contains_box(&bounding_box.to_f32())
            || self.contains_box(other)
    }

    pub fn contains_box(&self, other: &Box2D<f32, WorldSpace>) -> bool {
        for x in [other.min.x, other.max.x] {
            for y in [other.min.y, other.max.y] {
                if !self.contains_point(&point2(x, y)) {
                    return false;
                }
            }
        }
        true
    }

    pub fn points(&self) -> std::slice::Iter<Point2D<f32, WorldSpace>> {
        self.0.iter()
    }
}

// Check if line ab intersects with cd
// Does not deal with collinearity
fn line_intersects(
    a: &Point2D<f32, WorldSpace>,
    b: &Point2D<f32, WorldSpace>,
    c: &Point2D<f32, WorldSpace>,
    d: &Point2D<f32, WorldSpace>,
) -> bool {
    fn ccw(
        a: &Point2D<f32, WorldSpace>,
        b: &Point2D<f32, WorldSpace>,
        c: &Point2D<f32, WorldSpace>,
    ) -> bool {
        (c.y - a.y) * (b.x - a.x) > (b.y - a.y) * (c.x - a.x)
    }
    (ccw(a, c, d) != ccw(b, c, d)) && (ccw(a, b, c) != ccw(a, b, d))
}
