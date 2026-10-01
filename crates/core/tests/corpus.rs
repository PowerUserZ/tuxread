//! The adoption gate (spec §7.2): every corpus image must read exactly as the
//! Linux kernel read it, or be refused for the declared reason.
//!
//! Build the corpus with `scripts/make-corpus.sh <dir>` (Linux/WSL, as root), then:
//! `TUXREAD_CORPUS=<dir> cargo test -p tuxread-core --test corpus`

use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;
use std::sync::Arc;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tuxread_core::cache::CachedDev;
use tuxread_core::dev::FileDev;
use tuxread_core::fs::{Fs, Kind, join};
use tuxread_core::probe::{self, Node, NodeKind, Status};

#[derive(Deserialize)]
struct Expect {
    volumes: Vec<VolumeExpect>,
}

#[derive(Deserialize)]
#[serde(tag = "outcome", rename_all = "lowercase")]
enum VolumeExpect {
    Match { entries: Vec<Want> },
    Unsupported { reason: String },
}

#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
struct Want {
    path: String,
    kind: String,
    size: u64,
    sha256: Option<String>,
    mtime_ns: i64,
    mode: u32,
    uid: u32,
    gid: u32,
    link: Option<String>,
}

#[test]
fn corpus_reads_like_the_linux_kernel() {
    let Some(dir) = std::env::var_os("TUXREAD_CORPUS") else {
        eprintln!("TUXREAD_CORPUS is not set; skipping the corpus gate");
        return;
    };
    let mut expects: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.to_string_lossy().ends_with(".expect.json"))
        .collect();
    expects.sort();
    assert!(!expects.is_empty(), "no *.expect.json files in {dir:?}");

    let mut failures = Vec::new();
    for expect_path in &expects {
        let name = expect_path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .replace(".expect.json", "");
        let expect: Expect = serde_json::from_slice(&std::fs::read(expect_path).unwrap()).unwrap();
        let image = Path::new(&dir).join(format!("{name}.img"));
        let dev = Arc::new(CachedDev::new(Arc::new(FileDev::open(&image).unwrap())));
        let nodes = probe::probe(dev);
        let leaves = probe::leaves(&nodes);
        if leaves.len() != expect.volumes.len() {
            failures.push(format!(
                "{name}: found {} volumes, expected {}",
                leaves.len(),
                expect.volumes.len()
            ));
            continue;
        }
        for (i, (leaf, want)) in leaves.iter().zip(&expect.volumes).enumerate() {
            match check(leaf, want) {
                Ok(()) => eprintln!("ok   {name} #{}", i + 1),
                Err(e) => failures.push(format!("{name} #{}: {e}", i + 1)),
            }
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

fn check(leaf: &Node, want: &VolumeExpect) -> Result<(), String> {
    match (&leaf.kind, want) {
        (NodeKind::Volume(volume), VolumeExpect::Match { entries }) => {
            let fs = volume.open().map_err(|e| e.to_string())?;
            let mut got = Vec::new();
            walk(fs.as_ref(), b"/", &mut got)?;
            compare(entries, &got)
        }
        (
            NodeKind::Detected {
                status: Status::NotSupported(why),
                ..
            },
            VolumeExpect::Unsupported { reason },
        ) => {
            if why.contains(reason.as_str()) {
                Ok(())
            } else {
                Err(format!(
                    "refused for \"{why}\", expected the reason to mention \"{reason}\""
                ))
            }
        }
        (NodeKind::Detected { status, .. }, _) => Err(format!("got {} ({status:?})", leaf.label)),
        (_, VolumeExpect::Unsupported { reason }) => {
            Err(format!("read it, but expected \"not supported: {reason}\""))
        }
        (NodeKind::Partition { .. }, _) => Err("leaf is a partition".into()),
    }
}

fn walk(fs: &dyn Fs, dir: &[u8], out: &mut Vec<Want>) -> Result<(), String> {
    let shown = |p: &[u8]| String::from_utf8_lossy(p).into_owned();
    for e in fs
        .read_dir(dir)
        .map_err(|e| format!("{}: {e}", shown(dir)))?
    {
        let path = join(dir, &e.name);
        let kind = match e.kind {
            Kind::File => "file",
            Kind::Dir => "dir",
            Kind::Symlink => "symlink",
            Kind::Other => "other",
        };
        let sha256 = if e.kind == Kind::File {
            let mut reader = fs
                .open(&path)
                .map_err(|err| format!("{}: {err}", shown(&path)))?;
            let mut hasher = Sha256::new();
            let mut buf = vec![0u8; 1 << 20];
            loop {
                let n = reader
                    .read(&mut buf)
                    .map_err(|err| format!("{}: {err}", shown(&path)))?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
            }
            Some(format!("{:x}", hasher.finalize()))
        } else {
            None
        };
        let link = if e.kind == Kind::Symlink {
            Some(
                B64.encode(
                    fs.read_link(&path)
                        .map_err(|err| format!("{}: {err}", shown(&path)))?,
                ),
            )
        } else {
            None
        };
        let mtime = e
            .mtime
            .ok_or_else(|| format!("{}: no mtime", shown(&path)))?;
        out.push(Want {
            path: B64.encode(&path),
            kind: kind.into(),
            size: e.size,
            sha256,
            mtime_ns: mtime.secs * 1_000_000_000 + i64::from(mtime.nanos),
            mode: e.mode,
            uid: e.uid,
            gid: e.gid,
            link,
        });
        if e.kind == Kind::Dir {
            walk(fs, &path, out)?;
        }
    }
    Ok(())
}

fn compare(want: &[Want], got: &[Want]) -> Result<(), String> {
    let key = |w: &Want| B64.decode(&w.path).unwrap();
    let want: BTreeMap<_, _> = want.iter().map(|w| (key(w), w)).collect();
    let got: BTreeMap<_, _> = got.iter().map(|w| (key(w), w)).collect();
    let mut diffs = Vec::new();
    for (path, w) in &want {
        let shown = String::from_utf8_lossy(path);
        match got.get(path) {
            None => diffs.push(format!("missing {shown}")),
            Some(g) if g != w => diffs.push(format!(
                "differs {shown}\n    kernel: {w:?}\n    engine: {g:?}"
            )),
            Some(_) => {}
        }
    }
    diffs.extend(
        got.keys()
            .filter(|p| !want.contains_key(*p))
            .map(|p| format!("extra {}", String::from_utf8_lossy(p))),
    );
    if diffs.is_empty() {
        Ok(())
    } else {
        let total = diffs.len();
        diffs.truncate(10);
        Err(format!(
            "{total} difference(s), first {}:\n  {}",
            diffs.len(),
            diffs.join("\n  ")
        ))
    }
}
