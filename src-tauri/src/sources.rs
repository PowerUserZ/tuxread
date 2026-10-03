//! Disks and images the window shows, what probing found in them, and their volumes.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use serde::Serialize;
use tuxread_core::cache::CachedDev;
use tuxread_core::dev::{BlockDev, FileDev};
use tuxread_core::fs::FsInfo;
use tuxread_core::part::{self, DiskId, TypeCode};
use tuxread_core::probe::{self, Node, NodeKind, Status, TypeFix, Volume};
use tuxread_win::disk::{DiskInfo, WinDisk, list_disks};
use tuxread_win::helper::{self, Helper, HelperDisk};

use crate::display::display_name;
use crate::error::{CmdResult, Code, CommandError};
use crate::worker::Worker;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FsView {
    pub fs_type: String,
    pub label: String,
    pub uuid: String,
    pub size: u64,
    pub used: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeView {
    pub label: String,
    pub size: u64,
    /// "partition", "volume" or "detected".
    pub kind: &'static str,
    /// For volumes: the number list_dir, stat and copy take.
    pub volume: Option<u32>,
    /// For detected content: "windows", "windowsSkips", "later", "unsupported",
    /// "unrecognized" or "error".
    pub status: Option<&'static str>,
    pub detail: Option<String>,
    /// For "windowsSkips": what makes Windows mount the partition.
    pub fix: Option<FixView>,
    pub fs: Option<FsView>,
    pub children: Vec<NodeView>,
}

/// A partition Windows skips, as PowerShell's storage commands name it: the window turns this
/// into the commands it shows (src/windowsFix.ts).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FixView {
    /// "gpt" or "mbr".
    pub table: &'static str,
    /// `Get-Disk`'s Guid ("{…}") for GPT, or its Signature (decimal) for MBR.
    pub disk: String,
    /// `Get-Partition`'s Offset, in bytes.
    pub offset: u64,
    /// The partition's GptType ("{…}") or MbrType (decimal) now, and the one Windows mounts.
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceView {
    /// "disk" or "image".
    pub kind: &'static str,
    /// The disk number, or the image's number in this session.
    pub key: u32,
    pub name: String,
    pub detail: String,
    pub size: u64,
    /// Disks stay locked until opened through the disk helper.
    pub open: bool,
    pub nodes: Vec<NodeView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourcesView {
    pub disks: Vec<SourceView>,
    pub images: Vec<SourceView>,
}

/// An opened disk or image.
pub struct Opened {
    pub key: u32,
    pub disk: Option<DiskInfo>,
    path: Option<PathBuf>,
    /// Read through the disk helper: unusable once the helper exits.
    via_helper: bool,
    pub size: u64,
    pub nodes: Vec<Node>,
    views: Vec<NodeView>,
}

pub struct VolumeSlot {
    pub volume: Volume,
    /// The disk it lives on, when it is read through the helper.
    pub disk: Option<u32>,
    worker: Mutex<Option<Arc<Worker>>>,
}

impl VolumeSlot {
    /// The volume's thread, started on first use.
    pub fn worker(&self) -> CmdResult<Arc<Worker>> {
        let mut worker = lock(&self.worker);
        if let Some(w) = worker.as_ref() {
            return Ok(Arc::clone(w));
        }
        let w = Arc::new(Worker::start(self.volume.clone())?);
        *worker = Some(Arc::clone(&w));
        Ok(w)
    }
}

#[derive(Default)]
pub struct Registry {
    next_image: u32,
    next_volume: u32,
    pub images: Vec<Opened>,
    pub disks: Vec<Opened>,
    volumes: HashMap<u32, Arc<VolumeSlot>>,
}

/// Everything the commands share.
#[derive(Default)]
pub struct App {
    pub(crate) registry: Mutex<Registry>,
    helper: Mutex<Option<Helper>>,
}

pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl App {
    pub fn list_sources(&self) -> SourcesView {
        self.forget_disks_if_helper_gone();
        let registry = lock(&self.registry);
        let disks = list_disks()
            .into_iter()
            .map(
                |d| match registry.disks.iter().find(|o| o.key == d.number) {
                    Some(opened) => source_view(opened),
                    None => locked_disk_view(&d),
                },
            )
            .collect();
        let images = registry.images.iter().map(source_view).collect();
        SourcesView { disks, images }
    }

    pub fn open_image(&self, path: &Path) -> CmdResult<SourceView> {
        if let Some(open) = lock(&self.registry)
            .images
            .iter()
            .find(|o| o.path.as_deref() == Some(path))
        {
            return Ok(source_view(open));
        }
        let dev: Arc<dyn BlockDev> = Arc::new(FileDev::open(path)?);
        let mut registry = lock(&self.registry);
        registry.next_image += 1;
        let key = registry.next_image;
        let opened = registry.open(key, dev, None, Some(path.to_path_buf()), false);
        let view = source_view(&opened);
        registry.images.push(opened);
        Ok(view)
    }

    /// Opens disk `number`: directly when this process is elevated, otherwise through the
    /// disk helper, started (one UAC prompt, owned by window `owner`) if it is not running.
    pub fn open_disk(&self, number: u32, owner: isize) -> CmdResult<SourceView> {
        self.forget_disks_if_helper_gone();
        if let Some(open) = lock(&self.registry).disks.iter().find(|o| o.key == number) {
            return Ok(source_view(open));
        }
        let info = list_disks()
            .into_iter()
            .find(|d| d.number == number)
            .ok_or_else(|| CommandError::new(Code::NotFound, format!("no disk {number}")))?;
        let (dev, via_helper): (Arc<dyn BlockDev>, bool) = if tuxread_win::is_elevated() {
            (Arc::new(WinDisk::open(number)?), false)
        } else {
            (Arc::new(self.open_through_helper(number, owner)?), true)
        };
        let mut registry = lock(&self.registry);
        let opened = registry.open(number, dev, Some(info), None, via_helper);
        let view = source_view(&opened);
        registry.disks.push(opened);
        Ok(view)
    }

    /// Disk `number` read through the helper, which is started first if it is not running.
    fn open_through_helper(&self, number: u32, owner: isize) -> CmdResult<HelperDisk> {
        let mut slot = lock(&self.helper);
        let helper = match slot.take() {
            Some(running) if running.is_running() => running,
            _ => helper::launch(&std::env::current_exe()?, &[], true, owner)?,
        };
        let disk = helper.open_disk(number);
        *slot = Some(helper);
        Ok(disk?)
    }

    pub fn helper_running(&self) -> bool {
        lock(&self.helper).as_ref().is_some_and(Helper::is_running)
    }

    /// Disks read through a helper that has exited cannot be read again: show them locked.
    fn forget_disks_if_helper_gone(&self) {
        if self.helper_running() {
            return;
        }
        let mut registry = lock(&self.registry);
        registry.disks.retain(|o| !o.via_helper);
        registry.volumes.retain(|_, slot| slot.disk.is_none());
    }

    pub fn volume(&self, id: u32) -> CmdResult<Arc<VolumeSlot>> {
        let slot = lock(&self.registry).volumes.get(&id).cloned();
        slot.ok_or_else(|| CommandError::new(Code::NotFound, format!("no volume #{id}")))
    }

    /// `error`, or "helper gone" when it happened on a disk whose helper has exited.
    pub fn explain(&self, slot: &VolumeSlot, error: CommandError) -> CommandError {
        if slot.disk.is_some() && !self.helper_running() {
            CommandError::new(Code::HelperGone, error.message)
        } else {
            error
        }
    }
}

impl Registry {
    /// Probes `dev` and numbers the volumes found.
    fn open(
        &mut self,
        key: u32,
        dev: Arc<dyn BlockDev>,
        disk: Option<DiskInfo>,
        path: Option<PathBuf>,
        via_helper: bool,
    ) -> Opened {
        let size = dev.len();
        let nodes = probe::probe(Arc::new(CachedDev::new(dev)));
        let helper_disk = via_helper.then_some(key);
        let views = nodes.iter().map(|n| self.view(n, helper_disk)).collect();
        Opened {
            key,
            disk,
            path,
            via_helper,
            size,
            nodes,
            views,
        }
    }

    fn view(&mut self, node: &Node, helper_disk: Option<u32>) -> NodeView {
        let children = node
            .children
            .iter()
            .map(|c| self.view(c, helper_disk))
            .collect();
        let mut view = NodeView {
            label: display_name(node.label.as_bytes()),
            size: node.size,
            kind: "partition",
            volume: None,
            status: None,
            detail: None,
            fix: None,
            fs: None,
            children,
        };
        match &node.kind {
            NodeKind::Partition { .. } => {}
            NodeKind::Volume(volume) => {
                self.next_volume += 1;
                let id = self.next_volume;
                self.volumes.insert(
                    id,
                    Arc::new(VolumeSlot {
                        volume: volume.clone(),
                        disk: helper_disk,
                        worker: Mutex::new(None),
                    }),
                );
                view.kind = "volume";
                view.volume = Some(id);
                view.fs = Some(fs_view(&volume.info));
            }
            NodeKind::Detected { status, .. } => {
                view.kind = "detected";
                let (name, detail) = match status {
                    Status::WindowsCanOpen => ("windows", None),
                    Status::WindowsSkips(fix) => {
                        view.fix = Some(fix_view(fix));
                        ("windowsSkips", Some(display_name(fix.type_name.as_bytes())))
                    }
                    Status::Later => ("later", None),
                    Status::NotSupported(why) => ("unsupported", Some(why.clone())),
                    Status::Unrecognized => ("unrecognized", None),
                    Status::Error(e) => ("error", Some(e.clone())),
                };
                view.status = Some(name);
                view.detail = detail;
            }
        }
        view
    }
}

fn fix_view(fix: &TypeFix) -> FixView {
    let guid = |g: &[u8; 16]| format!("{{{}}}", part::guid_string(g).to_lowercase());
    let code = |c: &TypeCode| match c {
        TypeCode::Gpt(g) => guid(g),
        TypeCode::Mbr(b) => b.to_string(),
    };
    let (table, disk) = match fix.disk {
        DiskId::Gpt(g) => ("gpt", guid(&g)),
        DiskId::Mbr(signature) => ("mbr", signature.to_string()),
    };
    FixView {
        table,
        disk,
        offset: fix.offset,
        from: code(&fix.from),
        to: code(&fix.to),
    }
}

fn fs_view(info: &FsInfo) -> FsView {
    FsView {
        fs_type: info.fs_type.clone(),
        label: display_name(info.label.as_bytes()),
        uuid: info.uuid.clone(),
        size: info.size,
        used: info.used,
    }
}

fn disk_detail(d: &DiskInfo) -> String {
    format!("{}, {}-byte sectors", d.bus, d.logical_sector)
}

fn locked_disk_view(d: &DiskInfo) -> SourceView {
    SourceView {
        kind: "disk",
        key: d.number,
        name: display_name(d.model.as_bytes()),
        detail: disk_detail(d),
        size: d.size,
        open: false,
        nodes: Vec::new(),
    }
}

fn source_view(o: &Opened) -> SourceView {
    match (&o.disk, &o.path) {
        (Some(d), _) => SourceView {
            open: true,
            nodes: o.views.clone(),
            ..locked_disk_view(d)
        },
        (None, path) => {
            let path = path.as_deref().unwrap_or(Path::new(""));
            let name = path.file_name().unwrap_or(path.as_os_str());
            SourceView {
                kind: "image",
                key: o.key,
                name: display_name(name.to_string_lossy().as_bytes()),
                detail: display_name(path.to_string_lossy().as_bytes()),
                size: o.size,
                open: true,
                nodes: o.views.clone(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker::tests::tiny_image;

    #[test]
    fn an_opened_image_shows_its_volume_and_is_opened_once() {
        let app = App::default();
        let view = app.open_image(&tiny_image()).unwrap();
        assert_eq!(
            (view.kind, view.name.as_str(), view.open),
            ("image", "tiny-ext4.img", true)
        );
        assert_eq!(view.size, 524_288);
        let [node] = view.nodes.as_slice() else {
            panic!("{view:?}")
        };
        assert_eq!(node.kind, "volume");
        assert_eq!(node.fs.as_ref().unwrap().fs_type, "ext4");
        let volume = node.volume.unwrap();
        assert!(app.volume(volume).is_ok());
        assert_eq!(app.open_image(&tiny_image()).unwrap(), view);
        assert_eq!(app.list_sources().images, [view]);
    }

    #[test]
    fn disks_are_listed_locked_without_starting_the_helper() {
        let app = App::default();
        let sources = app.list_sources();
        assert!(!sources.disks.is_empty());
        assert!(sources.disks.iter().all(|d| !d.open && d.nodes.is_empty()));
        assert!(!app.helper_running());
    }

    #[test]
    fn missing_images_and_volumes_are_not_found() {
        let app = App::default();
        let missing = app
            .open_image(Path::new(r"C:\no\such\image.img"))
            .unwrap_err();
        assert_eq!(missing.code, Code::NotFound);
        assert_eq!(app.volume(42).err().unwrap().code, Code::NotFound);
    }

    #[test]
    fn disks_whose_helper_is_gone_are_forgotten_and_their_errors_say_so() {
        // No helper runs in this test, as after it has exited. A disk opened through it:
        let app = App::default();
        let dev: Arc<dyn BlockDev> = Arc::new(FileDev::open(&tiny_image()).unwrap());
        let opened = lock(&app.registry).open(7, dev, None, None, true);
        let volume = opened.views[0].volume.unwrap();
        lock(&app.registry).disks.push(opened);
        let slot = app.volume(volume).unwrap();
        let error = CommandError::new(Code::Io, "the pipe has been ended");
        assert_eq!(app.explain(&slot, error.clone()).code, Code::HelperGone);

        // Errors on images keep their own code.
        let image = app.open_image(&tiny_image()).unwrap();
        let on_image = app.volume(image.nodes[0].volume.unwrap()).unwrap();
        assert_eq!(app.explain(&on_image, error).code, Code::Io);

        // The next look at the sources forgets the disk and its volume; the image stays.
        let sources = app.list_sources();
        assert!(sources.disks.iter().all(|d| !d.open));
        assert!(lock(&app.registry).disks.is_empty());
        assert_eq!(app.volume(volume).err().unwrap().code, Code::NotFound);
        assert!(app.volume(image.nodes[0].volume.unwrap()).is_ok());
    }

    /// The window builds PowerShell commands from this, so it is in Get-Disk's and
    /// Get-Partition's own forms. The GUID is the USB disk this was found on.
    #[test]
    fn a_partition_windows_skips_is_named_as_powershell_names_it() {
        let usb = [
            0x6E, 0x9C, 0xAE, 0x5E, 0xA2, 0xB1, 0xE1, 0x4B, 0xB3, 0x3C, 0xD4, 0x06, 0xEA, 0xB7,
            0x5D, 0xC1,
        ];
        let linux = [
            0xAF, 0x3D, 0xC6, 0x0F, 0x83, 0x84, 0x72, 0x47, 0x8E, 0x79, 0x3D, 0x69, 0xD8, 0x47,
            0x7D, 0xE4,
        ];
        let gpt = TypeFix {
            disk: DiskId::Gpt(usb),
            offset: 1 << 20,
            from: TypeCode::Gpt(linux),
            to: TypeCode::Gpt(part::BASIC_DATA),
            type_name: "Linux filesystem".into(),
        };
        assert_eq!(
            fix_view(&gpt),
            FixView {
                table: "gpt",
                disk: "{5eae9c6e-b1a2-4be1-b33c-d406eab75dc1}".into(),
                offset: 1 << 20,
                from: "{0fc63daf-8483-4772-8e79-3d69d8477de4}".into(),
                to: "{ebd0a0a2-b9e5-4433-87c0-68b6b72699c7}".into(),
            }
        );
        let mbr = TypeFix {
            disk: DiskId::Mbr(0x1234_5678),
            offset: 65536,
            from: TypeCode::Mbr(0x83),
            to: TypeCode::Mbr(0x07),
            type_name: "Linux".into(),
        };
        let view = fix_view(&mbr);
        assert_eq!(
            (
                view.table,
                view.disk.as_str(),
                view.from.as_str(),
                view.to.as_str()
            ),
            ("mbr", "305419896", "131", "7")
        );
    }
}
