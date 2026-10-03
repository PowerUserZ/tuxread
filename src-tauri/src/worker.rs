//! One thread per opened volume, owning its filesystem (spec §5.5): `Fs` is not `Send`, and
//! opening it (journal replay included) is too slow to repeat for every listing.

use std::collections::HashMap;
use std::sync::mpsc;

use serde::Serialize;
use tuxread_core::fs::{Entry, Fs, Kind, join};
use tuxread_core::probe::Volume;

use crate::display::display_name;
use crate::error::{CmdResult, Code, CommandError};

/// Entries per `ListEvent::Page` (spec §5.5).
pub const PAGE: usize = 2000;

/// Numbers for raw paths, per volume, so the window never handles path bytes (spec §5.5).
/// ponytail: grows with every path listed in a session; fine for browsing, forget paths
/// on a schedule if long sessions show it.
pub struct PathIds {
    paths: Vec<Vec<u8>>,
    ids: HashMap<Vec<u8>, u32>,
}

impl Default for PathIds {
    fn default() -> Self {
        let mut ids = Self {
            paths: Vec::new(),
            ids: HashMap::new(),
        };
        ids.id(b"/");
        ids
    }
}

impl PathIds {
    pub const ROOT: u32 = 0;

    pub fn id(&mut self, path: &[u8]) -> u32 {
        if let Some(&id) = self.ids.get(path) {
            return id;
        }
        let id = self.paths.len() as u32;
        self.paths.push(path.to_vec());
        self.ids.insert(path.to_vec(), id);
        id
    }

    pub fn path(&self, id: u32) -> CmdResult<&[u8]> {
        self.paths
            .get(id as usize)
            .map(Vec::as_slice)
            .ok_or_else(|| CommandError::new(Code::NotFound, format!("no entry #{id}")))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryView {
    pub id: u32,
    pub name: String,
    pub kind: &'static str,
    pub size: u64,
    /// Seconds since 1970 (UTC); absent when the entry could not be read.
    pub mtime: Option<i64>,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Crumb {
    pub id: u32,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "event", content = "data", rename_all = "camelCase")]
pub enum ListEvent {
    /// The folder being listed: its path from the root, and how many entries follow.
    Start {
        crumbs: Vec<Crumb>,
        total: usize,
    },
    Page {
        entries: Vec<EntryView>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Properties {
    pub name: String,
    pub path: String,
    pub kind: &'static str,
    pub size: u64,
    pub mtime: Option<i64>,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub inode: u64,
    pub link: Option<String>,
}

pub fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::File => "file",
        Kind::Dir => "dir",
        Kind::Symlink => "symlink",
        Kind::Other => "other",
    }
}

/// What a worker thread owns.
pub struct Session {
    pub fs: Box<dyn Fs>,
    pub ids: PathIds,
}

impl Session {
    /// Lists folder `dir`, sending a `Start` and then pages of at most `page` entries.
    pub fn list(
        &mut self,
        dir: u32,
        page: usize,
        send: &mut dyn FnMut(ListEvent),
    ) -> CmdResult<usize> {
        let path = self.ids.path(dir)?.to_vec();
        let entries = self.fs.read_dir(&path)?;
        let crumbs = self.crumbs(&path);
        send(ListEvent::Start {
            crumbs,
            total: entries.len(),
        });
        for chunk in entries.chunks(page.max(1)) {
            let entries = chunk.iter().map(|e| self.view(&path, e)).collect();
            send(ListEvent::Page { entries });
        }
        Ok(entries.len())
    }

    fn view(&mut self, dir: &[u8], e: &Entry) -> EntryView {
        EntryView {
            id: self.ids.id(&join(dir, &e.name)),
            name: display_name(&e.name),
            kind: kind_name(e.kind),
            size: e.size,
            mtime: e.mtime.map(|t| t.secs),
            mode: e.mode,
            uid: e.uid,
            gid: e.gid,
        }
    }

    /// The root, then each folder down to `path`.
    fn crumbs(&mut self, path: &[u8]) -> Vec<Crumb> {
        let mut crumbs = vec![Crumb {
            id: PathIds::ROOT,
            name: String::new(),
        }];
        let mut prefix = Vec::new();
        for part in path.split(|&b| b == b'/').filter(|p| !p.is_empty()) {
            prefix = join(&prefix, part);
            crumbs.push(Crumb {
                id: self.ids.id(&prefix),
                name: display_name(part),
            });
        }
        crumbs
    }

    pub fn properties(&mut self, entry: u32) -> CmdResult<Properties> {
        let path = self.ids.path(entry)?.to_vec();
        let e = self.fs.stat(&path)?;
        let link = if e.kind == Kind::Symlink {
            self.fs.read_link(&path).ok().map(|t| display_name(&t))
        } else {
            None
        };
        Ok(Properties {
            name: display_name(&e.name),
            path: display_name(&path),
            kind: kind_name(e.kind),
            size: e.size,
            mtime: e.mtime.map(|t| t.secs),
            mode: e.mode,
            uid: e.uid,
            gid: e.gid,
            inode: e.ino,
            link,
        })
    }

    /// The raw paths of `entries`, for a copy job.
    pub fn paths(&self, entries: &[u32]) -> CmdResult<Vec<Vec<u8>>> {
        entries
            .iter()
            .map(|&id| self.ids.path(id).map(<[u8]>::to_vec))
            .collect()
    }
}

type Call = Box<dyn FnOnce(&mut Session) + Send>;

/// Handle to a volume's thread; the thread ends when the last handle is dropped.
pub struct Worker {
    tx: mpsc::Sender<Call>,
}

impl Worker {
    /// Opens `volume`'s filesystem on a new thread, which keeps it for later calls.
    pub fn start(volume: Volume) -> CmdResult<Self> {
        let (tx, rx) = mpsc::channel::<Call>();
        let (ready_tx, ready_rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("volume".into())
            .spawn(move || {
                let fs = match volume.open() {
                    Ok(fs) => fs,
                    Err(e) => {
                        let _ = ready_tx.send(Err(CommandError::from(e)));
                        return;
                    }
                };
                let _ = ready_tx.send(Ok(()));
                let mut session = Session {
                    fs,
                    ids: PathIds::default(),
                };
                for call in rx {
                    call(&mut session);
                }
            })?;
        ready_rx.recv().map_err(|_| stopped())??;
        Ok(Self { tx })
    }

    /// Runs `f` on the volume's thread and waits for its result.
    pub fn call<R: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Session) -> R + Send + 'static,
    ) -> CmdResult<R> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.tx
            .send(Box::new(move |session| {
                let _ = reply_tx.send(f(session));
            }))
            .map_err(|_| stopped())?;
        reply_rx.recv().map_err(|_| stopped())
    }
}

fn stopped() -> CommandError {
    CommandError::new(Code::Other, "the volume's thread stopped")
}

#[cfg(test)]
pub(crate) mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use tuxread_core::cache::CachedDev;
    use tuxread_core::dev::FileDev;
    use tuxread_core::probe::{self, NodeKind};

    use super::*;

    pub fn tiny_image() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../crates/core/tests/data/tiny-ext4.img")
    }

    fn tiny_volume() -> Volume {
        let dev = Arc::new(CachedDev::new(Arc::new(
            FileDev::open(&tiny_image()).unwrap(),
        )));
        let nodes = probe::probe(dev);
        match &probe::leaves(&nodes)[0].kind {
            NodeKind::Volume(v) => v.clone(),
            _ => panic!("tiny image has no volume"),
        }
    }

    fn list(worker: &Worker, dir: u32, page: usize) -> Vec<ListEvent> {
        worker
            .call(move |s| {
                let mut events = Vec::new();
                s.list(dir, page, &mut |e| events.push(e)).map(|_| events)
            })
            .unwrap()
            .unwrap()
    }

    #[test]
    fn listings_come_in_pages_after_a_start() {
        let worker = Worker::start(tiny_volume()).unwrap();
        let events = list(&worker, PathIds::ROOT, 3);
        let [
            ListEvent::Start { crumbs, total },
            ListEvent::Page { entries: first },
            ListEvent::Page { entries: second },
        ] = events.as_slice()
        else {
            panic!("unexpected events: {events:?}");
        };
        assert_eq!(*total, 4);
        assert_eq!(
            crumbs,
            &[Crumb {
                id: 0,
                name: String::new()
            }]
        );
        assert_eq!((first.len(), second.len()), (3, 1));
        let mut names: Vec<_> = first
            .iter()
            .chain(second)
            .map(|e| e.name.as_str())
            .collect();
        names.sort();
        assert_eq!(names, ["a.txt", "dir", "link", "lost+found"]);
    }

    #[test]
    fn entries_are_numbered_and_folders_can_be_entered_by_number() {
        let worker = Worker::start(tiny_volume()).unwrap();
        let root = list(&worker, PathIds::ROOT, PAGE);
        let ListEvent::Page { entries } = &root[1] else {
            panic!()
        };
        let dir = entries.iter().find(|e| e.name == "dir").unwrap();
        assert_eq!(dir.kind, "dir");
        let inside = list(&worker, dir.id, PAGE);
        let ListEvent::Start { crumbs, total } = &inside[0] else {
            panic!()
        };
        assert_eq!(*total, 1);
        assert_eq!(
            crumbs.last().unwrap(),
            &Crumb {
                id: dir.id,
                name: "dir".into()
            }
        );
        let ListEvent::Page { entries } = &inside[1] else {
            panic!()
        };
        assert_eq!(entries[0].name, "b.txt");
        assert_eq!(entries[0].size, 5);
        // The same path keeps its number.
        let again = list(&worker, PathIds::ROOT, PAGE);
        let ListEvent::Page { entries: again } = &again[1] else {
            panic!()
        };
        assert_eq!(again.iter().find(|e| e.name == "dir").unwrap().id, dir.id);
    }

    #[test]
    fn properties_show_link_targets_and_unknown_numbers_are_errors() {
        let worker = Worker::start(tiny_volume()).unwrap();
        let root = list(&worker, PathIds::ROOT, PAGE);
        let ListEvent::Page { entries } = &root[1] else {
            panic!()
        };
        let link = entries.iter().find(|e| e.name == "link").unwrap().id;
        let props = worker.call(move |s| s.properties(link)).unwrap().unwrap();
        assert_eq!((props.kind, props.path.as_str()), ("symlink", "/link"));
        assert_eq!(props.link.as_deref(), Some("a.txt"));
        assert!(props.inode > 0);
        let missing = worker.call(|s| s.properties(9999)).unwrap().unwrap_err();
        assert_eq!(missing.code, Code::NotFound);
        let file = entries.iter().find(|e| e.name == "a.txt").unwrap().id;
        let not_a_dir = list_err(&worker, file);
        assert_eq!(not_a_dir.code, Code::NotADirectory);
    }

    fn list_err(worker: &Worker, dir: u32) -> CommandError {
        worker
            .call(move |s| s.list(dir, PAGE, &mut |_| {}))
            .unwrap()
            .unwrap_err()
    }

    #[test]
    fn a_volume_that_cannot_be_opened_is_an_error_not_a_thread() {
        let mut volume = tiny_volume();
        volume.dev = Arc::new(tuxread_core::dev::MemDev(vec![0u8; 4096]));
        let error = Worker::start(volume).err().unwrap();
        assert_ne!(error.code, Code::Other, "{error:?}");
    }
}
