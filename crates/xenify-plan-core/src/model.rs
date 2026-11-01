//! What a plan holds: projects, tasks and their links, the people, groups
//! and things given to them, the plan's own lists (types, statuses, fields)
//! and settings, baselines, and the list of every change made (history).
//!
//! Only what people chose is kept. Dates worked out from links and working
//! time (schedule.rs) are never stored, so two people editing at once never
//! disagree about them.

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use std::collections::BTreeMap;
use xenify_holidays::Date;

pub type Id = String;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Plan {
    pub title: String,
    pub settings: Settings,
    pub projects: Vec<Project>,
    pub types: Vec<TaskType>,
    pub statuses: Vec<Status>,
    pub fields: Vec<Field>,
    /// Exported copies only: the people (groups, things) its issues are
    /// given to, so the copy reads without the server. A plan on a server
    /// keeps only their ids; the register has the rest.
    pub resources: Vec<Resource>,
    pub tasks: BTreeMap<Id, Task>,
    /// Every task in breakdown order (a parent before its children).
    pub order: Vec<Id>,
    pub baselines: Vec<Baseline>,
    /// Done issues sent out together, oldest first.
    pub releases: Vec<Release>,
    /// Exported copies only: the issues in other plans this one waits for,
    /// as they were (on a server they come from the server, world.outside).
    pub frozen: BTreeMap<Id, Outside>,
    /// Exported copies only: the server's company days off and the other
    /// plans' bookings of its people, as they were.
    pub frozen_days: Vec<(Date, Date, String)>,
    pub frozen_bookings: Vec<crate::schedule::Booking>,
    /// Every change, oldest first (History). Not compared when checking
    /// two copies agree: the order can differ until the server's settles.
    pub log: Vec<Step>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// New tasks: scheduled automatically (true) or by hand.
    pub auto: bool,
    /// When the plan opens, unstarted late tasks move to today.
    pub keep_up_to_date: bool,
    pub mark_late: bool,
    /// Monday … Sunday.
    pub work_days: [bool; 7],
    /// Minutes in a working day (after lunch).
    pub day_minutes: u32,
    /// Holidays: ISO country ("GB") and region ("ENG", "" for none).
    pub country: String,
    pub region: String,
    /// People's own regions count for their tasks.
    pub own_regions: bool,
    /// Where tasks with nothing before them start.
    pub start: Option<Date>,
    /// New issues' estimate, in working days (0: none).
    pub default_days: f64,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            auto: true,
            keep_up_to_date: false,
            mark_late: true,
            work_days: [true, true, true, true, true, false, false],
            day_minutes: 450,
            country: String::new(),
            region: String::new(),
            own_regions: true,
            start: None,
            default_days: 1.0,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Project {
    pub id: Id,
    pub name: String,
    /// Task keys start with it: "WEB" gives WEB-1, WEB-2 …
    pub prefix: String,
    pub color: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TaskType {
    pub id: Id,
    pub name: String,
    pub color: String,
    /// "square" or "round".
    pub shape: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Status {
    pub id: Id,
    pub name: String,
    pub color: String,
    /// Tasks here count as finished.
    pub done: bool,
    /// Tasks here count as started.
    pub started: bool,
}

/// A field of the plan's own (Story points, Customer, Release …): what
/// kind of value it holds and the rules a value must keep (fields.rs checks
/// them). Every rule is optional.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Field {
    pub id: Id,
    pub name: String,
    /// text, long_text, number, date, choice, choices, person, check, link, email.
    pub kind: String,
    pub choices: Vec<String>,
    /// The choices' colours, in the same order ("" or missing: grey).
    pub colors: Vec<String>,
    /// Task type ids it's for; empty for all.
    pub types: Vec<Id>,
    pub on_cards: bool,
    pub in_list: bool,
    /// Asked for in the Create dialog.
    pub on_create: bool,
    /// Shown under it: what to put in.
    pub help: String,
    /// An issue without it needs details.
    pub required: bool,
    /// Numbers: the lowest and highest allowed.
    pub min: Option<f64>,
    pub max: Option<f64>,
    /// Numbers: places after the point (0: whole numbers only); None: any.
    pub decimals: Option<u32>,
    /// Numbers: shown before and after ("£", "pts", "%").
    pub prefix: String,
    pub suffix: String,
    /// Text: the fewest and most characters.
    pub min_len: Option<u32>,
    pub max_len: Option<u32>,
    /// Text: a pattern it must match (a regular expression), and what to say when it doesn't.
    pub pattern: String,
    pub pattern_says: String,
    /// Dates: the earliest and latest allowed, and "not before today".
    pub min_date: Option<Date>,
    pub max_date: Option<Date>,
    pub not_past: bool,
    /// Several choices: the fewest and most that can be picked.
    pub min_picks: Option<u32>,
    pub max_picks: Option<u32>,
    /// What a new issue starts with (null: nothing).
    pub default: Json,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResourceKind {
    #[default]
    Person,
    Group,
    Thing,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Resource {
    pub id: Id,
    pub kind: ResourceKind,
    pub name: String,
    /// The server account (people).
    pub account: Option<i64>,
    /// Minutes a working day; 0 for the plan's day.
    pub day_minutes: u32,
    /// Their own holidays (people): country and region.
    pub country: String,
    pub region: String,
    /// Groups: who is in it, and when.
    pub members: Vec<Member>,
    /// Away (out of office): first and last day, inclusive.
    pub away: Vec<(Date, Date)>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Member {
    pub person: Id,
    pub from: Option<Date>,
    pub to: Option<Date>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DepKind {
    /// Starts after the other finishes.
    #[default]
    FS,
    /// Starts after the other starts.
    SS,
    /// Finishes after the other finishes.
    FF,
    /// Finishes after the other starts.
    SF,
}

/// "This task comes after `task`."
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Dep {
    pub task: Id,
    pub kind: DepKind,
    /// Working days of gap (negative: overlap).
    pub lag: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Task {
    pub id: Id,
    /// "WEB-12": the first project's prefix and a number.
    pub key: String,
    pub title: String,
    pub kind: Id,
    pub status: Id,
    pub projects: Vec<Id>,
    pub parent: Option<Id>,
    pub description: String,
    /// Field id → value.
    pub fields: BTreeMap<Id, Json>,
    pub after: Vec<Dep>,
    pub relates: Vec<Id>,
    pub duplicates: Vec<Id>,
    /// Held back: it starts no earlier than this (dragged on the Timeline).
    pub start: Option<Date>,
    pub milestone: bool,
    /// The estimate: minutes of work. None: not estimated yet, so it isn't
    /// on the Timeline.
    pub work: Option<u32>,
    /// Percent done.
    pub done: u32,
    pub started: Option<Date>,
    pub finished: Option<Date>,
    /// Who does it (a person's resource id).
    pub who: Option<Id>,
    /// "highest", "high", "medium" (also empty) or "low".
    pub priority: String,
    /// A real deadline, when there is one (the Timeline shows if it's missed).
    pub due: Option<Date>,
    /// Labels (tags): any words, kept as typed ("Front end", "Customer A").
    pub labels: Vec<String>,
    pub comments: Vec<Comment>,
    /// Goes ahead even where its person is booked in plans made before
    /// this one (normally it waits for them): both plans then warn.
    pub overbook: bool,
    /// The release it went out in (Board › Done › Release): off the Board
    /// and the Backlog from then on.
    pub release: Option<Id>,
    /// A mirror: it shows when something in another plan is done, nothing
    /// more ("n12:*" a whole plan, "n12:t3" an epic or issue). Its dates
    /// are that one's; it has no work of its own and waits for nothing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mirror: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Comment {
    pub id: Id,
    /// Unix ms.
    pub at: i64,
    pub who: String,
    /// Who wrote it, as the server's register knows them ("u12"); only they
    /// may change or delete it, which the server checks.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub by: String,
    /// Simple formatted text (paragraphs, lists, bold, italic, links).
    pub html: String,
}

/// An issue in another plan on the same server ("n<plan>:<issue>"), as
/// this plan sees it: its key and dates always; its title and plan only for
/// people who may open that plan.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Outside {
    pub key: String,
    pub title: String,
    pub plan: String,
    pub readable: bool,
    pub placed: bool,
    pub start: Option<Date>,
    pub finish: Option<Date>,
    pub milestone: bool,
    pub done: bool,
    /// Minutes of work (its estimate).
    pub work: Option<u32>,
    /// Gone: deleted (or its plan was).
    pub gone: bool,
    /// Someone who can't open its plan: whom to ask for access.
    pub ask: String,
    /// It waits (through other plans) for this plan: a loop, so the link
    /// is left out of the dates until it's broken.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub looped: bool,
}

/// Whether a link's other end is in another plan ("n12:t3").
pub fn is_outside(id: &str) -> bool {
    id.starts_with('n')
        && id
            .split_once(':')
            .is_some_and(|(n, t)| n.len() > 1 && n[1..].bytes().all(|b| b.is_ascii_digit()) && !t.is_empty())
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Release {
    pub id: Id,
    /// "1.0", "Spring launch".
    pub name: String,
    pub date: Option<Date>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Baseline {
    pub id: Id,
    pub saved: i64,
    pub by: String,
    /// Task → (start, finish).
    pub tasks: BTreeMap<Id, (Date, Date)>,
}

/// One change as people see it in History: what was done, by whom, when.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Step {
    /// Unix ms.
    pub at: i64,
    pub who: String,
    /// "Moved Wireframes to Mon 16 Nov; 5 tasks followed".
    pub what: String,
    pub ops: Vec<crate::ops::Op>,
}

fn starter_types() -> Vec<TaskType> {
    let ty = |id: &str, name: &str, color: &str, shape: &str| TaskType {
        id: id.into(),
        name: name.into(),
        color: color.into(),
        shape: shape.into(),
    };
    vec![
        ty("epic", "Epic", "#6250d6", "square"),
        ty("story", "Story", "#1baf7a", "square"),
        ty("task", "Task", "#2a78d6", "square"),
        ty("bug", "Bug", "#B91C1C", "round"),
        ty("milestone", "Milestone", "#1C1917", "diamond"),
    ]
}

impl Plan {
    /// A new plan with the usual lists.
    pub fn starter(title: &str) -> Plan {
        let st = |id: &str, name: &str, color: &str, started: bool, done: bool| Status {
            id: id.into(),
            name: name.into(),
            color: color.into(),
            started,
            done,
        };
        Plan {
            title: title.into(),
            statuses: vec![
                st("todo", "To do", "#A8A29E", false, false),
                st("doing", "In progress", "#2a78d6", true, false),
                st("review", "In review", "#eda100", true, false),
                st("done", "Done", "#1baf7a", true, true),
            ],
            types: starter_types(),
            ..Plan::default()
        }
    }

    pub fn task(&self, id: &str) -> Option<&Task> {
        self.tasks.get(id)
    }

    pub fn resource(&self, id: &str) -> Option<&Resource> {
        self.resources.iter().find(|r| r.id == id)
    }

    pub fn status(&self, id: &str) -> Option<&Status> {
        self.statuses.iter().find(|s| s.id == id)
    }

    pub fn children(&self, id: &str) -> impl Iterator<Item = &Task> {
        self.order.iter().filter_map(|t| self.tasks.get(t)).filter(move |t| t.parent.as_deref() == Some(id))
    }

    pub fn has_children(&self, id: &str) -> bool {
        self.tasks.values().any(|t| t.parent.as_deref() == Some(id))
    }

    /// Depth in the breakdown (0 at the top).
    pub fn depth(&self, id: &str) -> usize {
        let mut n = 0;
        let mut at = self.tasks.get(id).and_then(|t| t.parent.clone());
        while let Some(p) = at {
            n += 1;
            if n > 64 {
                break;
            }
            at = self.tasks.get(&p).and_then(|t| t.parent.clone());
        }
        n
    }

    /// Whether `a` is `b` or inside it.
    pub fn within(&self, a: &str, b: &str) -> bool {
        let mut at = Some(a.to_string());
        let mut n = 0;
        while let Some(x) = at {
            if x == b {
                return true;
            }
            n += 1;
            if n > 64 {
                return false;
            }
            at = self.tasks.get(&x).and_then(|t| t.parent.clone());
        }
        false
    }

    /// The task counts as started (status, done, or a start day).
    pub fn is_started(&self, t: &Task) -> bool {
        t.started.is_some() || t.done > 0 || self.status(&t.status).is_some_and(|s| s.started || s.done)
    }

    pub fn is_done(&self, t: &Task) -> bool {
        t.finished.is_some() || t.done >= 100 || self.status(&t.status).is_some_and(|s| s.done)
    }

    /// The next free key number for `prefix`.
    pub fn next_key(&self, prefix: &str) -> String {
        let n = self
            .tasks
            .values()
            .filter_map(|t| {
                t.key.strip_prefix(prefix).and_then(|r| r.strip_prefix('-')).and_then(|n| n.parse::<u32>().ok())
            })
            .max()
            .unwrap_or(0);
        format!("{prefix}-{}", n + 1)
    }

    pub fn is_epic(&self, t: &Task) -> bool {
        t.kind == "epic" || self.has_children(&t.id)
    }

    pub fn is_milestone(&self, t: &Task) -> bool {
        t.milestone || t.kind == "milestone"
    }

    /// The plan as it is, without the history (to compare two copies).
    pub fn content(&self) -> String {
        let mut p = self.clone();
        p.log.clear();
        serde_json::to_string(&p).unwrap_or_default()
    }
}
