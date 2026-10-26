//! Installed application icons. Filesystem work and image decoding stay off the UI thread.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::time::{Duration, Instant};
use eframe::egui::{self, Color32, ColorImage, Rect, Stroke, TextureHandle, Ui, vec2};

const MAX_ICONS: usize = 512;
const RECHECK: Duration = Duration::from_secs(60);
enum Cached { Missing, Ready(TextureHandle) }
struct Entry { image: Cached, pending: bool, checked: Instant, used: Instant }
struct Request { name: String, context: egui::Context, has_image: bool }
struct ResultIcon { name: String, image: Option<ColorImage>, changed: bool }
struct SharedCache {
    entries: HashMap<String, Entry>,
    requests: SyncSender<Request>,
    results: Receiver<ResultIcon>,
}
/// Artwork belongs to the egui context, shared by launcher, shelf and tray.
/// Closing a surface never drops its textures or restarts image decoding.
#[derive(Default)]
pub struct IconCache;
impl IconCache {
    pub fn new() -> Self { Self }
    pub fn paint(&mut self, ui: &Ui, rect: Rect, icon: Option<&str>) {
        let cache = ui.ctx().data_mut(|data| data.get_temp_mut_or_insert_with(
            egui::Id::new("shared-application-icons-v2"),
            || std::sync::Arc::new(std::sync::Mutex::new(SharedCache::new())),
        ).clone());
        let mut cache = cache.lock().unwrap_or_else(|e|e.into_inner());
        while let Ok(result) = cache.results.try_recv() {
            if let Some(entry) = cache.entries.get_mut(&result.name) {
                entry.pending=false; entry.checked=Instant::now();
                if result.changed {
                    entry.image=result.image.map(|image| Cached::Ready(ui.ctx().load_texture(
                        format!("app-icon:{}",result.name),image,egui::TextureOptions::LINEAR,
                    ))).unwrap_or(Cached::Missing);
                }
            }
        }
        if let Some(name) = icon.filter(|name| !name.is_empty()) {
            let now=Instant::now();
            let request=cache.entries.get(name).is_none_or(|entry|!entry.pending && entry.checked.elapsed()>=RECHECK);
            if request {
                if cache.entries.len()>=MAX_ICONS && !cache.entries.contains_key(name) {
                    let oldest=cache.entries.iter().filter(|(_,entry)|!entry.pending).min_by_key(|(_,entry)|entry.used).map(|(name,_)|name.clone());
                    if let Some(oldest)=oldest {cache.entries.remove(&oldest);}
                }
                if cache.entries.contains_key(name) || cache.entries.len()<MAX_ICONS {
                    if cache.requests.try_send(Request{name:name.to_owned(),context:ui.ctx().clone(),has_image:cache.entries.get(name).is_some_and(|entry|matches!(entry.image,Cached::Ready(_)))}).is_ok() {
                        cache.entries.entry(name.to_owned()).and_modify(|entry|entry.pending=true).or_insert(Entry{image:Cached::Missing,pending:true,checked:now,used:now});
                    } else {ui.ctx().request_repaint_after(Duration::from_millis(50));}
                }
            }
            if let Some(entry)=cache.entries.get_mut(name) {
                entry.used=now;
                if let Cached::Ready(texture)=&entry.image {
                    let size=texture.size_vec2();
                    let scale=(rect.width()/size.x).min(rect.height()/size.y);
                    ui.painter().image(texture.id(),Rect::from_center_size(rect.center(),size*scale),Rect::from_min_max(egui::Pos2::ZERO,egui::pos2(1.0,1.0)),Color32::WHITE);
                    return;
                }
            }
        }
        let painter=ui.painter();
        let body=rect.shrink(rect.width()*0.1);
        painter.rect_filled(body,4.0,crate::ui::TRACK);
        painter.rect_stroke(body,4.0,Stroke::new(1.2,crate::ui::SECONDARY),egui::StrokeKind::Inside);
        let line=body.top()+body.height()*0.28;
        painter.line_segment([egui::pos2(body.left()+2.0,line),egui::pos2(body.right()-2.0,line)],Stroke::new(1.0,crate::ui::SECONDARY));
        painter.rect_filled(Rect::from_center_size(body.center()+vec2(0.0,3.0),body.size()*0.28),1.0,crate::ui::LABEL);
    }
}
impl SharedCache {
    fn new()->Self {
        let (requests,jobs)=mpsc::sync_channel::<Request>(32);
        let (results,completed)=mpsc::channel();
        std::thread::spawn(move|| {
            let roots=icon_roots();
            let mut signatures=HashMap::new();
            for request in jobs {
                let source=resolve(&request.name,&roots);
                let signature=source.as_deref().and_then(source_signature);
                let changed=!request.has_image || signatures.get(&request.name)!=Some(&signature);
                let image=if changed {source.as_deref().and_then(|path|warm_decode(path,signature.as_deref()))}else{None};
                if signatures.len()>=MAX_ICONS && !signatures.contains_key(&request.name) {
                    if let Some(oldest)=signatures.keys().next().cloned(){signatures.remove(&oldest);}
                }
                signatures.insert(request.name.clone(),signature);
                if results.send(ResultIcon{name:request.name,image,changed}).is_err(){break;}
                request.context.request_repaint();
            }
        });
        Self{entries:HashMap::new(),requests,results:completed}
    }
}
fn source_signature(path:&Path)->Option<String> {
    use std::hash::{Hash,Hasher};
    let meta=std::fs::metadata(path).ok()?;
    let mut hash=std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hash);meta.len().hash(&mut hash);meta.modified().ok()?.hash(&mut hash);
    use std::os::unix::fs::MetadataExt;
    meta.ino().hash(&mut hash);meta.ctime().hash(&mut hash);meta.ctime_nsec().hash(&mut hash);
    Some(format!("v2-{:016x}",hash.finish()))
}
fn cache_root()->Option<PathBuf> {
    std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).filter(|p|p.is_absolute())
        .or_else(||std::env::var_os("HOME").map(|home|PathBuf::from(home).join(".cache")))
        .map(|root|root.join("des-ui/app-icons"))
}
fn warm_decode(path:&Path,signature:Option<&str>)->Option<ColorImage> {
    let cached=cache_root().zip(signature).map(|(root,key)|root.join(format!("{key}.png")));
    decode_cached(path,cached)
}
fn decode_cached(path:&Path,cached:Option<PathBuf>)->Option<ColorImage> {
    if let Some(image)=cached.as_deref().and_then(decode){return Some(image);}
    let image=decode(path)?;
    if let Some(cached)=cached {
        if let Some(root)=cached.parent().filter(|root|std::fs::create_dir_all(root).is_ok()) {
            // Atomic publication; corrupt or interrupted cache files fall back to source.
            if let Ok(mut file)=tempfile::NamedTempFile::new_in(root) {
                let rgba:Vec<u8>=image.pixels.iter().flat_map(|pixel|pixel.to_srgba_unmultiplied()).collect();
                if image::write_buffer_with_format(&mut file,&rgba,image.size[0] as u32,image.size[1] as u32,image::ColorType::Rgba8,image::ImageFormat::Png).is_ok(){let _=file.persist(&cached);}
            }
            let mut files:Vec<_>=std::fs::read_dir(root).into_iter().flatten().flatten().filter(|entry|entry.path().extension().is_some_and(|ext|ext=="png")).collect();
            files.sort_by_key(|entry|entry.metadata().and_then(|meta|meta.modified()).ok());
            let extra=files.len().saturating_sub(MAX_ICONS);
            for entry in files.into_iter().take(extra){let _=std::fs::remove_file(entry.path());}
        }
    }
    Some(image)
}
fn icon_roots() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let data = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).filter(|path|path.is_absolute()).unwrap_or_else(|| home.join(".local/share"));
    let mut roots = vec![home.join(".icons"), data.join("icons"), data.join("pixmaps")];
    for root in std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into()).split(':').filter(|root| Path::new(root).is_absolute()) {
        roots.push(PathBuf::from(root).join("icons"));
        roots.push(PathBuf::from(root).join("pixmaps"));
    }
    roots
}
pub(crate) fn resolve(icon: &str, roots: &[PathBuf]) -> Option<PathBuf> {
    let path = Path::new(icon);
    if path.is_absolute() { return path.is_file().then(|| path.to_owned()); }
    if icon.contains('/') { return None; }
    let mut themes=Vec::new();
    if let Some(theme)=selected_theme(){theme_chain(&theme,roots,&mut themes);}
    for fallback in ["hicolor","Adwaita"] {theme_chain(fallback,roots,&mut themes);}
    for theme in themes {
        for root in roots {
            let mut budget=30000;
            if let Some(path)=find(&root.join(&theme),icon,0,&mut budget){return Some(path);}
        }
    }
    for root in roots {
        let mut budget=30000;
        if let Some(path)=find(root,icon,0,&mut budget){return Some(path);}
    }
    None
}
fn selected_theme()->Option<String> {
    if let Ok(theme)=std::env::var("GTK_ICON_THEME"){if safe_theme(&theme){return Some(theme);}}
    let config=std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).filter(|path|path.is_absolute())
        .or_else(||std::env::var_os("HOME").map(|home|PathBuf::from(home).join(".config")))?;
    for settings in ["gtk-4.0/settings.ini","gtk-3.0/settings.ini"] {
        if let Ok(text)=std::fs::read_to_string(config.join(settings)) {
            if let Some(theme)=text.lines().filter_map(|line|line.split_once('=')).find_map(|(key,value)|(key.trim()=="gtk-icon-theme-name").then(||value.trim().trim_matches('"').to_owned())).filter(|theme|safe_theme(theme)){return Some(theme);}
        }
    }
    None
}
fn safe_theme(theme:&str)->bool {!theme.is_empty() && theme!="." && theme!=".." && !theme.contains('/')}
fn theme_chain(theme:&str,roots:&[PathBuf],themes:&mut Vec<String>) {
    if !safe_theme(theme) || themes.len()>=32 || themes.iter().any(|name|name==theme){return;}
    themes.push(theme.to_owned());
    for root in roots {
        if let Ok(text)=std::fs::read_to_string(root.join(theme).join("index.theme")) {
            for inherited in text.lines().filter_map(|line|line.split_once('=')).filter(|(key,_)|key.trim()=="Inherits").flat_map(|(_,value)|value.split(',')).map(str::trim) {
                theme_chain(inherited,roots,themes);
            }
        }
    }
}
fn find(dir: &Path, icon: &str, depth: usize, budget: &mut usize) -> Option<PathBuf> {
    if depth > 7 || *budget == 0 { return None; }
    // Prefer ordinary 48px assets, then scalable assets, with theme fallbacks.
    for extension in ["", ".png", ".svg", ".xpm", ".jpg", ".jpeg", ".gif"] {
        let candidate = dir.join(format!("{icon}{extension}"));
        if candidate.is_file() { return Some(candidate); }
    }
    let mut children: Vec<_> = std::fs::read_dir(dir).ok()?.flatten().collect();
    children.sort_by_key(|entry| {
        let name = entry.file_name().to_string_lossy().into_owned();
        let priority = match name.as_str() { "hicolor" => 0, "Adwaita" => 1, "Yaru" => 2, "48x48" => 0, "64x64" => 1, "scalable" => 2, _ => 3 };
        (priority, name)
    });
    for entry in children {
        if *budget == 0 { break; }
        *budget -= 1;
        // Do not follow directory symlinks (themes can contain cycles).
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            if let Some(path) = find(&entry.path(), icon, depth+1, budget) { return Some(path); }
        }
    }
    None
}
fn decode(path: &Path) -> Option<ColorImage> {
    if std::fs::metadata(path).ok()?.len() > 8 * 1024 * 1024 { return None; }
    let raster = match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "svg" | "xpm" => {
            let output = tempfile::Builder::new().suffix(".png").tempfile().ok()?;
            let mut child = std::process::Command::new("/usr/bin/convert")
                .args(["-limit", "memory", "32MiB", "-limit", "map", "64MiB", "-background", "none"])
                .arg(path).args(["-thumbnail", "128x128"])
                .arg(output.path()).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().ok()?;
            let started = Instant::now();
            loop {
                match child.try_wait() {
                    Ok(Some(status)) if status.success() => break,
                    Ok(Some(_)) | Err(_) => return None,
                    Ok(None) if started.elapsed() < Duration::from_secs(2) => std::thread::sleep(Duration::from_millis(10)),
                    _ => { let _ = child.kill(); let _ = child.wait(); return None; }
                }
            }
            image::open(output.path()).ok()?
        }
        _ => {
            let mut reader = image::ImageReader::open(path).ok()?.with_guessed_format().ok()?;
            let mut limits = image::Limits::default();
            limits.max_image_width = Some(4096);
            limits.max_image_height = Some(4096);
            limits.max_alloc = Some(64 * 1024 * 1024);
            reader.limits(limits);
            reader.decode().ok()?
        }
    };
    let raster = if raster.width() > 128 || raster.height() > 128 {
        raster.thumbnail(128, 128)
    } else { raster }.into_rgba8();
    Some(ColorImage::from_rgba_unmultiplied([raster.width() as usize, raster.height() as usize], raster.as_raw()))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_textures_survive_surface_recreation() {
        let ctx=egui::Context::default();
        let shared=std::sync::Arc::new(std::sync::Mutex::new(SharedCache::new()));
        let texture=ctx.load_texture("fixture",ColorImage::new([2,2],Color32::WHITE),Default::default());
        let texture_id=texture.id();
        shared.lock().unwrap().entries.insert("fixture".into(),Entry{image:Cached::Ready(texture),pending:false,checked:Instant::now(),used:Instant::now()});
        ctx.data_mut(|data|data.insert_temp(egui::Id::new("shared-application-icons-v2"),shared.clone()));
        for _ in 0..3 {
            let mut surface=IconCache::new();
            let _=ctx.run(Default::default(),|ctx|{egui::CentralPanel::default().show(ctx,|ui|surface.paint(ui,Rect::from_min_size(egui::Pos2::ZERO,vec2(32.0,32.0)),Some("fixture")));});
            let cache=shared.lock().unwrap();
            assert!(matches!(&cache.entries["fixture"].image,Cached::Ready(texture) if texture.id()==texture_id));
            assert!(!cache.entries["fixture"].pending,"opening another surface must not queue decoding");
        }
    }
    #[test]
    fn warm_cache_is_reused_and_corruption_recovers_from_source() {
        let fixture=tempfile::tempdir().unwrap();
        let source=fixture.path().join("source.png");let cached=fixture.path().join("cache/thumbnail.png");
        image::RgbaImage::from_pixel(3,2,image::Rgba([12,34,56,255])).save(&source).unwrap();
        let before=source_signature(&source).unwrap();
        assert_eq!(decode_cached(&source,Some(cached.clone())).unwrap().size,[3,2]);
        assert!(cached.is_file());
        // A warm hit requires no source decode, even when the original is unreadable.
        std::fs::write(&source,b"broken").unwrap();
        assert_ne!(source_signature(&source).unwrap(),before);
        assert_eq!(decode_cached(&source,Some(cached.clone())).unwrap().size,[3,2]);
        image::RgbaImage::from_pixel(5,4,image::Rgba([56,34,12,255])).save(&source).unwrap();
        std::fs::write(&cached,b"broken cache").unwrap();
        assert_eq!(decode_cached(&source,Some(cached)).unwrap().size,[5,4]);
    }
    #[test]
    fn theme_inheritance_is_ordered_bounded_and_cycle_safe() {
        let fixture=tempfile::tempdir().unwrap();
        for (name,parent) in [("selected","parent"),("parent","selected")] {
            let dir=fixture.path().join(name);std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("index.theme"),format!("[Icon Theme]\nInherits={parent}\n")).unwrap();
        }
        let mut themes=Vec::new();theme_chain("selected",&[fixture.path().to_owned()],&mut themes);
        assert_eq!(themes,["selected","parent"]);theme_chain("../outside",&[],&mut themes);assert_eq!(themes.len(),2);
    }
    #[test]
    fn decodes_real_pixels_and_rejects_invalid_files() {
        let fixture = tempfile::tempdir().unwrap();
        let png = fixture.path().join("icon.png");
        image::RgbaImage::from_pixel(2, 3, image::Rgba([12, 34, 56, 255])).save(&png).unwrap();
        let icon = decode(&png).unwrap();
        assert_eq!(icon.size, [2, 3]);
        assert_eq!(icon.pixels[0], Color32::from_rgb(12, 34, 56));
        let broken = fixture.path().join("broken.png");
        std::fs::write(&broken, b"invalid").unwrap();
        assert!(decode(&broken).is_none());
    }
    #[test]
    fn resolves_svg_and_pixmap_filenames() {
        let fixture = tempfile::tempdir().unwrap();
        let theme = fixture.path().join("Adwaita/scalable/apps");
        std::fs::create_dir_all(&theme).unwrap();
        std::fs::write(theme.join("editor.svg"), []).unwrap();
        std::fs::write(fixture.path().join("legacy.xpm"), []).unwrap();
        let roots = [fixture.path().to_owned()];
        assert_eq!(resolve("editor", &roots), Some(theme.join("editor.svg")));
        assert_eq!(resolve("legacy.xpm", &roots), Some(fixture.path().join("legacy.xpm")));
        assert!(resolve("../outside", &roots).is_none());
    }
    #[test]
    fn finds_theme_icons_and_prefers_user_root() {
        let fixture = tempfile::tempdir().unwrap();
        let user = fixture.path().join("user");
        let system = fixture.path().join("system");
        for root in [&user, &system] {
            std::fs::create_dir_all(root.join("hicolor/48x48/apps")).unwrap();
            std::fs::write(root.join("hicolor/48x48/apps/editor.png"), []).unwrap();
        }
        assert_eq!(resolve("editor", &[user.clone(), system]), Some(user.join("hicolor/48x48/apps/editor.png")));
        let absolute = user.join("hicolor/48x48/apps/editor.png");
        assert_eq!(resolve(absolute.to_str().unwrap(), &[]), Some(absolute));
        assert_eq!(resolve("missing", &[user]), None);
    }
}
