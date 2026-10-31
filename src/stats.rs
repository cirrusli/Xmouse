use crate::{action::ActionKind, gesture::GestureId};
use anyhow::Result;
use rusqlite::{Connection, params};
use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS gesture_events (
        id INTEGER PRIMARY KEY,
        occurred_at INTEGER NOT NULL,
        gesture_key TEXT,
        action_key TEXT,
        outcome TEXT NOT NULL,
        score REAL,
        sample_count INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS gesture_events_time ON gesture_events(occurred_at);
";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Period {
    Day,
    Month,
    Year,
}

impl Period {
    fn strftime_pattern(self) -> &'static str {
        match self {
            Self::Day => "%Y-%m-%d",
            Self::Month => "%Y-%m",
            Self::Year => "%Y",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Success,
    Failed,
    Unrecognized,
    Cancelled,
    Disabled,
    TooShort,
}

impl Outcome {
    pub fn key(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failed => "failed",
            Self::Unrecognized => "unrecognized",
            Self::Cancelled => "cancelled",
            Self::Disabled => "disabled",
            Self::TooShort => "too_short",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct GestureEvent {
    pub gesture: Option<GestureId>,
    pub action: Option<ActionKind>,
    pub outcome: Outcome,
    pub score: Option<f32>,
    pub sample_count: usize,
}

#[derive(Debug, Clone)]
pub struct GestureRank {
    pub gesture: GestureId,
    pub count: u64,
}

#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub attempts: u64,
    pub successes: u64,
    pub failures: u64,
    pub unrecognized: u64,
    pub cancelled: u64,
    pub disabled: u64,
    pub too_short: u64,
    pub ranking: Vec<GestureRank>,
}

#[derive(Clone)]
pub struct GestureStats {
    path: PathBuf,
}

impl GestureStats {
    pub fn new(root: &Path) -> Self {
        Self {
            path: root.join("gesture-stats.db"),
        }
    }

    pub fn initialize(&self) -> Result<()> {
        let connection = self.connect()?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.execute_batch(SCHEMA)?;
        Ok(())
    }

    pub fn record(&self, event: GestureEvent) -> Result<()> {
        let connection = self.connect()?;
        insert_event(&connection, unix_now(), event)
    }

    pub fn snapshot(&self, period: Period) -> Result<Snapshot> {
        let connection = self.connect()?;
        snapshot_at(&connection, period, unix_now())
    }

    fn connect(&self) -> Result<Connection> {
        let connection = Connection::open(&self.path)?;
        connection.busy_timeout(Duration::from_millis(250))?;
        Ok(connection)
    }
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn insert_event(connection: &Connection, occurred_at: i64, event: GestureEvent) -> Result<()> {
    connection.execute(
        "INSERT INTO gesture_events
         (occurred_at, gesture_key, action_key, outcome, score, sample_count)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            occurred_at,
            event.gesture.map(gesture_key),
            event.action.map(|action| format!("{action:?}")),
            event.outcome.key(),
            event.score,
            event.sample_count as i64,
        ],
    )?;
    Ok(())
}

fn snapshot_at(connection: &Connection, period: Period, now: i64) -> Result<Snapshot> {
    let pattern = period.strftime_pattern();
    let mut snapshot = connection.query_row(
        "SELECT COUNT(*),
                COALESCE(SUM(outcome = 'success'), 0),
                COALESCE(SUM(outcome = 'failed'), 0),
                COALESCE(SUM(outcome = 'unrecognized'), 0),
                COALESCE(SUM(outcome = 'cancelled'), 0),
                COALESCE(SUM(outcome = 'disabled'), 0),
                COALESCE(SUM(outcome = 'too_short'), 0)
         FROM gesture_events
         WHERE strftime(?1, occurred_at, 'unixepoch', 'localtime') =
               strftime(?1, ?2, 'unixepoch', 'localtime')",
        params![pattern, now],
        |row| {
            Ok(Snapshot {
                attempts: row.get(0)?,
                successes: row.get(1)?,
                failures: row.get(2)?,
                unrecognized: row.get(3)?,
                cancelled: row.get(4)?,
                disabled: row.get(5)?,
                too_short: row.get(6)?,
                ranking: Vec::new(),
            })
        },
    )?;
    let mut statement = connection.prepare(
        "SELECT gesture_key, COUNT(*)
         FROM gesture_events
         WHERE gesture_key IS NOT NULL
           AND strftime(?1, occurred_at, 'unixepoch', 'localtime') =
               strftime(?1, ?2, 'unixepoch', 'localtime')
         GROUP BY gesture_key
         ORDER BY COUNT(*) DESC, gesture_key ASC",
    )?;
    let rows = statement.query_map(params![pattern, now], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
    })?;
    for row in rows {
        let (key, count) = row?;
        if let Some(gesture) = gesture_from_key(&key) {
            snapshot.ranking.push(GestureRank { gesture, count });
        }
    }
    Ok(snapshot)
}

fn gesture_key(gesture: GestureId) -> &'static str {
    match gesture {
        GestureId::Up => "up",
        GestureId::LetterL => "letter_l",
        GestureId::LetterS => "letter_s",
        GestureId::LetterC => "letter_c",
        GestureId::LetterV => "letter_v",
        GestureId::Left => "left",
        GestureId::Right => "right",
        GestureId::Seven => "seven",
        GestureId::Circle => "circle",
    }
}

fn gesture_from_key(key: &str) -> Option<GestureId> {
    Some(match key {
        "up" => GestureId::Up,
        "letter_l" => GestureId::LetterL,
        "letter_s" => GestureId::LetterS,
        "letter_c" => GestureId::LetterC,
        "letter_v" => GestureId::LetterV,
        "left" => GestureId::Left,
        "right" => GestureId::Right,
        "seven" => GestureId::Seven,
        "circle" => GestureId::Circle,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::{GestureEvent, GestureStats, Outcome, Period, SCHEMA, insert_event, snapshot_at};
    use crate::{action::ActionKind, gesture::GestureId};
    use rusqlite::Connection;

    #[test]
    fn rankings_are_scoped_to_local_day_month_and_year() {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(SCHEMA).unwrap();
        let now = 1_779_000_000;
        let event = |gesture, outcome| GestureEvent {
            gesture,
            action: Some(ActionKind::SwitchDesktopLeft),
            outcome,
            score: Some(0.93),
            sample_count: 64,
        };
        insert_event(
            &connection,
            now,
            event(Some(GestureId::Right), Outcome::Success),
        )
        .unwrap();
        insert_event(
            &connection,
            now,
            event(Some(GestureId::Right), Outcome::Failed),
        )
        .unwrap();
        insert_event(&connection, now, event(None, Outcome::Unrecognized)).unwrap();
        insert_event(&connection, now, event(None, Outcome::Cancelled)).unwrap();
        insert_event(&connection, now, event(None, Outcome::TooShort)).unwrap();
        insert_event(
            &connection,
            now - 86_400,
            event(Some(GestureId::Left), Outcome::Success),
        )
        .unwrap();
        insert_event(
            &connection,
            now - 40 * 86_400,
            event(Some(GestureId::Up), Outcome::Success),
        )
        .unwrap();

        let day = snapshot_at(&connection, Period::Day, now).unwrap();
        assert_eq!(day.attempts, 5);
        assert_eq!(day.successes, 1);
        assert_eq!(day.failures, 1);
        assert_eq!(day.unrecognized, 1);
        assert_eq!(day.cancelled, 1);
        assert_eq!(day.too_short, 1);
        assert_eq!(day.ranking.len(), 1);
        assert_eq!(day.ranking[0].gesture, GestureId::Right);
        assert_eq!(day.ranking[0].count, 2);

        let month = snapshot_at(&connection, Period::Month, now).unwrap();
        assert!(month.attempts >= day.attempts);
        let year = snapshot_at(&connection, Period::Year, now).unwrap();
        assert_eq!(year.attempts, 7);
        assert_eq!(year.ranking.iter().map(|rank| rank.count).sum::<u64>(), 4);
    }

    #[test]
    fn gesture_database_path_is_independent_of_clipboard_history() {
        let root = std::path::Path::new("C:/tmp/xmouse");
        let stats = GestureStats::new(root);
        assert_eq!(stats.path, root.join("gesture-stats.db"));
    }
}
