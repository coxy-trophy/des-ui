//! Bounded local filename lookup; file contents are never read.
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct Found {
    pub paths: Vec<PathBuf>,
    pub truncated: bool,
    pub errors: usize,
}

pub fn parse_roots(text: &str) -> Result<Vec<PathBuf>, &'static str> {
    let mut roots=Vec::new();
    for word in text.split(';').map(str::trim).filter(|word| !word.is_empty()) {
        let path=PathBuf::from(word);
        if !path.is_absolute() || word.chars().any(char::is_control) { return Err("Use absolute folder paths, separated by semicolons."); }
        if !roots.contains(&path) { roots.push(path); }
        if roots.len()>8 { return Err("Choose at most eight folders."); }
    }
    if roots.is_empty() { return Err("Choose at least one folder."); }
    Ok(roots)
}

pub fn search(roots:&[PathBuf], query:&str, generation:u64, current:&AtomicU64) -> Found {
    let mut found=Found::default();
    let query=query.trim().to_lowercase();
    if query.chars().count()<2 { return found; }
    let deadline=Instant::now()+Duration::from_millis(400);
    let mut stack:Vec<_>=roots.iter().take(8).cloned().map(|path|(path,0usize)).collect();
    let mut visited=0;
    let mut seen=std::collections::HashSet::new();
    while let Some((directory,depth))=stack.pop() {
        if current.load(Ordering::Relaxed)!=generation { return Found::default(); }
        if Instant::now()>=deadline || visited>=20_000 { found.truncated=true; break; }
        let Ok(meta)=std::fs::symlink_metadata(&directory) else { found.errors+=1; continue };
        if !meta.is_dir() || meta.file_type().is_symlink() { continue; }
        let listing=match std::fs::read_dir(&directory) { Ok(listing)=>listing, Err(_)=>{ found.errors+=1; continue; } };
        for item in listing {
            if current.load(Ordering::Relaxed)!=generation { return Found::default(); }
            visited+=1;
            if Instant::now()>=deadline || visited>20_000 { found.truncated=true; break; }
            let Ok(item)=item else { found.errors+=1; continue };
            let name=item.file_name(); let name=name.to_string_lossy();
            if name.starts_with('.') || matches!(name.as_ref(),"node_modules"|"target"|"dist"|"build"|"vendor") { continue; }
            let Ok(kind)=item.file_type() else { found.errors+=1; continue };
            if kind.is_symlink() { continue; }
            let path=item.path();
            if name.to_lowercase().contains(&query) && seen.insert(path.clone()) {
                found.paths.push(path.clone());
                if found.paths.len()==32 { found.truncated=true; break; }
            }
            if kind.is_dir() && depth<5 { stack.push((path,depth+1)); }
        }
        if found.truncated { break; }
    }
    found.paths.sort_by_key(|p| {
        let name=p.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
        (!name.starts_with(&query),name,p.clone())
    });
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;
    fn fixture() -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("des-find-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir(&root).unwrap(); root
    }
    #[test]
    fn finds_unicode_names_and_skips_hidden_build_and_symlink_trees() {
        let root=fixture();
        for dir in ["Docs", ".private", "node_modules", "target"] { std::fs::create_dir(root.join(dir)).unwrap(); std::fs::write(root.join(dir).join("RÉSUMÉ.txt"),"").unwrap(); }
        std::os::unix::fs::symlink(root.join("Docs"),root.join("alias")).unwrap();
        let found=search(&[root.clone()],"résumé",1,&AtomicU64::new(1));
        assert_eq!(found.paths,vec![root.join("Docs/RÉSUMÉ.txt")]);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn cancellation_and_result_limit_bound_work() {
        let root=fixture();
        for n in 0..70 { std::fs::write(root.join(format!("report-{n}.txt")),"").unwrap(); }
        let cancelled=search(&[root.clone()],"report",1,&AtomicU64::new(2));
        assert!(cancelled.paths.is_empty());
        let found=search(&[root.clone()],"report",1,&AtomicU64::new(1));
        assert_eq!(found.paths.len(),32); assert!(found.truncated);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn folder_selection_requires_absolute_owned_choices_and_is_bounded() {
        assert!(parse_roots("relative/path").is_err());
        assert!(parse_roots("").is_err());
        assert_eq!(parse_roots("/tmp; /tmp; /var/tmp").unwrap(),vec![std::path::PathBuf::from("/tmp"),std::path::PathBuf::from("/var/tmp")]);
    }
}
