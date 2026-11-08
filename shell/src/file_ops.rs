use std::{path::PathBuf, sync::atomic::{AtomicBool, AtomicU64}};
#[derive(Clone, Debug, PartialEq)]
pub enum Operation {
    CreateFolder { parent: PathBuf, name: String },
    Rename { source: PathBuf, name: String },
    Copy { source: PathBuf, directory: PathBuf, conflict: Conflict },
    Move { source: PathBuf, directory: PathBuf, conflict: Conflict },
    Trash { source: PathBuf },
    Restore { source: PathBuf },
    /// Packs items of one folder into a new .zip or .tar.gz named like `archive`.
    Compress { sources: Vec<PathBuf>, archive: PathBuf },
    /// Unpacks an archive into a new folder beside it.
    Extract { archive: PathBuf },
}
/// What a copy or move does when its name is already taken where it goes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Conflict {
    /// Stop, so a person can choose.
    #[default]
    Ask,
    /// Give the newcomer a free name, such as "report (2).pdf".
    KeepBoth,
    /// Move what is there to Trash first, so the replacement can be undone.
    Replace,
    /// Leave both where they are.
    Skip,
}
pub struct Completed {
    pub path: PathBuf,
    pub message: String,
    /// What a replacement moved to Trash, in Trash.
    pub replaced: Option<PathBuf>,
    /// Nothing was done, by choice.
    pub skipped: bool,
}
use std::{fs::{self, File, OpenOptions}, io::{Read, Write}, path::{Path, Component}, os::{fd::AsRawFd, unix::{ffi::OsStrExt, fs::{OpenOptionsExt, PermissionsExt, MetadataExt, DirBuilderExt}}}, sync::atomic::Ordering};

fn check(cancelled: &AtomicBool) -> Result<(), String> {
    if cancelled.load(Ordering::Relaxed) { Err("Operation cancelled".into()) } else { Ok(()) }
}
fn safe_path(path: &Path) -> Result<(), String> {
    if !path.is_absolute() || path.parent().is_none() || path.components().any(|c| matches!(c, Component::ParentDir | Component::CurDir)) {
        return Err("An absolute, non-root path without relative components is required".into());
    }
    Ok(())
}
fn name_valid(name: &str) -> Result<(), String> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\0') { Err("Enter a single folder or file name".into()) } else { Ok(()) }
}
fn rename_safe(source: &Path, target: &Path) -> Result<(), String> {
    rustix::fs::renameat_with(rustix::fs::CWD, source, rustix::fs::CWD, target, rustix::fs::RenameFlags::NOREPLACE)
        .map_err(|e| if e == rustix::io::Errno::XDEV { "Cannot move across filesystems; source was preserved. Copy the item first.".into() } else { format!("Cannot move item (source preserved): {e}") })
}
fn done(path: PathBuf, message: &str) -> Completed { Completed {path,message:message.into(),replaced:None,skipped:false} }
/// Where a copy or move would put its item.
pub fn target_of(operation: &Operation) -> Option<PathBuf> {
    match operation {
        Operation::Copy { source, directory, .. } | Operation::Move { source, directory, .. } => source.file_name().map(|name| directory.join(name)),
        _ => None,
    }
}
/// The first of "name", "name (2)", "name (3)"… not taken in `directory`, the
/// number before the extension.
pub fn free_name(directory: &Path, name: &std::ffi::OsStr) -> PathBuf {
    let first = directory.join(name);
    if fs::symlink_metadata(&first).is_err() { return first; }
    let path = Path::new(name);
    let text = name.to_string_lossy();
    let (stem, extension) = match (path.file_stem(), path.extension()) {
        (Some(stem), Some(extension)) if !text.starts_with('.') || text.matches('.').count() > 1 => (stem.to_string_lossy().into_owned(), format!(".{}", extension.to_string_lossy())),
        _ => (text.into_owned(), String::new()),
    };
    (2..).map(|n| directory.join(format!("{stem} ({n}){extension}"))).find(|candidate| fs::symlink_metadata(candidate).is_err()).unwrap()
}
/// Settles where a copy or move goes: the planned place, a free name, or the
/// planned place after what was there went to Trash. `None` means skip.
fn settle(source: &Path, directory: &Path, conflict: Conflict, cancelled: &AtomicBool) -> Result<Option<(PathBuf, Option<PathBuf>)>, String> {
    let name = source.file_name().ok_or("Missing source name")?;
    let target = directory.join(name);
    if fs::symlink_metadata(&target).is_err() { return Ok(Some((target, None))); }
    match conflict {
        Conflict::Ask => Err("Destination already exists".into()),
        Conflict::Skip => Ok(None),
        Conflict::KeepBoth => Ok(Some((free_name(directory, name), None))),
        Conflict::Replace => {
            if fs::canonicalize(&target).ok() == fs::canonicalize(source).ok() { return Err("An item cannot replace itself".into()); }
            let root = trash_root().ok_or("Home directory is unavailable")?;
            let trashed = trash_at(&target, &root, cancelled).map_err(|e| format!("Could not move the existing item to Trash: {e}"))?;
            Ok(Some((target, Some(trashed.path))))
        },
    }
}
fn skipped(source: PathBuf) -> Completed { Completed { path: source, message: "Item skipped".into(), replaced: None, skipped: true } }
pub fn execute(operation: Operation, cancelled: &AtomicBool, progress: &AtomicU64) -> Result<Completed, String> {
    progress.store(0,Ordering::Relaxed); check(cancelled)?;
    match operation {
        Operation::CreateFolder {parent,name} => {
            name_valid(&name)?; if !parent.is_absolute() { return Err("Folder must be absolute".into()); }
            let target=parent.join(name); fs::create_dir(&target).map_err(|e|e.to_string())?; Ok(done(target,"Folder created"))
        },
        Operation::Rename {source,name} => {
            safe_path(&source)?; name_valid(&name)?;
            let target=source.parent().unwrap().join(name); check(cancelled)?; rename_safe(&source,&target)?; Ok(done(target,"Item renamed"))
        },
        Operation::Move {source,directory,conflict} => {
            safe_path(&source)?; if !directory.is_absolute() { return Err("Destination must be absolute".into()); }
            check(cancelled)?;
            let Some((target, replaced)) = settle(&source, &directory, conflict, cancelled)? else { return Ok(skipped(source)) };
            rename_safe(&source,&target)?; Ok(Completed { replaced, ..done(target,"Item moved") })
        },
        Operation::Copy {source,directory,conflict} => copy_staged(&source,&directory,conflict,cancelled,progress),
        Operation::Trash {source} => trash_at(&source,&trash_root().ok_or("Home directory is unavailable")?,cancelled),
        Operation::Compress {sources,archive} => compress(&sources,&archive,cancelled),
        Operation::Extract {archive} => extract(&archive,cancelled),
        Operation::Restore {source} => restore_at(&source,&trash_root().ok_or("Home directory is unavailable")?,cancelled),
    }
}
fn copy_staged(source:&Path,directory:&Path,conflict:Conflict,cancelled:&AtomicBool,progress:&AtomicU64)->Result<Completed,String> {
    safe_path(source)?; if !directory.is_absolute() { return Err("Destination must be absolute".into()); }
    let directory=fs::canonicalize(directory).map_err(|e|e.to_string())?;
    let name = source.file_name().ok_or("Missing source name")?;
    if conflict == Conflict::Ask && fs::symlink_metadata(directory.join(name)).is_ok() { return Err("Destination already exists".into()); }
    let meta=fs::symlink_metadata(source).map_err(|e|e.to_string())?;
    if meta.is_dir() {
        let original=fs::canonicalize(source).map_err(|e|e.to_string())?;
        if directory.starts_with(&original) { return Err("Cannot copy a folder into itself".into()); }
    }
    let stage=tempfile::Builder::new().prefix(".desui-copy-").tempdir_in(&directory).map_err(|e|e.to_string())?;
    let payload=stage.path().join("payload"); let mut count=0;
    copy_node(source,&payload,0,&mut count,cancelled,progress)?;
    check(cancelled)?;
    // Settle the name only once the copy is whole, so a replacement happens last.
    let Some((target, replaced)) = settle(source, &directory, conflict, cancelled)? else { return Ok(skipped(source.to_path_buf())) };
    rename_safe(&payload,&target)?; Ok(Completed { replaced, ..done(target,"Item copied") })
}
fn copy_node(source:&Path,target:&Path,depth:usize,count:&mut usize,cancelled:&AtomicBool,progress:&AtomicU64)->Result<(),String> {
    check(cancelled)?; *count+=1;
    if depth>128 || *count>100_000 { return Err("Copy exceeds the folder depth or entry limit".into()); }
    let meta=fs::symlink_metadata(source).map_err(|e|e.to_string())?;
    if meta.file_type().is_symlink() {
        let link=fs::read_link(source).map_err(|e|e.to_string())?;
        std::os::unix::fs::symlink(link,target).map_err(|e|e.to_string())?;
    } else if meta.is_dir() {
        let fd=rustix::fs::open(source,rustix::fs::OFlags::RDONLY|rustix::fs::OFlags::DIRECTORY|rustix::fs::OFlags::NOFOLLOW|rustix::fs::OFlags::CLOEXEC,rustix::fs::Mode::empty()).map_err(|e|e.to_string())?;
        fs::create_dir(target).map_err(|e|e.to_string())?;
        fs::set_permissions(target,fs::Permissions::from_mode(0o700)).map_err(|e|e.to_string())?;
        let pinned=PathBuf::from(format!("/proc/self/fd/{}",fd.as_raw_fd()));
        for entry in fs::read_dir(&pinned).map_err(|e|e.to_string())? {
            let entry=entry.map_err(|e|e.to_string())?;
            copy_node(&pinned.join(entry.file_name()),&target.join(entry.file_name()),depth+1,count,cancelled,progress)?;
        }
        // Keep copied directories private and writable so failed staging can always be cleaned.
    } else if meta.is_file() {
        let fd=rustix::fs::open(source,rustix::fs::OFlags::RDONLY|rustix::fs::OFlags::NOFOLLOW|rustix::fs::OFlags::NONBLOCK|rustix::fs::OFlags::CLOEXEC,rustix::fs::Mode::empty()).map_err(|e|e.to_string())?;
        let mut input=File::from(fd); if !input.metadata().map_err(|e|e.to_string())?.is_file() { return Err("Source changed to an unsupported file type".into()); }
        let mut output=OpenOptions::new().write(true).create_new(true).mode(0o600).open(target).map_err(|e|e.to_string())?;
        let mut bytes=vec![0;65536]; loop { check(cancelled)?; let n=input.read(&mut bytes).map_err(|e|e.to_string())?; if n==0 { break; } output.write_all(&bytes[..n]).map_err(|e|e.to_string())?; progress.fetch_add(n as u64,Ordering::Relaxed); }
        output.sync_all().map_err(|e|e.to_string())?;
        output.set_permissions(fs::Permissions::from_mode(meta.permissions().mode() & 0o777)).map_err(|e|e.to_string())?;
    } else { return Err("Sockets, devices and named pipes cannot be copied".into()); }
    Ok(())
}
fn private_dir(path:&Path)->Result<(),String> {
    if !path.exists() { fs::DirBuilder::new().recursive(true).mode(0o700).create(path).map_err(|e|e.to_string())?; }
    let meta=fs::symlink_metadata(path).map_err(|e|e.to_string())?;
    if !meta.is_dir() || meta.file_type().is_symlink() || meta.uid()!=rustix::process::getuid().as_raw() || meta.permissions().mode() & 0o077 != 0 { return Err("Trash must be an owned, private directory without symlinks".into()); }
    Ok(())
}
// Resolve existing ancestors without creating the future Trash directories.
fn canonical_future_path(path:&Path)->Result<PathBuf,String> {
    match fs::canonicalize(path) {
        Ok(path)=>Ok(path),
        Err(error) if error.kind()==std::io::ErrorKind::NotFound=> {
            let parent=path.parent().ok_or("Cannot resolve Trash location")?;
            let name=path.file_name().ok_or("Cannot resolve Trash location")?;
            Ok(canonical_future_path(parent)?.join(name))
        },
        Err(error)=>Err(format!("Cannot resolve Trash location: {error}")),
    }
}
fn protect_trash_storage(source:&Path,root:&Path)->Result<(),String> {
    let root=canonical_future_path(root)?;
    // Canonicalize only the parent: a symlink outside Trash is an item, not its target.
    let source_location=fs::canonicalize(source.parent().ok_or("Missing source parent")?).map_err(|e|e.to_string())?.join(source.file_name().ok_or("Missing source name")?);
    if source_location.starts_with(&root) {return Err("Cannot trash the Trash folder, its service folders, or items already inside Trash".into());}
    let metadata=fs::symlink_metadata(source).map_err(|e|e.to_string())?;
    if metadata.is_dir() {
        let directory=fs::canonicalize(source).map_err(|e|e.to_string())?;
        if root.starts_with(&directory) {return Err("Cannot trash a folder containing your Trash storage".into());}
    }
    Ok(())
}
fn trash_at(source:&Path,root:&Path,cancelled:&AtomicBool)->Result<Completed,String> {
    safe_path(source)?; check(cancelled)?; protect_trash_storage(source,root)?; private_dir(root)?; let files=root.join("files");let info=root.join("info");private_dir(&files)?;private_dir(&info)?;
    let original=fs::canonicalize(source.parent().ok_or("Missing source parent")?).map_err(|e|e.to_string())?.join(source.file_name().ok_or("Missing source name")?);
    let mut record=tempfile::Builder::new().prefix("desui-").suffix(".trashinfo").tempfile_in(&info).map_err(|e|e.to_string())?;
    let encoded=original.as_os_str().as_bytes().iter().map(|&b|if b.is_ascii_alphanumeric() || b"/-._~".contains(&b) { (b as char).to_string() } else {format!("%{b:02X}")}).collect::<String>();
    write!(record,"[Trash Info]\nPath={}\nDeletionDate={}\n",encoded,chrono::Local::now().format("%Y-%m-%dT%H:%M:%S")).map_err(|e|e.to_string())?;
    record.as_file().sync_all().map_err(|e|e.to_string())?;
    let name=record.path().file_name().unwrap().to_string_lossy().trim_end_matches(".trashinfo").to_string(); let target=files.join(name);
    check(cancelled)?;
    // Persist metadata before touching the source: even metadata setup failures preserve it.
    let (_,metadata_path)=record.keep().map_err(|e|format!("Cannot save Trash metadata: {e}"))?;
    if let Err(error)=check(cancelled).and_then(|()|rename_safe(source,&target)) {
        let _=fs::remove_file(metadata_path); return Err(error);
    }
    Ok(done(target,"Item moved to Trash"))
}
/// The archive formats Files makes and opens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Archive { Zip, TarGz, TarXz, TarBz2, Tar, SevenZip }
impl Archive {
    pub fn of(path: &Path) -> Option<Archive> {
        let name = path.file_name()?.to_string_lossy().to_ascii_lowercase();
        [(".tar.gz", Archive::TarGz), (".tgz", Archive::TarGz), (".tar.xz", Archive::TarXz), (".txz", Archive::TarXz), (".tar.bz2", Archive::TarBz2),
         (".tar", Archive::Tar), (".zip", Archive::Zip), (".7z", Archive::SevenZip)]
            .into_iter().find(|(ending, _)| name.ends_with(ending)).map(|(_, kind)| kind)
    }
    /// The name without what marks it as an archive: "photos.tar.gz" → "photos".
    pub fn stem(path: &Path) -> String {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        for ending in [".tar.gz", ".tar.xz", ".tar.bz2", ".tgz", ".txz", ".tar", ".zip", ".7z"] {
            if name.to_ascii_lowercase().ends_with(ending) && name.len() > ending.len() { return name[..name.len() - ending.len()].to_string(); }
        }
        name
    }
}
/// Runs a packing tool, stopping it when cancelled.
fn run_tool(mut command: std::process::Command, cancelled: &AtomicBool) -> Result<(), String> {
    let program = command.get_program().to_string_lossy().into_owned();
    let mut child = command.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::piped())
        .spawn().map_err(|e| format!("{program} could not run: {e}. Install it to use this format."))?;
    loop {
        if cancelled.load(Ordering::Relaxed) { let _ = child.kill(); let _ = child.wait(); return Err("Operation cancelled".into()); }
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(status) if status.success() => return Ok(()),
            Some(status) => {
                let mut error = String::new();
                if let Some(mut stderr) = child.stderr.take() { let _ = stderr.read_to_string(&mut error); }
                let line = error.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string();
                return Err(if line.is_empty() { format!("{program} failed ({status})") } else { format!("{program}: {line}") });
            },
            None => std::thread::sleep(std::time::Duration::from_millis(40)),
        }
    }
}
fn compress(sources: &[PathBuf], archive: &Path, cancelled: &AtomicBool) -> Result<Completed, String> {
    let directory = archive.parent().ok_or("Missing archive folder")?;
    if sources.is_empty() { return Err("Nothing to compress".into()); }
    let mut names = Vec::new();
    for source in sources {
        safe_path(source)?;
        if source.parent() != Some(directory) { return Err("Compress items from one folder into that folder".into()); }
        names.push(source.file_name().ok_or("Missing source name")?.to_os_string());
    }
    let kind = Archive::of(archive).ok_or("Name the archive .zip or .tar.gz")?;
    // Packed under a private name, then given its own once whole.
    let stage = tempfile::Builder::new().prefix(".desui-archive-").tempdir_in(directory).map_err(|e| e.to_string())?;
    // Its real name inside the staging folder, as zip and 7z add their ending to a bare one.
    let packed = stage.path().join(archive.file_name().ok_or("Missing archive name")?);
    let mut command;
    match kind {
        Archive::Zip => { command = std::process::Command::new("zip"); command.current_dir(directory).args(["-r", "-q", "-y"]).arg(&packed).arg("--").args(&names); },
        Archive::SevenZip => { command = std::process::Command::new("7z"); command.current_dir(directory).args(["a", "-bd", "-y", "-t7z"]).arg(&packed).arg("--").args(&names); },
        tar => {
            let flag = match tar { Archive::TarGz => "-czf", Archive::TarXz => "-cJf", Archive::TarBz2 => "-cjf", _ => "-cf" };
            command = std::process::Command::new("tar"); command.arg(flag).arg(&packed).arg("-C").arg(directory).arg("--").args(&names);
        },
    }
    run_tool(command, cancelled)?;
    let made = packed;
    let target = free_name(directory, archive.file_name().ok_or("Missing archive name")?);
    check(cancelled)?; rename_safe(&made, &target)?;
    Ok(done(target, "Archive created"))
}
fn extract(archive: &Path, cancelled: &AtomicBool) -> Result<Completed, String> {
    safe_path(archive)?;
    let directory = archive.parent().ok_or("Missing archive folder")?;
    let kind = Archive::of(archive).ok_or("Not an archive Files can open")?;
    let stage = tempfile::Builder::new().prefix(".desui-extract-").tempdir_in(directory).map_err(|e| e.to_string())?;
    let mut command;
    match kind {
        Archive::Zip => { command = std::process::Command::new("unzip"); command.arg("-q").arg(archive).arg("-d").arg(stage.path()); },
        Archive::SevenZip => { command = std::process::Command::new("7z"); command.args(["x", "-bd", "-y"]).arg(format!("-o{}", stage.path().display())).arg(archive); },
        _ => { command = std::process::Command::new("tar"); command.arg("-xf").arg(archive).arg("-C").arg(stage.path()).args(["--no-same-owner", "--no-same-permissions"]); },
    }
    run_tool(command, cancelled)?;
    check(cancelled)?;
    // One folder inside comes out as itself; anything else goes in a folder named for the archive.
    let inside: Vec<_> = fs::read_dir(stage.path()).map_err(|e| e.to_string())?.flatten().collect();
    let target = match inside.as_slice() {
        [only] if only.file_type().is_ok_and(|t| t.is_dir()) => {
            let target = free_name(directory, &only.file_name());
            rename_safe(&only.path(), &target)?;
            target
        },
        _ => {
            let target = free_name(directory, std::ffi::OsStr::new(&Archive::stem(archive)));
            let kept = stage.keep();
            if let Err(error) = rename_safe(&kept, &target) { let _ = fs::remove_dir_all(&kept); return Err(error); }
            let _ = fs::set_permissions(&target, fs::Permissions::from_mode(0o755));
            target
        },
    };
    Ok(done(target, "Archive extracted"))
}
/// What puts a finished operation back as it was, in the order to run it; empty
/// when there is nothing to put back.
pub fn reverse(operation: &Operation, completed: &Completed) -> Vec<Operation> {
    if completed.skipped { return Vec::new(); }
    let mut steps = match operation {
        Operation::CreateFolder { .. } => vec![Operation::Trash { source: completed.path.clone() }],
        Operation::Rename { source, .. } => match source.file_name() {
            Some(name) => vec![Operation::Rename { source: completed.path.clone(), name: name.to_string_lossy().into_owned() }],
            None => Vec::new(),
        },
        Operation::Copy { .. } => vec![Operation::Trash { source: completed.path.clone() }],
        Operation::Move { source, .. } => {
            let Some(parent) = source.parent() else { return Vec::new() };
            let mut steps = vec![Operation::Move { source: completed.path.clone(), directory: parent.to_path_buf(), conflict: Conflict::Ask }];
            if completed.path.file_name() != source.file_name() {
                if let (Some(moved), Some(name)) = (completed.path.file_name(), source.file_name()) {
                    steps.push(Operation::Rename { source: parent.join(moved), name: name.to_string_lossy().into_owned() });
                }
            }
            steps
        },
        Operation::Trash { .. } => vec![Operation::Restore { source: completed.path.clone() }],
        Operation::Restore { .. } => vec![Operation::Trash { source: completed.path.clone() }],
        Operation::Compress { .. } | Operation::Extract { .. } => vec![Operation::Trash { source: completed.path.clone() }],
    };
    // What a replacement sent to Trash comes back once its place is free again.
    if let Some(trashed) = &completed.replaced { steps.push(Operation::Restore { source: trashed.clone() }); }
    steps
}
fn trash_root() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).filter(|p|p.is_absolute())
        .or_else(||std::env::var_os("HOME").map(PathBuf::from).filter(|p|p.is_absolute()).map(|h|h.join(".local/share")))
        .map(|data|data.join("Trash"))
}
/// Resolve the user's home Trash folder without filesystem I/O.
pub fn trash_files() -> Option<PathBuf> {
    trash_root().map(|root|root.join("files"))
}
/// Read an item's original name. Call only from a background worker: this reads metadata.
pub fn trash_original_name(source:&Path)->Option<std::ffi::OsString> {
    trash_original_name_at(source,&trash_root()?)
}
fn require_private_dir(path:&Path)->Result<(),String> {
    let meta=fs::symlink_metadata(path).map_err(|e|format!("Trash folder is unavailable: {e}"))?;
    if !meta.is_dir() || meta.file_type().is_symlink() || meta.uid()!=rustix::process::getuid().as_raw() || meta.permissions().mode() & 0o077 != 0 {
        return Err("Trash must be an owned, private directory without symlinks".into());
    }
    Ok(())
}
fn decode_trash_path(encoded:&[u8])->Result<PathBuf,String> {
    use std::{ffi::OsString,os::unix::ffi::OsStringExt};
    let mut bytes=Vec::with_capacity(encoded.len());let mut index=0;
    while index<encoded.len() {
        let byte=if encoded[index]==b'%' {
            let digits=encoded.get(index+1..index+3).ok_or("Invalid percent escape in Trash metadata")?;
            let high=(digits[0] as char).to_digit(16).ok_or("Invalid percent escape in Trash metadata")?;
            let low=(digits[1] as char).to_digit(16).ok_or("Invalid percent escape in Trash metadata")?;
            index+=3;((high<<4)|low) as u8
        } else {let byte=encoded[index];index+=1;byte};
        if byte==0 {return Err("Trash metadata contains a null byte".into());}
        bytes.push(byte);
    }
    let path=PathBuf::from(OsString::from_vec(bytes));safe_path(&path)?;Ok(path)
}
fn read_trash_entry(source:&Path,root:&Path)->Result<(PathBuf,PathBuf,PathBuf),String> {
    safe_path(source)?;
    require_private_dir(root)?;let files=root.join("files");let info=root.join("info");require_private_dir(&files)?;require_private_dir(&info)?;
    let files=fs::canonicalize(&files).map_err(|e|e.to_string())?;
    let source_parent=fs::canonicalize(source.parent().ok_or("Missing Trash item parent")?).map_err(|e|e.to_string())?;
    if source_parent!=files {return Err("Restore is only available for items directly inside your Trash".into());}
    let name=source.file_name().ok_or("Missing Trash item name")?;
    let source=files.join(name);
    fs::symlink_metadata(&source).map_err(|e|format!("Trashed item is unavailable: {e}"))?;
    let mut metadata_name=name.to_os_string();metadata_name.push(".trashinfo");let metadata_path=info.join(metadata_name);
    let fd=rustix::fs::open(&metadata_path,rustix::fs::OFlags::RDONLY|rustix::fs::OFlags::NOFOLLOW|rustix::fs::OFlags::NONBLOCK|rustix::fs::OFlags::CLOEXEC,rustix::fs::Mode::empty()).map_err(|e|format!("Cannot open Trash metadata: {e}"))?;
    let metadata=File::from(fd);let attributes=metadata.metadata().map_err(|e|e.to_string())?;
    if !attributes.is_file() || attributes.uid()!=rustix::process::getuid().as_raw() || attributes.permissions().mode()&0o077!=0 {return Err("Trash metadata must be a private, owned regular file".into());}
    let mut bytes=Vec::new();metadata.take(16385).read_to_end(&mut bytes).map_err(|e|e.to_string())?;
    if bytes.len()>16384 {return Err("Trash metadata exceeds the 16 KiB limit".into());}
    let mut lines=bytes.split(|&b|b==b'\n');
    if lines.next().map(|line|line.strip_suffix(b"\r").unwrap_or(line))!=Some(&b"[Trash Info]"[..]) {return Err("Invalid Trash metadata header".into());}
    let mut original=None;
    for line in lines {
        let line=line.strip_suffix(b"\r").unwrap_or(line);
        if line.starts_with(b"[") {break;}
        if let Some(path)=line.strip_prefix(b"Path=") {if original.is_some() {return Err("Ambiguous Trash metadata path".into());}original=Some(decode_trash_path(path)?);}
    }
    let original=original.ok_or("Trash metadata is missing the original path")?;
    Ok((source,metadata_path,original))
}
fn restore_at(source:&Path,root:&Path,cancelled:&AtomicBool)->Result<Completed,String> {
    check(cancelled)?;
    let (source,metadata_path,original)=read_trash_entry(source,root)?;
    if !original.parent().ok_or("Missing original parent")?.is_dir() {return Err("The original parent folder no longer exists; restore it first".into());}
    check(cancelled)?;rename_safe(&source,&original)?;
    let message=if fs::remove_file(&metadata_path).is_ok() {"Item restored from Trash"} else {"Item restored; old Trash metadata could not be removed"};
    Ok(done(original,message))
}

fn trash_original_name_at(source:&Path,root:&Path)->Option<std::ffi::OsString> {
    let (_,_,original)=read_trash_entry(source,root).ok()?;
    original.file_name().map(|name|name.to_os_string())
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::symlink};
    fn run(op: Operation) -> Result<Completed,String> { execute(op, &AtomicBool::new(false), &AtomicU64::new(0)) }
    #[test]
    fn creates_renames_copies_and_moves_without_overwriting() {
        let t = tempfile::tempdir().unwrap(); let p = t.path();
        run(Operation::CreateFolder {parent:p.into(), name:"a".into()}).unwrap();
        fs::write(p.join("a/f"), b"contents").unwrap();
        run(Operation::Rename {source:p.join("a"), name:"b".into()}).unwrap();
        fs::create_dir(p.join("out")).unwrap();
        run(Operation::Copy {source:p.join("b"), directory:p.join("out"), conflict: Conflict::Ask}).unwrap();
        assert_eq!(fs::read(p.join("out/b/f")).unwrap(), b"contents");
        assert!(run(Operation::Move {source:p.join("b"), directory:p.join("out"), conflict: Conflict::Ask}).is_err());
        assert!(p.join("b/f").exists());
        fs::create_dir(p.join("other")).unwrap();
        run(Operation::Move {source:p.join("b"), directory:p.join("other"), conflict: Conflict::Ask}).unwrap();
        assert!(!p.join("b").exists());
    }
    #[test]
    fn conflicts_keep_both_skip_or_replace_and_each_can_be_undone() {
        let home = tempfile::tempdir().unwrap();
        let root = home.path().join("trash");
        let p = home.path();
        fs::create_dir(p.join("in")).unwrap(); fs::create_dir(p.join("out")).unwrap();
        fs::write(p.join("in/report.pdf"), b"new").unwrap(); fs::write(p.join("out/report.pdf"), b"old").unwrap();
        let copy = |conflict| execute(Operation::Copy { source: p.join("in/report.pdf"), directory: p.join("out"), conflict }, &AtomicBool::new(false), &AtomicU64::new(0));
        assert!(copy(Conflict::Ask).is_err());
        let both = copy(Conflict::KeepBoth).unwrap();
        assert_eq!(both.path, fs::canonicalize(p.join("out")).unwrap().join("report (2).pdf"));
        assert!(copy(Conflict::Skip).unwrap().skipped);
        assert_eq!(fs::read(p.join("out/report.pdf")).unwrap(), b"old");
        assert_eq!(free_name(&p.join("out"), std::ffi::OsStr::new("report.pdf")), p.join("out/report (3).pdf"));
        assert_eq!(free_name(&p.join("out"), std::ffi::OsStr::new(".bashrc")), p.join("out/.bashrc"));
        fs::write(p.join("out/.bashrc"), b"").unwrap();
        assert_eq!(free_name(&p.join("out"), std::ffi::OsStr::new(".bashrc")), p.join("out/.bashrc (2)"));
        // Replacing sends the old one to Trash, and undoing brings it back.
        let operation = Operation::Copy { source: p.join("in/report.pdf"), directory: p.join("out"), conflict: Conflict::Replace };
        let target = fs::canonicalize(p.join("out")).unwrap().join("report.pdf");
        let trashed = trash_at(&target, &root, &AtomicBool::new(false)).unwrap();
        let replaced = Completed { path: target.clone(), message: String::new(), replaced: Some(trashed.path.clone()), skipped: false };
        assert_eq!(reverse(&operation, &replaced), vec![Operation::Trash { source: target.clone() }, Operation::Restore { source: trashed.path }]);
        let moved = Completed { path: p.join("out/notes (2).txt"), message: String::new(), replaced: None, skipped: false };
        assert_eq!(reverse(&Operation::Move { source: p.join("in/notes.txt"), directory: p.join("out"), conflict: Conflict::KeepBoth }, &moved),
            vec![Operation::Move { source: p.join("out/notes (2).txt"), directory: p.join("in"), conflict: Conflict::Ask }, Operation::Rename { source: p.join("in/notes (2).txt"), name: "notes.txt".into() }]);
        assert!(reverse(&operation, &skipped(p.join("x"))).is_empty());
    }
    #[test]
    fn compress_then_extract_round_trips_and_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        fs::create_dir(p.join("photos")).unwrap(); fs::write(p.join("photos/a.txt"), b"A").unwrap(); fs::write(p.join("b.txt"), b"B").unwrap();
        let run = |op| execute(op, &AtomicBool::new(false), &AtomicU64::new(0));
        for name in ["pack.zip", "pack.tar.gz"] {
            let made = run(Operation::Compress { sources: vec![p.join("photos"), p.join("b.txt")], archive: p.join(name) }).unwrap();
            assert_eq!(made.path, p.join(name));
            let out = run(Operation::Extract { archive: made.path.clone() }).unwrap();
            assert_eq!(out.path, p.join("pack"), "several items come out in a folder named for the archive");
            assert_eq!(fs::read(out.path.join("photos/a.txt")).unwrap(), b"A");
            assert_eq!(fs::read(out.path.join("b.txt")).unwrap(), b"B");
            fs::remove_dir_all(&out.path).unwrap();
        }
        let again = run(Operation::Compress { sources: vec![p.join("b.txt")], archive: p.join("pack.zip") }).unwrap();
        assert_eq!(again.path, p.join("pack (2).zip"), "an archive never replaces one already there");
        let single = run(Operation::Compress { sources: vec![p.join("photos")], archive: p.join("photos.tar.gz") }).unwrap();
        fs::rename(p.join("photos"), p.join("photos-original")).unwrap();
        assert_eq!(run(Operation::Extract { archive: single.path }).unwrap().path, p.join("photos"), "one folder inside comes out as itself");
        assert_eq!(Archive::stem(Path::new("x.Tar.GZ")), "x");
        assert!(run(Operation::Compress { sources: vec![p.join("b.txt")], archive: p.join("sub/x.zip") }).is_err());
        assert!(!fs::read_dir(p).unwrap().flatten().any(|e| e.file_name().to_string_lossy().starts_with(".desui-")), "no staging is left behind");
    }
    #[test]
    fn rejects_unsafe_names_conflicts_aliases_and_cancellation() {
        let t=tempfile::tempdir().unwrap(); let p=t.path();
        for name in ["", ".", "..", "a/b", "a\0b"] { assert!(run(Operation::CreateFolder {parent:p.into(),name:name.into()}).is_err()); }
        fs::create_dir(p.join("a")).unwrap(); symlink(p.join("a"),p.join("alias")).unwrap();
        assert!(run(Operation::Copy {source:p.join("a"),directory:p.join("alias"), conflict: Conflict::Ask}).is_err());
        fs::write(p.join("f"),b"x").unwrap(); symlink(p.join("missing"),p.join("taken")).unwrap();
        assert!(run(Operation::Rename {source:p.join("f"),name:"taken".into()}).is_err());
        assert_eq!(fs::read(p.join("f")).unwrap(),b"x");
        assert!(execute(Operation::Copy {source:p.join("f"),directory:p.into(), conflict: Conflict::Ask},&AtomicBool::new(true),&AtomicU64::new(0)).is_err());
    }
    #[test]
    fn copy_preserves_symlinks_without_following_them() {
        let t=tempfile::tempdir().unwrap(); let p=t.path(); fs::create_dir(p.join("a")).unwrap(); fs::create_dir(p.join("out")).unwrap();
        symlink("../missing",p.join("a/link")).unwrap();
        run(Operation::Copy {source:p.join("a"),directory:p.join("out"), conflict: Conflict::Ask}).unwrap();
        assert_eq!(fs::read_link(p.join("out/a/link")).unwrap(),PathBuf::from("../missing"));
    }
    #[test]
    fn trash_is_recoverable_and_metadata_failure_preserves_source() {
        let t=tempfile::tempdir().unwrap(); let p=t.path(); let source=p.join("a b");
        fs::write(&source,b"precious").unwrap();
        fs::write(p.join("blocked"),b"x").unwrap();
        assert!(trash_at(&source,&p.join("blocked"),&AtomicBool::new(false)).is_err());
        assert!(source.exists());
        let done=trash_at(&source,&p.join("trash"),&AtomicBool::new(false)).unwrap();
        assert!(!source.exists()); assert_eq!(fs::read(&done.path).unwrap(),b"precious");
        let info=fs::read_dir(p.join("trash/info")).unwrap().next().unwrap().unwrap().path();
        let metadata=fs::read_to_string(info).unwrap(); assert!(metadata.contains("a%20b")); assert!(metadata.contains("DeletionDate="));
    }

    #[test]
    fn unsupported_pipe_leaves_no_partial_copy_and_progress_counts_bytes() {
        let t=tempfile::tempdir().unwrap();let p=t.path();fs::create_dir(p.join("out")).unwrap();
        rustix::fs::mknodat(rustix::fs::CWD,p.join("pipe"),rustix::fs::FileType::Fifo,rustix::fs::Mode::RUSR|rustix::fs::Mode::WUSR,0).unwrap();
        assert!(run(Operation::Copy {source:p.join("pipe"),directory:p.join("out"), conflict: Conflict::Ask}).is_err());
        assert_eq!(fs::read_dir(p.join("out")).unwrap().count(),0);
        let source=p.join(std::ffi::OsStr::from_bytes(b"native-\xff"));fs::write(&source,vec![42;150000]).unwrap();let progress=AtomicU64::new(0);
        let result=execute(Operation::Copy {source,directory:p.join("out"), conflict: Conflict::Ask},&AtomicBool::new(false),&progress).unwrap();
        assert_eq!(progress.load(Ordering::Relaxed),150000);assert_eq!(fs::metadata(result.path).unwrap().len(),150000);
    }

    #[test]
    fn excessive_depth_cleans_staging_without_touching_source() {
        let t=tempfile::tempdir().unwrap();let p=t.path();let source=p.join("deep");fs::create_dir(&source).unwrap();fs::create_dir(p.join("out")).unwrap();
        let mut current=source.clone();for _ in 0..130 {current=current.join("d");fs::create_dir(&current).unwrap();}
        fs::write(current.join("leaf"),b"keep").unwrap();
        assert!(run(Operation::Copy {source,directory:p.join("out"), conflict: Conflict::Ask}).is_err());
        assert_eq!(fs::read(current.join("leaf")).unwrap(),b"keep");assert_eq!(fs::read_dir(p.join("out")).unwrap().count(),0);
    }

    #[test]
    fn restore_round_trips_native_path_and_preserves_conflicts() {
        let t=tempfile::tempdir().unwrap();let p=t.path();let root=p.join("trash");
        let source=p.join(std::ffi::OsStr::from_bytes(b"original-\xff"));fs::write(&source,b"recover me").unwrap();
        let trashed=trash_at(&source,&root,&AtomicBool::new(false)).unwrap().path;
        fs::write(&source,b"new file").unwrap();
        assert!(restore_at(&trashed,&root,&AtomicBool::new(false)).is_err());
        assert_eq!(fs::read(&source).unwrap(),b"new file");assert_eq!(fs::read(&trashed).unwrap(),b"recover me");assert_eq!(fs::read_dir(root.join("info")).unwrap().count(),1);
        fs::remove_file(&source).unwrap();
        let restored=restore_at(&trashed,&root,&AtomicBool::new(false)).unwrap();
        assert_eq!(restored.path,source);assert_eq!(fs::read(source).unwrap(),b"recover me");assert!(!trashed.exists());assert_eq!(fs::read_dir(root.join("info")).unwrap().count(),0);
    }
    #[test]
    fn restore_rejects_outside_items_and_bad_metadata_and_missing_parent() {
        let t=tempfile::tempdir().unwrap();let p=t.path();let root=p.join("trash");let parent=p.join("parent");fs::create_dir(&parent).unwrap();let source=parent.join("f");fs::write(&source,b"preserved").unwrap();
        let trashed=trash_at(&source,&root,&AtomicBool::new(false)).unwrap().path;
        let info=fs::read_dir(root.join("info")).unwrap().next().unwrap().unwrap().path();let original=fs::read(&info).unwrap();
        let outside=p.join("outside");fs::write(&outside,b"outside").unwrap();
        assert!(restore_at(&outside,&root,&AtomicBool::new(false)).is_err());
        for metadata in [b"[Trash Info]\nPath=/\n".to_vec(),b"[Trash Info]\nPath=%XX\n".to_vec(),vec![b'x';16385]] {
            fs::write(&info,metadata).unwrap();assert!(restore_at(&trashed,&root,&AtomicBool::new(false)).is_err());assert!(trashed.exists());assert!(info.exists());
        }
        fs::write(&info,original).unwrap();fs::remove_dir(&parent).unwrap();
        assert!(restore_at(&trashed,&root,&AtomicBool::new(false)).is_err());assert!(trashed.exists());assert!(info.exists());assert!(!parent.exists());
    }

    #[test]
    fn restore_preserves_dangling_links_and_rejects_metadata_symlinks() {
        let t=tempfile::tempdir().unwrap();let p=t.path();let root=p.join("trash");let original=p.join("link");symlink("missing",&original).unwrap();
        let trashed=trash_at(&original,&root,&AtomicBool::new(false)).unwrap().path;
        let info=fs::read_dir(root.join("info")).unwrap().next().unwrap().unwrap().path();let bytes=fs::read(&info).unwrap();let external=p.join("external-info");fs::write(&external,&bytes).unwrap();fs::remove_file(&info).unwrap();symlink(&external,&info).unwrap();
        assert!(restore_at(&trashed,&root,&AtomicBool::new(false)).is_err());assert!(fs::symlink_metadata(&trashed).unwrap().file_type().is_symlink());
        fs::remove_file(&info).unwrap();fs::write(&info,&bytes).unwrap();fs::set_permissions(&info,fs::Permissions::from_mode(0o600)).unwrap();
        symlink("also-missing",&original).unwrap();assert!(restore_at(&trashed,&root,&AtomicBool::new(false)).is_err());assert!(info.exists());fs::remove_file(&original).unwrap();
        assert!(restore_at(&trashed,&root,&AtomicBool::new(true)).is_err());assert!(info.exists());
        restore_at(&trashed,&root,&AtomicBool::new(false)).unwrap();assert_eq!(fs::read_link(&original).unwrap(),PathBuf::from("missing"));assert!(!info.exists());
    }

    #[test]
    fn trash_display_name_preserves_bytes_without_mutating_or_accepting_unsafe_metadata() {
        let t=tempfile::tempdir().unwrap();let p=t.path();let root=p.join("trash");let name=std::ffi::OsStr::from_bytes(b"real-name-\xff");let source=p.join(name);fs::write(&source,b"keep").unwrap();
        let trashed=trash_at(&source,&root,&AtomicBool::new(false)).unwrap().path;let info=fs::read_dir(root.join("info")).unwrap().next().unwrap().unwrap().path();let bytes=fs::read(&info).unwrap();
        assert_eq!(trash_original_name_at(&trashed,&root),Some(name.to_os_string()));assert_eq!(fs::read(&info).unwrap(),bytes);assert_eq!(fs::read(&trashed).unwrap(),b"keep");
        let outside=p.join("outside");fs::write(&outside,b"outside").unwrap();assert_eq!(trash_original_name_at(&outside,&root),None);
        for metadata in [b"[Trash Info]\nPath=relative\n".to_vec(),b"[Trash Info]\nPath=/a/../unsafe\n".to_vec(),b"[Trash Info]\nPath=%XX\n".to_vec(),vec![b'x';16385]] {fs::write(&info,metadata).unwrap();assert_eq!(trash_original_name_at(&trashed,&root),None);assert!(trashed.exists());}
    }

    #[test]
    fn trash_rejects_own_service_folders_contents_and_ancestors() {
        let t=tempfile::tempdir().unwrap();let p=t.path();let container=p.join("container");fs::create_dir(&container).unwrap();let root=container.join("trash");let source=p.join("precious");fs::write(&source,b"recover").unwrap();
        let item=trash_at(&source,&root,&AtomicBool::new(false)).unwrap().path;
        let info=fs::read_dir(root.join("info")).unwrap().next().unwrap().unwrap().path();let record=fs::read(&info).unwrap();
        for forbidden in [root.join("info"),root.join("files"),root.clone(),container.clone(),item.clone(),info.clone()] {
            assert!(trash_at(&forbidden,&root,&AtomicBool::new(false)).is_err(),"must reject {}",forbidden.display());
            assert_eq!(fs::read(&item).unwrap(),b"recover");assert_eq!(fs::read(&info).unwrap(),record);assert_eq!(fs::read_dir(root.join("info")).unwrap().count(),1);
        }
        let alias=p.join("container-alias");symlink(&container,&alias).unwrap();assert!(trash_at(&alias.join("trash/files"),&root,&AtomicBool::new(false)).is_err());
        assert_eq!(fs::read(&info).unwrap(),record);
    }
    #[test]
    fn trashing_symlink_to_trash_preserves_target_and_records() {
        let t=tempfile::tempdir().unwrap();let p=t.path();let root=p.join("trash");let source=p.join("original");fs::write(&source,b"keep").unwrap();let item=trash_at(&source,&root,&AtomicBool::new(false)).unwrap().path;
        let link=p.join("shortcut");symlink(&root,&link).unwrap();let moved=trash_at(&link,&root,&AtomicBool::new(false)).unwrap().path;
        assert_eq!(fs::read_link(moved).unwrap(),root);assert_eq!(fs::read(item).unwrap(),b"keep");assert_eq!(fs::read_dir(root.join("info")).unwrap().count(),2);
    }

}
