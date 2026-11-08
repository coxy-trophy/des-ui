//! The file manager's work queue: operations wait in order and run one at a time;
//! a name already taken stops the queue until someone chooses; and every finished
//! batch remembers how to put things back, for undo.

use crate::file_ops::{Completed, Conflict, Operation};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// One batch's way back, newest last.
struct Undo { batch: u64, label: String, steps: Vec<Operation> }

/// What the next turn of the queue is.
#[derive(Debug, PartialEq)]
pub enum Next { Start(u64, Operation), Waiting, Idle }

/// Two items with one name, for someone to choose between.
#[derive(Clone, Debug, PartialEq)]
pub struct Clash {
    pub name: String,
    pub incoming: (u64, Option<SystemTime>),
    pub existing: (u64, Option<SystemTime>),
    /// More copies or moves of the same batch still to come.
    pub more: usize,
}

/// What the file manager shows of the queue.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct View {
    pub running: Option<String>,
    pub progress: u64,
    pub waiting: Vec<String>,
    pub paused: bool,
    /// Finished work, newest first, and whether it went well.
    pub recent: Vec<(String, bool)>,
    pub undo: Option<String>,
    pub clash: Option<Clash>,
}

#[derive(Default)]
pub struct Transfers {
    waiting: VecDeque<(u64, Operation)>,
    next_batch: u64,
    /// The running operation's batch and what it says it is doing.
    running: Option<(u64, String)>,
    paused: bool,
    /// An operation held back by a name already taken.
    clash: Option<(u64, Operation)>,
    /// A choice someone made for the rest of a batch.
    policy: HashMap<u64, Conflict>,
    /// Batches that are themselves an undo, which are not undone again.
    undoing: Vec<u64>,
    undo: Vec<Undo>,
    labels: HashMap<u64, String>,
    recent: VecDeque<(String, bool)>,
}

fn name(path: &Path) -> String { path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string()) }

/// One operation, as a person would say it.
pub fn describe(operation: &Operation) -> String {
    match operation {
        Operation::CreateFolder { name, .. } => format!("Create folder {name}"),
        Operation::Rename { source, name: new } => format!("Rename {} to {new}", name(source)),
        Operation::Copy { source, directory, .. } => format!("Copy {} to {}", name(source), name(directory)),
        Operation::Move { source, directory, .. } => format!("Move {} to {}", name(source), name(directory)),
        Operation::Trash { source } => format!("Move {} to Trash", name(source)),
        Operation::Compress { sources, archive } => match sources.as_slice() {
            [one] => format!("Compress {} into {}", name(one), name(archive)),
            many => format!("Compress {} items into {}", many.len(), name(archive)),
        },
        Operation::Extract { archive } => format!("Extract {}", name(archive)),
        Operation::Restore { source } => format!("Restore {}", crate::file_ops::trash_original_name(source).map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| name(source))),
    }
}

/// A batch of operations, in a few words.
fn describe_batch(operations: &[Operation]) -> String {
    match operations {
        [one] => describe(one),
        [first, ..] => {
            let count = operations.len();
            match first {
                Operation::Copy { directory, .. } => format!("Copy {count} items to {}", name(directory)),
                Operation::Move { directory, .. } => format!("Move {count} items to {}", name(directory)),
                Operation::Trash { .. } => format!("Move {count} items to Trash"),
                Operation::Restore { .. } => format!("Restore {count} items"),
                _ => format!("{count} changes"),
            }
        },
        [] => String::new(),
    }
}

fn stat(path: &Path) -> (u64, Option<SystemTime>) {
    std::fs::metadata(path).map(|m| (m.len(), m.modified().ok())).unwrap_or((0, None))
}

impl Transfers {
    /// Adds a batch to the end of the queue; returns its number.
    pub fn enqueue(&mut self, operations: Vec<Operation>) -> u64 {
        self.next_batch += 1;
        let batch = self.next_batch;
        self.labels.insert(batch, describe_batch(&operations));
        self.waiting.extend(operations.into_iter().map(|operation| (batch, operation)));
        batch
    }

    /// The next operation to start, when nothing runs, the queue is not paused,
    /// and no choice is waiting. A copy into its own folder makes a second copy; a
    /// move into its own folder does nothing; a name taken elsewhere asks.
    pub fn next(&mut self) -> Next {
        if self.running.is_some() || self.clash.is_some() { return Next::Waiting; }
        if self.paused { return if self.waiting.is_empty() { Next::Idle } else { Next::Waiting }; }
        let Some((batch, mut operation)) = self.waiting.pop_front() else { return Next::Idle };
        if let Operation::Copy { conflict, .. } | Operation::Move { conflict, .. } = &mut operation {
            if *conflict == Conflict::Ask { if let Some(choice) = self.policy.get(&batch) { *conflict = *choice; } }
        }
        let target = crate::file_ops::target_of(&operation);
        let (source, conflict) = match &operation {
            Operation::Copy { source, conflict, .. } | Operation::Move { source, conflict, .. } => (Some(source.clone()), *conflict),
            _ => (None, Conflict::Ask),
        };
        if let (Some(source), Some(target)) = (source, target) {
            let same = std::fs::canonicalize(&source).ok().is_some_and(|s| std::fs::canonicalize(&target).ok() == Some(s));
            if same {
                match &mut operation {
                    Operation::Copy { conflict, .. } => *conflict = Conflict::KeepBoth,
                    _ => { self.recent_push(describe(&operation), true); return self.next(); },
                }
            } else if conflict == Conflict::Ask && std::fs::symlink_metadata(&target).is_ok() {
                self.clash = Some((batch, operation));
                return Next::Waiting;
            }
        }
        self.running = Some((batch, describe(&operation)));
        Next::Start(batch, operation)
    }

    /// Answers the waiting question; `all` answers it for the rest of the batch too.
    pub fn resolve(&mut self, choice: Conflict, all: bool) {
        let Some((batch, mut operation)) = self.clash.take() else { return };
        if all { self.policy.insert(batch, choice); }
        if let Operation::Copy { conflict, .. } | Operation::Move { conflict, .. } = &mut operation { *conflict = choice; }
        self.waiting.push_front((batch, operation));
    }

    /// Records how the running operation ended; returns what to tell the person
    /// when its batch is over.
    pub fn finished(&mut self, batch: u64, operation: &Operation, result: &Result<Completed, String>) -> Option<String> {
        self.running = None;
        let label = describe(operation);
        match result {
            Ok(completed) => {
                self.recent_push(if completed.skipped { format!("Skipped {}", name(&operation_source(operation))) } else { label }, true);
                if !self.undoing.contains(&batch) {
                    let steps = crate::file_ops::reverse(operation, completed);
                    if !steps.is_empty() {
                        match self.undo.last_mut().filter(|undo| undo.batch == batch) {
                            Some(undo) => undo.steps.splice(0..0, steps).for_each(drop),
                            None => self.undo.push(Undo { batch, label: self.labels.get(&batch).cloned().unwrap_or_default(), steps }),
                        }
                        if self.undo.len() > 20 { self.undo.remove(0); }
                    }
                }
            },
            Err(error) => {
                self.recent_push(format!("{label}: {error}"), false);
                // The rest of a failed batch waits for nobody.
                self.waiting.retain(|(waiting, _)| *waiting != batch);
            },
        }
        let over = !self.waiting.iter().any(|(waiting, _)| *waiting == batch) && self.clash.as_ref().is_none_or(|(waiting, _)| *waiting != batch);
        over.then(|| { self.policy.remove(&batch); self.labels.remove(&batch).unwrap_or_default() })
    }

    /// Queues the way back for the newest batch that has one.
    pub fn undo(&mut self) -> Option<String> {
        let undo = self.undo.pop()?;
        let batch = self.enqueue(undo.steps);
        self.undoing.push(batch);
        self.labels.insert(batch, format!("Undo: {}", undo.label));
        Some(undo.label)
    }

    pub fn set_paused(&mut self, paused: bool) { self.paused = paused; }
    /// Drops everything still waiting, and a waiting question with it.
    pub fn clear(&mut self) -> usize {
        let count = self.waiting.len() + usize::from(self.clash.is_some());
        self.waiting.clear(); self.clash = None; self.policy.clear();
        count
    }

    fn recent_push(&mut self, text: String, ok: bool) {
        self.recent.push_front((text, ok));
        self.recent.truncate(6);
    }

    pub fn view(&self, progress: u64) -> View {
        let clash = self.clash.as_ref().map(|(batch, operation)| {
            let source = operation_source(operation);
            let target = crate::file_ops::target_of(operation).unwrap_or_default();
            Clash { name: name(&source), incoming: stat(&source), existing: stat(&target),
                more: self.waiting.iter().filter(|(waiting, op)| waiting == batch && matches!(op, Operation::Copy { .. } | Operation::Move { .. })).count() }
        });
        View {
            running: self.running.as_ref().map(|(_, label)| label.clone()),
            progress,
            waiting: self.waiting.iter().map(|(_, operation)| describe(operation)).collect(),
            paused: self.paused,
            recent: self.recent.iter().cloned().collect(),
            undo: self.undo.last().map(|undo| undo.label.clone()),
            clash,
        }
    }
}

fn operation_source(operation: &Operation) -> PathBuf {
    match operation {
        Operation::CreateFolder { parent, name } => parent.join(name),
        Operation::Rename { source, .. } | Operation::Copy { source, .. } | Operation::Move { source, .. } | Operation::Trash { source } | Operation::Restore { source } => source.clone(),
        Operation::Compress { archive, .. } | Operation::Extract { archive } => archive.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ok(path: PathBuf) -> Result<Completed, String> { Ok(Completed { path, message: String::new(), replaced: None, skipped: false }) }

    #[test]
    fn a_taken_name_holds_the_queue_until_someone_chooses_for_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let (from, to) = (dir.path().join("from"), dir.path().join("to"));
        std::fs::create_dir_all(&from).unwrap(); std::fs::create_dir_all(&to).unwrap();
        for n in ["a", "b", "c"] { std::fs::write(from.join(n), b"x").unwrap(); }
        for n in ["a", "b"] { std::fs::write(to.join(n), b"old").unwrap(); }
        let mut queue = Transfers::default();
        let copy = |n: &str| Operation::Copy { source: from.join(n), directory: to.clone(), conflict: Conflict::Ask };
        queue.enqueue(vec![copy("a"), copy("b"), copy("c")]);
        assert_eq!(queue.next(), Next::Waiting);
        let view = queue.view(0);
        assert_eq!(view.clash.as_ref().map(|c| (c.name.as_str(), c.more)), Some(("a", 2)));
        queue.resolve(Conflict::KeepBoth, true);
        let Next::Start(batch, first) = queue.next() else { panic!("the choice lets it run") };
        assert!(matches!(first, Operation::Copy { conflict: Conflict::KeepBoth, .. }));
        assert_eq!(queue.next(), Next::Waiting, "one at a time");
        assert!(queue.finished(batch, &first, &ok(to.join("a (2)"))).is_none(), "the batch is not over");
        let Next::Start(_, second) = queue.next() else { panic!() };
        assert!(matches!(second, Operation::Copy { conflict: Conflict::KeepBoth, .. }), "the same answer for the rest");
    }

    #[test]
    fn a_copy_into_its_own_folder_makes_a_second_copy_and_a_failure_drops_its_batch() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a"), b"x").unwrap();
        let mut queue = Transfers::default();
        queue.enqueue(vec![Operation::Copy { source: dir.path().join("a"), directory: dir.path().into(), conflict: Conflict::Ask },
                           Operation::Trash { source: dir.path().join("a") }]);
        let Next::Start(batch, copy) = queue.next() else { panic!() };
        assert!(matches!(copy, Operation::Copy { conflict: Conflict::KeepBoth, .. }));
        let done = queue.finished(batch, &copy, &Err("disk full".into()));
        assert!(done.is_some(), "a failure ends its batch");
        assert_eq!(queue.next(), Next::Idle, "the rest of a failed batch is dropped");
        assert!(!queue.view(0).recent[0].1);
    }

    #[test]
    fn undo_reverses_a_whole_batch_newest_first_and_is_not_undone_again() {
        let mut queue = Transfers::default();
        let trash = |n: &str| Operation::Trash { source: PathBuf::from("/home/you").join(n) };
        let batch = queue.enqueue(vec![trash("a"), trash("b")]);
        for (n, op) in [("1", trash("a")), ("2", trash("b"))] {
            let Next::Start(_, started) = queue.next() else { panic!() };
            assert_eq!(started, op);
            queue.finished(batch, &started, &ok(PathBuf::from("/trash/files").join(n)));
        }
        assert_eq!(queue.view(0).undo.as_deref(), Some("Move 2 items to Trash"));
        assert_eq!(queue.undo().as_deref(), Some("Move 2 items to Trash"));
        let Next::Start(back, first) = queue.next() else { panic!() };
        assert_eq!(first, Operation::Restore { source: "/trash/files/2".into() }, "the newest comes back first");
        queue.finished(back, &first, &ok("/home/you/b".into()));
        assert!(queue.view(0).undo.is_none(), "an undo is not itself undone");
    }

    #[test]
    fn pausing_holds_the_queue_and_clearing_drops_it() {
        let mut queue = Transfers::default();
        queue.enqueue(vec![Operation::Trash { source: "/home/you/a".into() }]);
        queue.set_paused(true);
        assert_eq!(queue.next(), Next::Waiting);
        assert_eq!(queue.view(0).waiting.len(), 1);
        assert_eq!(queue.clear(), 1);
        queue.set_paused(false);
        assert_eq!(queue.next(), Next::Idle);
    }
}
