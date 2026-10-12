//! Durable repository evidence for stopped workers. Capture does not authorize
//! restoration, task-result acceptance, or source deletion.
use std::{collections::BTreeMap,ffi::OsStr,io::Write,os::unix::fs::MetadataExt,path::{Path,PathBuf},time::Instant};
use anyhow::{Context,Result,ensure};
use serde::{Deserialize,Serialize};
use sha2::{Digest,Sha256};
use crate::{domain::*,runner::Cancellation,source_tree::{self,Budget,Control,Directory}};
mod git;
mod outputs;
mod output_reader;
pub use output_reader::{load_outputs,load_binding_outputs,load_binding_output_rows,VerifiedOutputs};
pub use outputs::OutputManifest;
pub use crate::domain::{OutputOmission,OutputOmissionReason};
pub(crate) use outputs::capture_outputs_held;
pub use git::{Archive as GitArchive,State as GitState};

#[derive(Debug,Clone,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub path:String,pub directory:bool,pub executable:bool,pub bytes:u64,pub sha256:String,
    #[serde(default,skip_serializing_if="is_false")]
    pub symlink:bool,
}
fn is_false(value:&bool)->bool {!*value}
#[derive(Debug,Clone,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrackedFile {pub path:String,pub bytes:u64,pub blob:String,pub mode:String}
#[derive(Debug,Clone,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrackedOmission {pub path:String,pub bytes:u64,pub blob:String,pub mode:String,pub reason:OutputOmissionReason}
#[derive(Debug,Clone,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub version:u32,pub scope:String,pub attempt:AttemptId,pub worktree:WorktreeReceipt,pub entries:Vec<Entry>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub git:Option<GitArchive>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub git_identity:Option<GitState>,
    #[serde(default,skip_serializing_if="Vec::is_empty")]
    pub excluded_ignored_paths:Vec<String>,
    #[serde(default,skip_serializing_if="Vec::is_empty")]
    pub omissions:Vec<TrackedOmission>,
    #[serde(default,skip_serializing_if="Vec::is_empty")]
    pub tracked_files:Vec<TrackedFile>,
}
#[derive(Debug,Clone,Serialize)]
pub struct Snapshot {pub digest:String,pub directory:PathBuf,pub manifest:Manifest}
fn hash(bytes:&[u8])->String {format!("{:x}",Sha256::digest(bytes))}

#[derive(Debug)]
pub(crate) struct CaptureLimit(pub CaptureIncomplete,pub &'static str);
impl std::fmt::Display for CaptureLimit {
    fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result {f.write_str(self.1)}
}
impl std::error::Error for CaptureLimit {}
fn structure_limit(error:anyhow::Error)->anyhow::Error {
    if error.chain().any(|e|e.is::<crate::worktree_preparation::GitCaptureLimit>()) {
        CaptureLimit(CaptureIncomplete::StructureLimit,"checkout Git inventory exceeds preservation bounds").into()
    }else{error}
}

// Some exclusions select worktree semantics (including skipping the .git pointer);
// Output exclusions are independent and never select Git/link semantics.
fn scan(directory:&Directory,relative:&Path,budget:&mut Budget,entries:&mut Vec<Entry>,blobs:&mut BTreeMap<String,Vec<u8>>,ignored:Option<&[PathBuf]>,exclusions:&[PathBuf])->Result<()> {
    let depth=relative.components().count();
    let skip_git=ignored.is_some();
    let before=directory.metadata()?;
    for name in if exclusions.is_empty(){directory.names(budget)?}else{directory.names_bounded(budget,30_000)?} {
        if skip_git && relative.as_os_str().is_empty() && name==".git" {continue;}
        let path=relative.join(&name);
        if ignored.is_some_and(|paths|paths.iter().any(|excluded|path.starts_with(excluded))) {continue;}
        if exclusions.iter().any(|excluded|path.starts_with(excluded)) {continue;}
        budget.entry(depth)?;
        let text=path.to_str().context("checkout snapshot paths must be UTF-8")?.to_owned();
        ensure!(text.len()<=4096,"checkout snapshot path exceeds bounds");
        if skip_git && directory.kind(&name)?==Some(source_tree::NodeKind::Link) {
            let bytes=directory.link_bytes(&name,budget)?;let digest=hash(&bytes);
            entries.push(Entry{path:text,directory:false,executable:false,bytes:bytes.len() as u64,sha256:digest.clone(),symlink:true});
            blobs.entry(digest).or_insert(bytes);continue;
        }
        let file=directory.child(&name)?;
        let metadata=file.metadata()?;
        if metadata.is_dir() {
            entries.push(Entry{symlink:false,path:text,directory:true,executable:false,bytes:0,sha256:String::new()});
            scan(&Directory::from_file(file)?,&path,budget,entries,blobs,ignored,exclusions)?;
        } else {
            budget.size(&file)?;
            let mut file=file;let mut bytes=Vec::new();let mut buffer=[0u8;65536];
            loop {let n=budget.read(&mut file,&mut buffer)?;if n==0 {break;}bytes.extend_from_slice(&buffer[..n]);}
            source_tree::unchanged(&file,&metadata)?;
            let digest=hash(&bytes);
            entries.push(Entry{symlink:false,path:text,directory:false,executable:metadata.mode()&0o111!=0,bytes:bytes.len() as u64,sha256:digest.clone()});
            blobs.entry(digest).or_insert(bytes);
        }
    }
    directory.unchanged(&before)?;
    budget.check()
}

// Worker-controlled ignore rules can only shrink preserved working files:
// ignored toolchains, caches and build output are by definition not work.
fn ignored_paths(git:&crate::worktree_preparation::Git,path:&Path)->Result<Vec<PathBuf>> {
    let raw=git.capture(path,&["ls-files","--others","--ignored","--exclude-standard","--directory","-z"],None,4*1024*1024).map_err(structure_limit)?;
    ensure!(raw.is_empty() || raw.last()==Some(&0),"incomplete ignored path inventory");
    let mut paths=Vec::new();
    for record in raw.split(|b|*b==0).filter(|p|!p.is_empty()) {
        let text=std::str::from_utf8(record)?.trim_end_matches('/');
        let path=PathBuf::from(text);
        ensure!(!text.is_empty() && text.len()<=4096 && path.components().all(|c|matches!(c,std::path::Component::Normal(_))),"invalid ignored checkout path");
        if paths.len()>=10_000 {return Err(CaptureLimit(CaptureIncomplete::StructureLimit,"ignored checkout inventory exceeds bounds").into());}
        paths.push(path);
    }
    Ok(paths)
}

fn tracked_file(source:&Directory,path:&Path)->Result<Option<std::fs::File>> {
    let mut parent=source.directory(Path::new(""))?;
    for part in path.parent().unwrap_or(Path::new("")).components() {
        let std::path::Component::Normal(name)=part else {anyhow::bail!("invalid tracked parent");};
        if parent.kind(name)?!=Some(source_tree::NodeKind::Directory) {return Ok(None);}
        parent=parent.directory(Path::new(name))?;
    }
    let leaf=path.file_name().context("tracked leaf missing")?;
    if parent.kind(leaf)?!=Some(source_tree::NodeKind::File) {return Ok(None);}
    Ok(Some(parent.child(leaf)?))
}

type TrackedInventory=Vec<(String,u64,bool)>;
type WorkingEvidence=(Vec<TrackedFile>,Vec<TrackedOmission>);
type WorkingSource<'a>=(&'a Directory,&'a WorktreeReceipt,&'a [PathBuf]);

fn tracked_inventory(git:&crate::worktree_preparation::Git,source:&Directory,receipt:&WorktreeReceipt,control:&Control)->Result<TrackedInventory> {
    let path=Path::new(&receipt.plan.path);
    let raw=git.capture(path,&["ls-files","-z"],None,4*1024*1024).map_err(structure_limit)?;
    ensure!(raw.is_empty() || raw.last()==Some(&0),"incomplete tracked inventory");
    let mut tracked=std::collections::BTreeSet::new();
    let mut candidates=Vec::new();
    for name in raw.split(|b|*b==0).filter(|b|!b.is_empty()) {
        let name=std::str::from_utf8(name)?;
        let relative=Path::new(name);
        ensure!(name.len()<=4096 && relative.components().all(|c|matches!(c,std::path::Component::Normal(_))),"invalid tracked path");
        control.check()?;
        if !tracked.insert(relative.to_owned()) {continue;}
        if tracked.len()>source_tree::ENTRY_LIMIT {return Err(source_tree::Limit::Entries.into());}
        if relative.components().count()>source_tree::DEPTH_LIMIT {return Err(source_tree::Limit::Depth.into());}
        let Some(file)=tracked_file(source,relative)? else {continue;};let metadata=file.metadata()?;
        ensure!(metadata.is_file() && metadata.nlink()==1,"tracked source must be a regular file with one link");
        candidates.push((name.to_owned(),metadata.len(),metadata.mode()&0o111!=0));
    }
    Ok(candidates)
}

fn tracked_selections(git:&crate::worktree_preparation::Git,sources:&[WorkingSource<'_>],control:&Control)->Result<Vec<WorkingEvidence>> {
    let mut inventories=Vec::new();let mut priority=Vec::new();
    let mut untracked_budget=control.budget();let mut untracked_bytes=0;
    for (repository,(source,receipt,ignored)) in sources.iter().enumerate() {
        let candidates=tracked_inventory(git,source,receipt,control)?;
        if priority.len()+candidates.len()>source_tree::ENTRY_LIMIT {return Err(source_tree::Limit::Entries.into());}
        // Reserve all non-ignored untracked bytes across the approved worktrees
        // before selecting any tracked byte copies. This scan remains fail-hard.
        let exclusions=candidates.iter().map(|c|PathBuf::from(&c.0)).collect::<Vec<_>>();
        let mut entries=Vec::new();let mut blobs=BTreeMap::new();
        scan(source,Path::new(""),&mut untracked_budget,&mut entries,&mut blobs,Some(ignored),&exclusions)?;
        untracked_bytes+=entries.iter().map(|e|e.bytes).sum::<u64>();
        for (index,(name,bytes,_)) in candidates.iter().enumerate() {
            let binary=matches!(Path::new(name).extension().and_then(|e|e.to_str()),Some("uasset"|"umap"|"png"|"jpg"|"jpeg"|"zip"|"bin"|"so"|"dll"|"exe"|"mp4"|"wav"));
            priority.push((repository,index,binary,*bytes,name.clone()));
        }
        inventories.push(candidates);
    }
    priority.sort_by(|a,b|(a.2,a.3,&a.4,a.0).cmp(&(b.2,b.3,&b.4,b.0)));
    let mut remaining=source_tree::BYTE_LIMIT.checked_sub(untracked_bytes).ok_or(source_tree::Limit::Bytes)?;
    let mut selected=vec![std::collections::BTreeSet::new();sources.len()];
    for (repository,index,_,bytes,_) in priority {
        if bytes<=remaining {remaining-=bytes;selected[repository].insert(index);}
    }
    sources.iter().zip(inventories).zip(selected).map(|(((source,receipt,_),candidates),selected)|
        tracked_evidence(git,source,receipt,candidates,&selected,control)).collect()
}

fn tracked_evidence(git:&crate::worktree_preparation::Git,source:&Directory,receipt:&WorktreeReceipt,candidates:TrackedInventory,selected:&std::collections::BTreeSet<usize>,control:&Control)->Result<WorkingEvidence> {
    let path=Path::new(&receipt.plan.path);
    let mut pending=Vec::new();
    for (index,(name,bytes,executable)) in candidates.into_iter().enumerate() {
        control.check()?;
        let omitted=!selected.contains(&index);
        let relative=Path::new(&name);let parent=source.directory(relative.parent().unwrap())?;
        let file=parent.child(relative.file_name().unwrap())?;let before=file.metadata()?;
        ensure!(before.len()==bytes && (before.mode()&0o111!=0)==executable,"tracked source changed during inventory");
        pending.push((name,bytes,executable,before,omitted));
    }
    // Bounded descriptor batches stream bytes through Git. Worker-controlled
    // path replacement cannot redirect hashing to a symlink or outside file.
    let mut omissions=Vec::new();let mut tracked_files=Vec::new();
    let batch_size=crate::runner::INPUT_DESCRIPTOR_LIMIT;
    for batch in pending.chunks(batch_size) {
        control.check()?;
        let files=batch.iter().map(|(name,_,_,before,_)|->Result<std::fs::File>{
            let file=tracked_file(source,Path::new(name))?.context("tracked file disappeared before hashing")?;
            source_tree::unchanged(&file,before)?;Ok(file)
        }).collect::<Result<Vec<_>>>()?;
        let raw=String::from_utf8(git.hash_files(path,&files)?)?;
        let identities=raw.lines().collect::<Vec<_>>();
        ensure!(identities.len()==batch.len(),"incomplete tracked blob identities");
        for (((name,bytes,executable,before,omitted),file),blob) in batch.iter().zip(&files).zip(identities) {
            ensure!(matches!(blob.len(),40|64) && blob.bytes().all(|b|b.is_ascii_hexdigit()),"invalid tracked blob identity");
            source_tree::unchanged(file,before)?;
            let mode=if *executable{"100755"}else{"100644"}.to_owned();
            tracked_files.push(TrackedFile{path:name.clone(),bytes:*bytes,blob:blob.into(),mode:mode.clone()});
            if *omitted {omissions.push(TrackedOmission{path:name.clone(),bytes:*bytes,blob:blob.into(),mode,reason:OutputOmissionReason::ByteLimit});}
        }
    }
    omissions.sort_by(|a,b|a.path.cmp(&b.path));tracked_files.sort_by(|a,b|a.path.cmp(&b.path));Ok((tracked_files,omissions))
}

fn exact_file(directory:&Directory,name:&str,bytes:&[u8],control:&Control)->Result<bool> {
    control.check()?;
    let Some(mut file)=directory.optional(OsStr::new(name))? else {return Ok(false);};
    let before=file.metadata()?;let mut budget=control.budget();budget.size(&file)?;
    ensure!(before.len()==bytes.len() as u64,"retained checkout snapshot size differs");
    let mut read=Vec::new();let mut buffer=[0u8;65536];
    loop {let n=budget.read(&mut file,&mut buffer)?;if n==0 {break;}read.extend_from_slice(&buffer[..n]);}
    source_tree::unchanged(&file,&before)?;
    ensure!(read==bytes,"retained checkout snapshot bytes differ");
    Ok(true)
}
fn retain(directory:&Directory,name:&str,bytes:&[u8],control:&Control)->Result<()> {
    if !exact_file(directory,name,bytes,control)? {
        directory.write_atomic(OsStr::new(name),|file|{control.check()?;file.write_all(bytes)?;control.check()})?;
    }
    ensure!(exact_file(directory,name,bytes,control)?,"checkout snapshot publication missing");
    Ok(())
}

/// Explicit capture after exact termination. No task transition, resource
/// release, Git mutation, source removal or verified-result receipt is produced.
pub fn capture_stopped_files(project:&Path,attempt:&AttemptId,expected_head:u64,deadline:Instant,cancellation:Cancellation)->Result<Vec<Snapshot>> {
    capture(project,attempt,expected_head,deadline,cancellation,false)
}
/// Preserve working files, reachable approved/current history, and indexed
/// objects plus exact index bytes. No restore or deletion authority is granted.
pub fn capture_stopped_repository(project:&Path,attempt:&AttemptId,expected_head:u64,deadline:Instant,cancellation:Cancellation)->Result<Vec<Snapshot>> {
    capture(project,attempt,expected_head,deadline,cancellation,true)
}
fn capture(project:&Path,attempt:&AttemptId,expected_head:u64,deadline:Instant,cancellation:Cancellation,include_git:bool)->Result<Vec<Snapshot>> {
    let control=Control{deadline,cancellation};control.check()?;
    let project=project.canonicalize()?;
    let guard=crate::execution_guard::RootGuard::exclusive(project.parent().context("project root missing")?)?;
    control.check()?;
    let mut db=crate::migration::open_active_controlled(&project,crate::store::controlled::ReadControl::new(deadline,control.cancellation.clone()))?;
    let state=db.read_snapshot(Some(expected_head))?;
    ensure!(state.control.as_ref().context("project control missing")?.state!=ProjectState::Archived,"archived project cannot capture new checkout snapshots");
    let current=state.attempts.iter().find(|a|&a.id==attempt).context("attempt missing")?;
    ensure!(current.termination_observed && !current.retains_capacity(),"checkout capture requires exact attempt termination");
    let record=state.attempt_inputs.iter().find(|r|&r.attempt==attempt).context("retained launch inputs missing")?;
    capture_held(&project,&state,record,&guard,&control,include_git)
}

// Trusted termination calls this only after proving native quiescence, before
// changing canonical disposition. Public capture still requires terminal state.
pub(crate) fn capture_held(project:&Path,state:&crate::domain::Snapshot,record:&AttemptInputRecord,guard:&crate::execution_guard::RootGuard,control:&Control,include_git:bool)->Result<Vec<Snapshot>> {
    capture_events_held(project,&state.events,record,guard,control,include_git)
}
pub(crate) fn capture_events_held(project:&Path,events:&[Event],record:&AttemptInputRecord,guard:&crate::execution_guard::RootGuard,control:&Control,include_git:bool)->Result<Vec<Snapshot>> {
    control.check()?;
    let deadline=control.deadline;
    let proof=crate::worktree_preparation::pin_started_events_held(project,events,record,deadline,control.cancellation.clone())?;
    if record.inputs.repositories.is_empty() {return Ok(vec![]);}
    let event=events.iter().find(|e|e.kind=="runtime.worktrees_ready" && e.entity==record.operation.as_str()).context("ready worktree evidence missing")?;
    let receipts:Vec<WorktreeReceipt>=serde_json::from_value(event.payload.clone())?;
    capture_receipts(project,record,proof,receipts,guard,control,include_git)
}
fn capture_receipts(project:&Path,record:&AttemptInputRecord,proof:crate::worktree_preparation::WorktreeProof,receipts:Vec<WorktreeReceipt>,guard:&crate::execution_guard::RootGuard,control:&Control,include_git:bool)->Result<Vec<Snapshot>> {
    let attempt=&record.attempt;let deadline=control.deadline;
    let git=crate::worktree_preparation::Git{deadline,cancellation:control.cancellation.clone(),locks:guard.inherit()?};
    let mut remaining=source_tree::BYTE_LIMIT as usize;
    let mut sources=Vec::new();
    for receipt in receipts {
        let source=Directory::open(Path::new(&receipt.plan.path))?;
        let ignored=ignored_paths(&git,Path::new(&receipt.plan.path))?;
        sources.push((source,receipt,ignored));
    }
    let selections=tracked_selections(&git,&sources.iter().map(|(source,receipt,ignored)|
        (source,receipt,ignored.as_slice())).collect::<Vec<_>>(),control)?;
    let mut budget=control.budget();let mut captured=Vec::new();
    for ((source,receipt,ignored),(tracked_files,omissions)) in sources.into_iter().zip(selections) {
        let mut entries=Vec::new();let mut blobs=BTreeMap::new();
        let exclusions=omissions.iter().map(|o|PathBuf::from(&o.path)).collect::<Vec<_>>();
        scan(&source,Path::new(""),&mut budget,&mut entries,&mut blobs,Some(&ignored),&exclusions)?;
        source.matches_path(Path::new(&receipt.plan.path))?;
        let git_identity=if include_git && !omissions.is_empty() {Some(git::observe(&git,&receipt,control,&mut remaining,&mut blobs)?)}else{None};
        let archive=if include_git && omissions.is_empty() {Some(git::capture(&git,&receipt,control,&mut remaining,&mut blobs)?)}else{None};
        captured.push((source,Manifest{tracked_files:if omissions.is_empty(){vec![]}else{tracked_files},git_identity,omissions:omissions.clone(),version:if !omissions.is_empty(){5}else if !ignored.is_empty(){4}else if entries.iter().any(|e|e.symlink){3}else if include_git{2}else{1},scope:if include_git{"repository_state"}else{"working_files"}.into(),attempt:attempt.clone(),worktree:receipt,entries,git:archive,excluded_ignored_paths:ignored.iter().map(|p|p.to_str().expect("validated UTF-8 ignored path").to_owned()).collect()},blobs,ignored));
    }
    // Verify again using bytes, not Git status or a size/mtime shortcut.
    let repeated=tracked_selections(&git,&captured.iter().map(|(source,manifest,_,ignored)|
        (source,&manifest.worktree,ignored.as_slice())).collect::<Vec<_>>(),control)?;
    let mut budget=control.budget();
    for ((source,manifest,_,ignored),(tracked_files,omissions)) in captured.iter().zip(repeated) {
        let mut entries=Vec::new();let mut blobs=BTreeMap::new();
        ensure!(omissions==manifest.omissions && (omissions.is_empty() || tracked_files==manifest.tracked_files),"tracked identities changed during capture");
        let exclusions=omissions.iter().map(|o|PathBuf::from(&o.path)).collect::<Vec<_>>();
        scan(source,Path::new(""),&mut budget,&mut entries,&mut blobs,Some(ignored),&exclusions)?;
        ensure!(entries==manifest.entries,"checkout changed during capture");
        source.matches_path(Path::new(&manifest.worktree.plan.path))?;
        if let Some(identity)=&manifest.git_identity {ensure!(&git::observe(&git,&manifest.worktree,control,&mut (source_tree::BYTE_LIMIT as usize),&mut BTreeMap::new())?==identity,"Git identity changed during preservation");}
        if let Some(archive)=&manifest.git {git::verify(&git,&manifest.worktree,&control,archive)?;}
    }
    proof.check()?;control.check()?;
    let state_dir=Directory::open(&project.join(".state"))?;
    let root=state_dir.create_dir(OsStr::new("worktree-file-snapshots"))?;
    let parent=root.create_dir(OsStr::new(attempt.as_str()))?;
    // Bound cumulative retained snapshots. Never reclaim accepted or incomplete
    // evidence implicitly to make space for another capture.
    let inventory=parent.names(&control.budget())?;
    ensure!(inventory.len()<=1024,"checkout snapshot inventory is full");
    let mut results=Vec::new();
    for (_,manifest,blobs,_) in captured {
        let bytes=serde_json::to_vec(&manifest)?;
        if bytes.len()>4*1024*1024 {return Err(CaptureLimit(CaptureIncomplete::StructureLimit,"checkout manifest exceeds bounds").into());}
        let digest=hash(&bytes);
        if parent.kind(OsStr::new(&digest))?.is_none() {ensure!(inventory.len()+results.len()<1024,"checkout snapshot inventory is full");}
        let directory=parent.create_dir(OsStr::new(&digest))?;
        let names=directory.names(&control.staging_cleanup_budget())?;
        ensure!(names.iter().filter(|name|name.as_encoded_bytes().starts_with(b".live-")).count()<16,"checkout snapshot temporary inventory is full");
        for (id,body) in blobs {retain(&directory,&id,&body,&control)?;}
        // Manifest is the completion marker and is published only after every
        // content-addressed file has been persisted and read back successfully.
        if let Some(identity)=&manifest.git_identity {ensure!(&git::observe(&git,&manifest.worktree,control,&mut (source_tree::BYTE_LIMIT as usize),&mut BTreeMap::new())?==identity,"Git identity changed during preservation");}
        if let Some(archive)=&manifest.git {git::verify(&git,&manifest.worktree,&control,archive)?;}
        proof.check()?;control.check()?;
        state_dir.matches_path(&project.join(".state"))?;
        root.matches_path(&project.join(".state/worktree-file-snapshots"))?;
        parent.matches_path(&project.join(".state/worktree-file-snapshots").join(attempt.as_str()))?;
        let published=project.join(".state/worktree-file-snapshots").join(attempt.as_str()).join(&digest);
        directory.matches_path(&published)?;
        retain(&directory,"manifest.json",&bytes,&control)?;
        directory.matches_path(&published)?;
        results.push(Snapshot{digest,directory:published,manifest});
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    fn entries(path:&Path,control:&Control)->Result<Vec<Entry>> {
        let mut result=Vec::new();let mut blobs=BTreeMap::new();
        scan(&Directory::open(path)?,Path::new(""),&mut control.budget(),&mut result,&mut blobs,Some(&[]),&[])?;
        Ok(result)
    }
    #[test]
    fn file_scan_detects_same_size_same_mtime_edits_and_honors_control() {
        let root=tempfile::tempdir().unwrap();let path=root.path().join("result");
        fs::write(&path,b"first").unwrap();let before=fs::metadata(&path).unwrap().modified().unwrap();
        let first=entries(root.path(),&Control::default()).unwrap();
        fs::write(&path,b"other").unwrap();
        fs::File::options().write(true).open(&path).unwrap().set_times(fs::FileTimes::new().set_modified(before)).unwrap();
        assert_ne!(first,entries(root.path(),&Control::default()).unwrap());
        let cancelled=Control::default();cancelled.cancellation.cancel();
        assert!(entries(root.path(),&cancelled).is_err());
        assert!(entries(root.path(),&Control{deadline:Instant::now(),cancellation:Default::default()}).is_err());
    }
    #[test]
    fn special_sources_are_rejected_without_following_or_waiting() {
        let root=tempfile::tempdir().unwrap();let outside=tempfile::tempdir().unwrap();
        fs::write(outside.path().join("private"),b"unchanged").unwrap();
        let path=root.path().join("entry");
        std::os::unix::fs::symlink(outside.path(),&path).unwrap();
        let captured=entries(root.path(),&Control::default()).unwrap();assert_eq!(captured.len(),1);
        assert!(captured[0].symlink);assert_eq!(captured[0].sha256,hash(outside.path().as_os_str().as_encoded_bytes()));
        assert_eq!(fs::read(outside.path().join("private")).unwrap(),b"unchanged");fs::remove_file(&path).unwrap();
        let name=std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
        // SAFETY: live NUL-terminated pathname and valid mode.
        assert_eq!(unsafe{libc::mkfifo(name.as_ptr(),0o600)},0);
        assert!(entries(root.path(),&Control::default()).is_err());fs::remove_file(&path).unwrap();
        fs::hard_link(outside.path().join("private"),&path).unwrap();
        assert!(entries(root.path(),&Control::default()).is_err());
        assert_eq!(fs::read(outside.path().join("private")).unwrap(),b"unchanged");
    }
    #[test]
    fn interrupted_file_publication_resumes_without_replacing_conflicting_bytes() {
        let root=tempfile::tempdir().unwrap();let directory=Directory::open(root.path()).unwrap();
        let bytes=b"partial worker result";let id=hash(bytes);
        retain(&directory,&id,bytes,&Control::default()).unwrap();
        assert!(!root.path().join("manifest.json").exists());
        retain(&directory,&id,bytes,&Control::default()).unwrap();
        fs::write(root.path().join(&id),b"corrupt").unwrap();
        assert!(retain(&directory,&id,bytes,&Control::default()).is_err());
        assert_eq!(fs::read(root.path().join(&id)).unwrap(),b"corrupt");
    }
}


pub(crate) fn capture_preparation_held(project:&Path,events:&[Event],record:&AttemptInputRecord,intent:&WorktreeCreation,guard:&crate::execution_guard::RootGuard,control:&Control)->Result<(Vec<PreparationSnapshotReference>,AttemptOutputReference)> {
    control.check()?;
    let git=crate::worktree_preparation::Git{deadline:control.deadline,cancellation:control.cancellation.clone(),locks:guard.inherit()?};
    let(proof,receipts,missing)=crate::worktree_preparation::pin_preparation_held(&git,events,intent)?;
    let captured=capture_receipts(project,record,proof,receipts,guard,control,true)?;
    let output=capture_outputs_held(project,record,control)?;
    for plan in &missing {crate::worktree_preparation::verify_uncreated(&git,plan)?;}
    let references=intent.plans.iter().map(|plan|->Result<PreparationSnapshotReference>{
        let digest=if missing.contains(plan){None}else{Some(captured.iter().find(|s|&s.manifest.worktree.plan==plan).context("preparation snapshot coverage missing")?.digest.clone())};
        Ok(PreparationSnapshotReference{plan:plan.clone(),digest})
    }).collect::<Result<Vec<_>>>()?;
    control.check()?;Ok((references,output))
}
