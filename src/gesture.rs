use std::f32::consts::PI;

const SAMPLE_COUNT: usize = 64;
const MIN_MARGIN: f32 = 0.06;

#[derive(Debug, Clone, Copy, PartialEq)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GestureAction {
    ToggleTopmost,
    CloseTab,
    SearchSelection,
    CopySelection,
    OpenHistory,
}

impl GestureAction {
    #[cfg(test)]
    pub fn label(self) -> &'static str {
        match self {
            Self::ToggleTopmost => "置顶切换",
            Self::CloseTab => "关闭页面",
            Self::SearchSelection => "搜索选中内容",
            Self::CopySelection => "复制",
            Self::OpenHistory => "剪贴板历史",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct GestureMatch {
    pub action: GestureAction,
    pub score: f32,
}

#[derive(Clone)]
struct Template {
    action: GestureAction,
    vector: Vec<f32>,
}

pub struct Recognizer {
    templates: Vec<Template>,
}

impl Default for Recognizer {
    fn default() -> Self {
        Self::new()
    }
}

impl Recognizer {
    pub fn new() -> Self {
        let mut templates = Vec::new();
        for (action, variants) in template_points() {
            for points in variants {
                if let Some(vector) = normalize(&points) {
                    templates.push(Template { action, vector });
                }
            }
        }
        Self { templates }
    }

    pub fn recognize(&self, points: &[Point], threshold: f32) -> Option<GestureMatch> {
        let candidate = normalize(points)?;
        let mut by_action: Vec<(GestureAction, f32)> = Vec::new();

        for template in &self.templates {
            let score = cosine_similarity(&candidate, &template.vector);
            if let Some((_, best)) = by_action
                .iter_mut()
                .find(|(action, _)| *action == template.action)
            {
                *best = best.max(score);
            } else {
                by_action.push((template.action, score));
            }
        }

        by_action.sort_by(|a, b| b.1.total_cmp(&a.1));
        let (action, score) = *by_action.first()?;
        let second = by_action.get(1).map(|(_, score)| *score).unwrap_or(-1.0);
        let margin = score - second;
        (score >= threshold && margin >= MIN_MARGIN).then_some(GestureMatch { action, score })
    }
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

fn normalize(points: &[Point]) -> Option<Vec<f32>> {
    let points = resample(points, SAMPLE_COUNT)?;
    let min_x = points.iter().map(|p| p.x).fold(f32::INFINITY, f32::min);
    let max_x = points.iter().map(|p| p.x).fold(f32::NEG_INFINITY, f32::max);
    let min_y = points.iter().map(|p| p.y).fold(f32::INFINITY, f32::min);
    let max_y = points.iter().map(|p| p.y).fold(f32::NEG_INFINITY, f32::max);
    let scale = (max_x - min_x).max(max_y - min_y);
    if scale < 1.0 {
        return None;
    }
    let center_x = points.iter().map(|p| p.x).sum::<f32>() / points.len() as f32;
    let center_y = points.iter().map(|p| p.y).sum::<f32>() / points.len() as f32;
    let normalized: Vec<Point> = points
        .iter()
        .map(|p| Point::new((p.x - center_x) / scale, (p.y - center_y) / scale))
        .collect();

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

fn template_points() -> Vec<(GestureAction, Vec<Vec<Point>>)> {
    let up = vec![
        line(&[(50.0, 100.0), (50.0, 0.0)]),
        line(&[(45.0, 100.0), (50.0, 0.0)]),
    ];
    let l = vec![
        line(&[(25.0, 0.0), (25.0, 100.0), (100.0, 100.0)]),
        line(&[(30.0, 0.0), (25.0, 95.0), (100.0, 100.0)]),
    ];
    let v = vec![
        line(&[(0.0, 0.0), (50.0, 100.0), (100.0, 0.0)]),
        line(&[(5.0, 0.0), (48.0, 95.0), (95.0, 0.0)]),
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

    vec![
        (GestureAction::ToggleTopmost, up),
        (GestureAction::CloseTab, l),
        (GestureAction::SearchSelection, vec![s1, s2]),
        (GestureAction::CopySelection, c),
        (GestureAction::OpenHistory, v),
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
        for (action, variants) in template_points() {
            let candidate = jitter(&variants[0], 0.4);
            let matched = recognizer
                .recognize(&candidate, 0.80)
                .unwrap_or_else(|| panic!("failed to recognize {}", action.label()));
            assert_eq!(matched.action, action);
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
}
