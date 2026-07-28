use serde::{Deserialize, Serialize};
use std::f32::consts::PI;

const SAMPLE_COUNT: usize = 64;
const MIN_MARGIN: f32 = 0.06;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    fn distance(self, other: Self) -> f32 {
        ((self.x - other.x).powi(2) + (self.y - other.y).powi(2)).sqrt()
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GestureId {
    #[serde(alias = "toggle_topmost")]
    Up,
    #[serde(alias = "close_tab")]
    LetterL,
    #[serde(alias = "search_selection")]
    LetterS,
    #[serde(alias = "copy_selection")]
    LetterC,
    #[serde(alias = "open_history")]
    LetterV,
    #[serde(alias = "switch_desktop_left")]
    Left,
    #[serde(alias = "switch_desktop_right")]
    Right,
    Seven,
    Circle,
}

impl GestureId {
    pub const ALL: [Self; 9] = [
        Self::Up,
        Self::LetterL,
        Self::LetterS,
        Self::LetterC,
        Self::LetterV,
        Self::Left,
        Self::Right,
        Self::Seven,
        Self::Circle,
    ];

    pub fn short_label(self) -> &'static str {
        match self {
            Self::Up => "↑ 上划",
            Self::LetterL => "L 字形",
            Self::LetterS => "S 字形",
            Self::LetterC => "C 字形",
            Self::LetterV => "V 字形",
            Self::Left => "← 左划",
            Self::Right => "→ 右划",
            Self::Seven => "7 字形",
            Self::Circle => "○ 圆形",
        }
    }

    #[cfg(test)]
    pub fn label(self) -> &'static str {
        match self {
            Self::Up => "上划",
            Self::LetterL => "L 字形",
            Self::LetterS => "S 字形",
            Self::LetterC => "C 字形",
            Self::LetterV => "V 字形",
            Self::Left => "左划",
            Self::Right => "右划",
            Self::Seven => "7 字形",
            Self::Circle => "圆形",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UserGestureTemplate {
    #[serde(alias = "action")]
    pub gesture: GestureId,
    pub points: Vec<Point>,
}

impl UserGestureTemplate {
    pub fn from_stroke(gesture: GestureId, points: &[Point]) -> Option<Self> {
        Some(Self {
            gesture,
            points: normalized_points(points)?,
        })
    }

    pub fn is_valid(&self) -> bool {
        self.points.len() == SAMPLE_COUNT
            && self
                .points
                .iter()
                .all(|point| point.x.is_finite() && point.y.is_finite())
            && normalize(&self.points).is_some()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct GestureMatch {
    pub gesture: GestureId,
    pub score: f32,
}

#[derive(Clone)]
struct Template {
    gesture: GestureId,
    vector: Vec<f32>,
    closed: bool,
}

pub struct Recognizer {
    built_in_templates: Vec<Template>,
    user_templates: Vec<Template>,
}

impl Default for Recognizer {
    fn default() -> Self {
        Self::new()
    }
}

impl Recognizer {
    pub fn new() -> Self {
        let mut templates = Vec::new();
        for (gesture, variants) in template_points() {
            for points in variants {
                if let Some(vector) = normalize(&points) {
                    templates.push(Template {
                        gesture,
                        vector,
                        closed: gesture == GestureId::Circle,
                    });
                }
            }
        }
        Self {
            built_in_templates: templates,
            user_templates: Vec::new(),
        }
    }

    pub fn set_user_templates(&mut self, samples: &[UserGestureTemplate]) {
        self.user_templates = samples
            .iter()
            .filter_map(|sample| {
                normalize(&sample.points).map(|vector| Template {
                    gesture: sample.gesture,
                    vector,
                    closed: sample.gesture == GestureId::Circle,
                })
            })
            .collect();
    }

    pub fn recognize(&self, points: &[Point], threshold: f32) -> Option<GestureMatch> {
        let candidate = normalize(points)?;
        let candidate_closed = is_closed_path(points);
        let mut by_gesture: Vec<(GestureId, f32)> = Vec::new();

        for template in self
            .built_in_templates
            .iter()
            .chain(self.user_templates.iter())
        {
            if template.closed != candidate_closed {
                continue;
            }
            let score = cosine_similarity(&candidate, &template.vector);
            if let Some((_, best)) = by_gesture
                .iter_mut()
                .find(|(gesture, _)| *gesture == template.gesture)
            {
                *best = best.max(score);
            } else {
                by_gesture.push((template.gesture, score));
            }
        }

        by_gesture.sort_by(|a, b| b.1.total_cmp(&a.1));
        let (gesture, score) = *by_gesture.first()?;
        let second = by_gesture.get(1).map(|(_, score)| *score).unwrap_or(-1.0);
        let margin = score - second;
        (score >= threshold && margin >= MIN_MARGIN).then_some(GestureMatch { gesture, score })
    }
}

fn is_closed_path(points: &[Point]) -> bool {
    let Some(first) = points.first().copied() else {
        return false;
    };
    let Some(last) = points.last().copied() else {
        return false;
    };
    let min_x = points.iter().map(|p| p.x).fold(f32::INFINITY, f32::min);
    let max_x = points.iter().map(|p| p.x).fold(f32::NEG_INFINITY, f32::max);
    let min_y = points.iter().map(|p| p.y).fold(f32::INFINITY, f32::min);
    let max_y = points.iter().map(|p| p.y).fold(f32::NEG_INFINITY, f32::max);
    let span = (max_x - min_x).max(max_y - min_y);
    span.is_finite() && span > f32::EPSILON && first.distance(last) / span <= 0.40
}

fn path_length(points: &[Point]) -> f32 {
    points
        .windows(2)
        .map(|pair| pair[0].distance(pair[1]))
        .sum()
}

fn resample(points: &[Point], count: usize) -> Option<Vec<Point>> {
    if points.len() < 2 {
        return None;
    }
    let total = path_length(points);
    if total < 1.0 {
        return None;
    }
    let interval = total / (count - 1) as f32;
    let mut result = Vec::with_capacity(count);
    result.push(points[0]);
    let mut accumulated = 0.0;
    let mut previous = points[0];
    let mut index = 1;

    while index < points.len() && result.len() < count - 1 {
        let current = points[index];
        let segment = previous.distance(current);
        if segment <= f32::EPSILON {
            previous = current;
            index += 1;
            continue;
        }
        if accumulated + segment >= interval {
            let ratio = (interval - accumulated) / segment;
            let point = Point::new(
                previous.x + ratio * (current.x - previous.x),
                previous.y + ratio * (current.y - previous.y),
            );
            result.push(point);
            previous = point;
            accumulated = 0.0;
        } else {
            accumulated += segment;
            previous = current;
            index += 1;
        }
    }

    while result.len() < count {
        result.push(*points.last()?);
    }
    Some(result)
}

fn normalized_points(points: &[Point]) -> Option<Vec<Point>> {
    let points = resample(points, SAMPLE_COUNT)?;
    let min_x = points.iter().map(|p| p.x).fold(f32::INFINITY, f32::min);
    let max_x = points.iter().map(|p| p.x).fold(f32::NEG_INFINITY, f32::max);
    let min_y = points.iter().map(|p| p.y).fold(f32::INFINITY, f32::min);
    let max_y = points.iter().map(|p| p.y).fold(f32::NEG_INFINITY, f32::max);
    let scale = (max_x - min_x).max(max_y - min_y);
    if !scale.is_finite() || scale <= f32::EPSILON {
        return None;
    }
    let center_x = points.iter().map(|p| p.x).sum::<f32>() / points.len() as f32;
    let center_y = points.iter().map(|p| p.y).sum::<f32>() / points.len() as f32;
    Some(
        points
            .iter()
            .map(|p| Point::new((p.x - center_x) / scale, (p.y - center_y) / scale))
            .collect(),
    )
}

fn normalize(points: &[Point]) -> Option<Vec<f32>> {
    let normalized = normalized_points(points)?;

    let mut vector = Vec::with_capacity((SAMPLE_COUNT - 1) * 2);
    for pair in normalized.windows(2) {
        let dx = pair[1].x - pair[0].x;
        let dy = pair[1].y - pair[0].y;
        let length = (dx * dx + dy * dy).sqrt();
        if length > f32::EPSILON {
            vector.push(dx / length);
            vector.push(dy / length);
        } else {
            vector.push(0.0);
            vector.push(0.0);
        }
    }
    let magnitude = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if magnitude <= f32::EPSILON {
        return None;
    }
    for value in &mut vector {
        *value /= magnitude;
    }
    Some(vector)
}

fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    left.iter().zip(right).map(|(a, b)| a * b).sum()
}

fn line(points: &[(f32, f32)]) -> Vec<Point> {
    let mut result = Vec::new();
    for pair in points.windows(2) {
        for step in 0..16 {
            let t = step as f32 / 16.0;
            result.push(Point::new(
                pair[0].0 + (pair[1].0 - pair[0].0) * t,
                pair[0].1 + (pair[1].1 - pair[0].1) * t,
            ));
        }
    }
    if let Some((x, y)) = points.last() {
        result.push(Point::new(*x, *y));
    }
    result
}

fn arc(center: Point, radius_x: f32, radius_y: f32, start: f32, end: f32) -> Vec<Point> {
    (0..=64)
        .map(|index| {
            let t = index as f32 / 64.0;
            let angle = start + (end - start) * t;
            Point::new(
                center.x + angle.cos() * radius_x,
                center.y + angle.sin() * radius_y,
            )
        })
        .collect()
}

fn template_points() -> Vec<(GestureId, Vec<Vec<Point>>)> {
    let up = vec![
        line(&[(50.0, 100.0), (50.0, 0.0)]),
        line(&[(45.0, 100.0), (50.0, 0.0)]),
    ];
    let desktop_left = vec![
        line(&[(100.0, 50.0), (0.0, 50.0)]),
        line(&[(100.0, 54.0), (0.0, 48.0)]),
    ];
    let desktop_right = vec![
        line(&[(0.0, 50.0), (100.0, 50.0)]),
        line(&[(0.0, 48.0), (100.0, 54.0)]),
    ];
    let l = vec![
        line(&[(25.0, 0.0), (25.0, 100.0), (100.0, 100.0)]),
        line(&[(30.0, 0.0), (25.0, 95.0), (100.0, 100.0)]),
    ];
    let v = vec![
        line(&[(0.0, 0.0), (50.0, 100.0), (100.0, 0.0)]),
        line(&[(5.0, 0.0), (48.0, 95.0), (95.0, 0.0)]),
    ];
    let seven = vec![
        line(&[(0.0, 4.0), (100.0, 4.0), (28.0, 100.0)]),
        line(&[(4.0, 0.0), (96.0, 5.0), (22.0, 100.0)]),
        line(&[(0.0, 8.0), (100.0, 0.0), (35.0, 100.0)]),
    ];
    let c = vec![
        arc(
            Point::new(50.0, 50.0),
            50.0,
            50.0,
            -PI / 4.0,
            -7.0 * PI / 4.0,
        ),
        arc(
            Point::new(50.0, 50.0),
            48.0,
            52.0,
            -PI / 5.0,
            -9.0 * PI / 5.0,
        ),
    ];

    let mut s1 = Vec::new();
    s1.extend(arc(Point::new(50.0, 27.0), 42.0, 27.0, -PI / 5.0, PI));
    s1.extend(arc(Point::new(50.0, 73.0), 42.0, 27.0, 0.0, PI * 4.0 / 5.0));
    let s2 = line(&[
        (92.0, 8.0),
        (45.0, 0.0),
        (10.0, 28.0),
        (70.0, 52.0),
        (92.0, 75.0),
        (55.0, 100.0),
        (8.0, 92.0),
    ]);

    let mut circles = Vec::new();
    for (radius_x, radius_y) in [(50.0, 50.0), (52.0, 44.0), (44.0, 52.0)] {
        for start_index in 0..8 {
            let start = start_index as f32 * PI / 4.0;
            circles.push(arc(
                Point::new(50.0, 50.0),
                radius_x,
                radius_y,
                start,
                start + 2.0 * PI,
            ));
            circles.push(arc(
                Point::new(50.0, 50.0),
                radius_x,
                radius_y,
                start,
                start - 2.0 * PI,
            ));
        }
    }

    vec![
        (GestureId::Up, up),
        (GestureId::LetterL, l),
        (GestureId::LetterS, vec![s1, s2]),
        (GestureId::LetterC, c),
        (GestureId::LetterV, v),
        (GestureId::Left, desktop_left),
        (GestureId::Right, desktop_right),
        (GestureId::Seven, seven),
        (GestureId::Circle, circles),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jitter(points: &[Point], amount: f32) -> Vec<Point> {
        points
            .iter()
            .enumerate()
            .map(|(index, point)| {
                let wave = ((index * 17) as f32).sin() * amount;
                Point::new(point.x * 1.4 + 200.0 + wave, point.y * 0.8 - 90.0 - wave)
            })
            .collect()
    }

    #[test]
    fn recognizes_all_templates_with_scale_and_jitter() {
        let recognizer = Recognizer::new();
        for (gesture, variants) in template_points() {
            let candidate = jitter(&variants[0], 0.4);
            let matched = recognizer
                .recognize(&candidate, 0.80)
                .unwrap_or_else(|| panic!("failed to recognize {}", gesture.label()));
            assert_eq!(matched.gesture, gesture);
            assert!(matched.score >= 0.80);
        }
    }

    #[test]
    fn rejects_short_and_stationary_paths() {
        let recognizer = Recognizer::new();
        assert!(recognizer.recognize(&[Point::new(0.0, 0.0)], 0.8).is_none());
        assert!(
            recognizer
                .recognize(&[Point::new(1.0, 1.0), Point::new(1.0, 1.0)], 0.8)
                .is_none()
        );
    }

    #[test]
    fn learns_a_personalized_template() {
        let zigzag = line(&[
            (0.0, 0.0),
            (30.0, 70.0),
            (60.0, 5.0),
            (90.0, 75.0),
            (120.0, 10.0),
        ]);
        let sample =
            UserGestureTemplate::from_stroke(GestureId::LetterS, &zigzag).expect("valid sample");
        assert!(sample.is_valid());

        let mut recognizer = Recognizer::new();
        recognizer.set_user_templates(&[sample]);
        let matched = recognizer
            .recognize(&jitter(&zigzag, 0.15), 0.82)
            .expect("personalized gesture should match");
        assert_eq!(matched.gesture, GestureId::LetterS);
    }

    #[test]
    fn normalized_personalized_template_remains_valid() {
        let vertical = line(&[(20.0, 120.0), (22.0, 65.0), (20.0, 5.0)]);
        let sample = UserGestureTemplate::from_stroke(GestureId::Up, &vertical)
            .expect("valid personalized stroke");
        assert_eq!(sample.points.len(), SAMPLE_COUNT);
        assert!(sample.is_valid());
    }

    #[test]
    fn horizontal_desktop_gestures_keep_their_direction() {
        let recognizer = Recognizer::new();
        let left = line(&[(180.0, 42.0), (20.0, 46.0)]);
        let right = line(&[(20.0, 46.0), (180.0, 42.0)]);

        assert_eq!(
            recognizer.recognize(&left, 0.82).unwrap().gesture,
            GestureId::Left
        );
        assert_eq!(
            recognizer.recognize(&right, 0.82).unwrap().gesture,
            GestureId::Right
        );
    }

    #[test]
    fn circle_is_tolerant_of_start_point_direction_and_aspect_ratio() {
        let recognizer = Recognizer::new();
        for candidate in [
            arc(
                Point::new(90.0, 60.0),
                62.0,
                48.0,
                PI / 7.0,
                PI / 7.0 + 2.0 * PI,
            ),
            arc(
                Point::new(90.0, 60.0),
                44.0,
                60.0,
                5.0 * PI / 6.0,
                5.0 * PI / 6.0 - 2.0 * PI,
            ),
            arc(
                Point::new(90.0, 60.0),
                57.0,
                46.0,
                PI / 3.0,
                PI / 3.0 + 1.85 * PI,
            ),
        ] {
            assert_eq!(
                recognizer
                    .recognize(&jitter(&candidate, 0.35), 0.82)
                    .unwrap()
                    .gesture,
                GestureId::Circle
            );
        }
    }

    #[test]
    fn seven_does_not_collide_with_l_or_v() {
        let recognizer = Recognizer::new();
        let candidate = line(&[(6.0, 3.0), (96.0, 6.0), (30.0, 98.0)]);
        assert_eq!(
            recognizer
                .recognize(&jitter(&candidate, 0.25), 0.82)
                .unwrap()
                .gesture,
            GestureId::Seven
        );
    }

    #[test]
    fn legacy_personalized_sample_names_deserialize_to_gesture_ids() {
        let json = r#"{
            "action": "search_selection",
            "points": []
        }"#;
        let sample: UserGestureTemplate = serde_json::from_str(json).unwrap();
        assert_eq!(sample.gesture, GestureId::LetterS);
    }
}
