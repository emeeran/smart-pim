//! Tasks repository (T3.22–T3.27 backend): CRUD with completion, Kanban
//! columns by percent bucket, due-ordered list. Recurring tasks (rrule on
//! the task row) reuse the ICS RRULE grammar; expansion lands with the
//! task module's calendar integration (T3.26).

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use crate::error::{Error, Result};
use crate::model::Kind;
use crate::repo;

#[derive(Debug, Clone, Serialize)]
pub struct TaskRow {
    pub id: i64,
    pub item_id: i64,
    pub title: String,
    pub notes: Option<String>,
    pub due_utc: Option<String>,
    pub priority: i64,
    pub percent: i64,
    pub completed_at: Option<String>,
}

fn row_from(r: &rusqlite::Row<'_>) -> rusqlite::Result<TaskRow> {
    Ok(TaskRow {
        id: r.get(0)?,
        item_id: r.get(1)?,
        title: r.get(2)?,
        notes: r.get(3)?,
        due_utc: r.get(4)?,
        priority: r.get(5)?,
        percent: r.get(6)?,
        completed_at: r.get(7)?,
    })
}

/// Open tasks ordered by due date (NULLs last), then priority desc.
pub fn tasks_open(conn: &Connection, limit: i64) -> Result<Vec<TaskRow>> {
    let mut stmt = conn.prepare(
        "SELECT t.id, t.item_id, t.title, t.notes, t.due_utc, t.priority, t.percent, \
         t.completed_at FROM task t JOIN item i ON i.id = t.item_id \
         WHERE i.deleted_at IS NULL AND t.completed_at IS NULL \
         ORDER BY (t.due_utc IS NULL), t.due_utc, t.priority DESC, t.id LIMIT ?1",
    )?;
    let rows = stmt.query_map([limit], row_from)?;
    Ok(rows.flatten().collect())
}

/// Completed tasks, newest first.
pub fn tasks_completed(conn: &Connection, limit: i64) -> Result<Vec<TaskRow>> {
    let mut stmt = conn.prepare(
        "SELECT t.id, t.item_id, t.title, t.notes, t.due_utc, t.priority, t.percent, \
         t.completed_at FROM task t JOIN item i ON i.id = t.item_id \
         WHERE i.deleted_at IS NULL AND t.completed_at IS NOT NULL \
         ORDER BY t.completed_at DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map([limit], row_from)?;
    Ok(rows.flatten().collect())
}

#[derive(Debug, Clone)]
pub struct TaskInput {
    pub id: Option<i64>,
    pub title: String,
    pub notes: Option<String>,
    pub due_utc: Option<String>,
    pub priority: i64,
    pub percent: i64,
}

/// Create or update. Updates snapshot the previous state (S10). Completion
/// is derived from `percent` (100 => `completed_at` set now; below 100 => cleared).
pub fn save_task(conn: &Connection, input: &TaskInput) -> Result<i64> {
    let completed_now = if input.percent >= 100 {
        Some(crate::model::now_iso())
    } else {
        None
    };
    let Some(id) = input.id else {
        let item_id = repo::create_item(conn, Kind::Task, None, false)?;
        conn.execute(
            "INSERT INTO task (item_id, title, notes, due_utc, priority, percent, completed_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                item_id,
                input.title,
                input.notes,
                input.due_utc,
                input.priority,
                input.percent,
                completed_now
            ],
        )?;
        return Ok(conn.last_insert_rowid());
    };
    let existing: i64 = conn
        .query_row("SELECT id FROM task WHERE id = ?1", [id], |r| r.get(0))
        .optional()?
        .ok_or(Error::NotFound(id))?;
    let item_id: i64 =
        conn.query_row("SELECT item_id FROM task WHERE id = ?1", [existing], |r| {
            r.get(0)
        })?;
    let item = repo::get_item(conn, item_id)?;
    repo::snapshot_item(conn, &item)?;
    conn.execute(
        "UPDATE task SET title=?2, notes=?3, due_utc=?4, priority=?5, percent=?6, \
         completed_at=COALESCE(?7, CASE WHEN ?6 >= 100 THEN completed_at ELSE NULL END) \
         WHERE id=?1",
        params![
            existing,
            input.title,
            input.notes,
            input.due_utc,
            input.priority,
            input.percent,
            completed_now
        ],
    )?;
    let _ = repo::touch_item(conn, item_id);
    Ok(existing)
}

pub fn delete_task(conn: &Connection, id: i64) -> Result<()> {
    let item_id: i64 = conn
        .query_row("SELECT item_id FROM task WHERE id = ?1", [id], |r| r.get(0))
        .optional()?
        .ok_or(Error::NotFound(id))?;
    repo::trash_item(conn, item_id)
}

/// Kanban buckets (S38): Todo (<1%), In Progress (1–99), Done (100%).
pub fn kanban(conn: &Connection) -> Result<(Vec<TaskRow>, Vec<TaskRow>, Vec<TaskRow>)> {
    let all = tasks_open(conn, 500)?;
    let mut todo = Vec::new();
    let mut doing = Vec::new();
    for t in all {
        if t.percent <= 0 {
            todo.push(t);
        } else {
            doing.push(t);
        }
    }
    Ok((todo, doing, tasks_completed(conn, 100)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    fn store() -> Store {
        Store::open_memory("x").unwrap()
    }

    #[test]
    fn save_complete_and_ordering() {
        let s = store();
        let conn = s.conn();
        let a = save_task(
            conn,
            &TaskInput {
                id: None,
                title: "Write report".into(),
                notes: None,
                due_utc: Some("2026-11-01T17:00:00+00:00".into()),
                priority: 2,
                percent: 0,
            },
        )
        .unwrap();
        save_task(
            conn,
            &TaskInput {
                id: None,
                title: "No due date".into(),
                notes: None,
                due_utc: None,
                priority: 9,
                percent: 0,
            },
        )
        .unwrap();

        let open = tasks_open(conn, 10).unwrap();
        assert_eq!(open.len(), 2);
        assert_eq!(open[0].title, "Write report", "dated tasks come first");
        assert!(open[1].due_utc.is_none());

        // Completing moves it to the completed list.
        save_task(
            conn,
            &TaskInput {
                id: Some(a),
                title: "Write report".into(),
                notes: None,
                due_utc: None,
                priority: 2,
                percent: 100,
            },
        )
        .unwrap();
        assert_eq!(tasks_open(conn, 10).unwrap().len(), 1);
        let done = tasks_completed(conn, 10).unwrap();
        assert_eq!(done.len(), 1);
        assert!(done[0].completed_at.is_some());

        // Un-completing clears completed_at and snapshots the change.
        save_task(
            conn,
            &TaskInput {
                id: Some(a),
                title: "Write report".into(),
                notes: None,
                due_utc: None,
                priority: 2,
                percent: 50,
            },
        )
        .unwrap();
        assert_eq!(tasks_completed(conn, 10).unwrap().len(), 0);
        let versions: i64 = conn
            .query_row(
                "SELECT count(*) FROM item_version v JOIN task t ON t.item_id = v.item_id WHERE t.id = ?1",
                [a],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(versions, 2, "both edits snapshotted");
    }

    #[test]
    fn kanban_buckets_and_trash() {
        let s = store();
        let conn = s.conn();
        save_task(
            conn,
            &TaskInput {
                id: None,
                title: "todo".into(),
                notes: None,
                due_utc: None,
                priority: 0,
                percent: 0,
            },
        )
        .unwrap();
        let doing = save_task(
            conn,
            &TaskInput {
                id: None,
                title: "doing".into(),
                notes: None,
                due_utc: None,
                priority: 0,
                percent: 40,
            },
        )
        .unwrap();
        save_task(
            conn,
            &TaskInput {
                id: None,
                title: "done".into(),
                notes: None,
                due_utc: None,
                priority: 0,
                percent: 100,
            },
        )
        .unwrap();

        let (todo, in_progress, done) = kanban(conn).unwrap();
        assert_eq!((todo.len(), in_progress.len(), done.len()), (1, 1, 1));

        delete_task(conn, doing).unwrap();
        let (todo, in_progress, done) = kanban(conn).unwrap();
        assert_eq!((todo.len(), in_progress.len(), done.len()), (1, 0, 1));
        assert!(tasks_open(conn, 10)
            .unwrap()
            .iter()
            .all(|t| t.title != "doing"));
    }
}

// ------------------------------------------------------------------- tree

/// A task row plus its depth in the `item.parent_id` hierarchy.
#[derive(Debug, Clone, Serialize)]
pub struct TaskTreeRow {
    #[serde(flatten)]
    pub task: TaskRow,
    pub depth: i64,
    pub parent_id: Option<i64>,
}

/// Depth-ordered task tree. Roots (parent NULL) keep the open-tasks due
/// ordering; children follow their parents in the same order. `depth < 32`
/// bounds the recursion; a deleted or filtered-out ancestor prunes its
/// subtree from the output.
struct Node {
    row: TaskTreeRow,
    due_key: (i64, Option<String>),
    id: i64,
}

pub fn tasks_tree(
    conn: &Connection,
    include_completed: bool,
    limit: i64,
) -> Result<Vec<TaskTreeRow>> {
    let mut stmt = conn.prepare(
        "WITH RECURSIVE tree(item_id, depth) AS (            SELECT i.id, 0 FROM item i JOIN task t ON t.item_id = i.id            WHERE i.deleted_at IS NULL AND i.parent_id IS NULL AND (?1 OR t.completed_at IS NULL)            UNION ALL            SELECT i.id, tree.depth + 1 FROM item i JOIN tree ON i.parent_id = tree.item_id            JOIN task t ON t.item_id = i.id            WHERE tree.depth < 32 AND i.deleted_at IS NULL AND (?1 OR t.completed_at IS NULL)          )          SELECT t.id, t.item_id, t.title, t.notes, t.due_utc, t.priority, t.percent,                 t.completed_at, tree.depth, i.parent_id,                 (t.due_utc IS NULL), t.due_utc, t.id          FROM tree JOIN item i ON i.id = tree.item_id          JOIN task t ON t.item_id = i.id",
    )?;
    let mut nodes: Vec<Node> = stmt
        .query_map(params![include_completed], |r| {
            Ok(Node {
                row: TaskTreeRow {
                    task: row_from(r)?,
                    depth: r.get(8)?,
                    parent_id: r.get(9)?,
                },
                due_key: (r.get(10)?, r.get(11)?),
                id: r.get(12)?,
            })
        })?
        .flatten()
        .collect();

    // Roots keep the open-tasks order; siblings are sorted the same way.
    let open_order = |a: &Node, b: &Node| {
        a.due_key
            .cmp(&b.due_key)
            .then(b.row.task.priority.cmp(&a.row.task.priority))
            .then(a.id.cmp(&b.id))
    };
    nodes.sort_by(open_order);

    // DFS from roots over the original parent links, emitting depth-ordered.
    let mut children: std::collections::HashMap<i64, Vec<usize>> = std::collections::HashMap::new();
    let mut roots: Vec<usize> = Vec::new();
    for (idx, n) in nodes.iter().enumerate() {
        match n.row.parent_id {
            Some(p) => children.entry(p).or_default().push(idx),
            None => roots.push(idx),
        }
    }
    for v in children.values_mut() {
        v.sort_by(|&a, &b| open_order(&nodes[a], &nodes[b]));
    }
    let mut out = Vec::with_capacity(nodes.len());
    let mut stack: Vec<(usize, i64)> = roots.into_iter().rev().map(|i| (i, 0)).collect();
    while let Some((idx, depth)) = stack.pop() {
        let node = &nodes[idx];
        out.push(TaskTreeRow {
            task: node.row.task.clone(),
            depth,
            parent_id: node.row.parent_id,
        });
        if let Some(kids) = children.get(&node.row.task.item_id) {
            for &k in kids.iter().rev() {
                stack.push((k, depth + 1));
            }
        }
    }
    out.truncate(limit as usize);
    Ok(out)
}

/// Reparent a task's item (tree view drag target / indent action).
pub fn set_task_parent(conn: &Connection, task_id: i64, parent_task_id: Option<i64>) -> Result<()> {
    let item_id: i64 = conn
        .query_row("SELECT item_id FROM task WHERE id = ?1", [task_id], |r| {
            r.get(0)
        })
        .optional()?
        .ok_or(Error::NotFound(task_id))?;
    let parent_item = match parent_task_id {
        Some(pid) => {
            // Refuse: self-parenting and cycles (walk up from the new parent).
            let (my_item, pid_item): (i64, i64) = conn.query_row(
                "SELECT (SELECT item_id FROM task WHERE id = ?1), (SELECT item_id FROM task WHERE id = ?2)",
                params![task_id, pid],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            if my_item == pid_item {
                return Err(Error::Parse {
                    what: "task tree",
                    message: String::from("cannot parent a task to itself"),
                });
            }
            let mut cursor = Some(pid_item);
            let mut guard = 0;
            while let Some(cur) = cursor {
                if cur == my_item {
                    return Err(Error::Parse {
                        what: "task tree",
                        message: String::from("cannot create a task cycle"),
                    });
                }
                cursor = conn
                    .query_row("SELECT parent_id FROM item WHERE id = ?1", [cur], |r| {
                        r.get::<_, Option<i64>>(0)
                    })
                    .ok()
                    .unwrap_or(None);
                guard += 1;
                if guard > 32 {
                    break;
                }
            }
            Some(pid_item)
        }
        None => None,
    };
    conn.execute(
        "UPDATE item SET parent_id = ?2 WHERE id = ?1",
        params![item_id, parent_item],
    )?;
    repo::audit(
        conn,
        Some(item_id),
        "task_reparent",
        &serde_json::json!({ "parent": parent_item }),
    )?;
    Ok(())
}

#[cfg(test)]
mod tree_tests {
    use super::*;
    use crate::store::Store;

    #[test]
    fn tree_orders_by_depth_and_detects_cycles() {
        let s = Store::open_memory("x").unwrap();
        let conn = s.conn();
        let a = save_task(
            conn,
            &TaskInput {
                id: None,
                title: "root".into(),
                notes: None,
                due_utc: None,
                priority: 1,
                percent: 0,
            },
        )
        .unwrap();
        let b = save_task(
            conn,
            &TaskInput {
                id: None,
                title: "child".into(),
                notes: None,
                due_utc: None,
                priority: 1,
                percent: 0,
            },
        )
        .unwrap();
        let c = save_task(
            conn,
            &TaskInput {
                id: None,
                title: "grandchild".into(),
                notes: None,
                due_utc: None,
                priority: 1,
                percent: 0,
            },
        )
        .unwrap();
        set_task_parent(conn, b, Some(a)).unwrap();
        set_task_parent(conn, c, Some(b)).unwrap();
        let tree = tasks_tree(conn, false, 100).unwrap();
        let titles: Vec<_> = tree
            .iter()
            .map(|r| (r.task.title.as_str(), r.depth))
            .collect();
        assert_eq!(titles, vec![("root", 0), ("child", 1), ("grandchild", 2)]);
        // Self-parenting refused.
        assert!(set_task_parent(conn, a, Some(a)).is_err());
        // Cycle refused (a under c would close a->b->c->a).
        assert!(set_task_parent(conn, a, Some(c)).is_err());
        // Unparent works: b becomes a root; c stays b's child at depth 1.
        set_task_parent(conn, b, None).unwrap();
        let tree = tasks_tree(conn, false, 100).unwrap();
        let depths: Vec<_> = tree
            .iter()
            .map(|r| (r.task.title.as_str(), r.depth))
            .collect();
        assert_eq!(depths, vec![("root", 0), ("child", 0), ("grandchild", 1)]);
    }
}
