//! Capture the dedicated output directory after native quiescence. Partial
//! reports are evidence candidates; no report or task success is required.
use super::*;
use crate::source_tree::NodeKind;

#[derive(Debug,Clone,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputManifest {
    pub version:u32,pub attempt:AttemptId,pub source:String,pub entries:Vec<Entry>,
    #[serde(default,skip_serializing_if="Vec::is_empty")]
    pub omissions:Vec<OutputOmission>,
}

pub(crate) fn capture_outputs_held(project:&Path,record:&AttemptInputRecord,control:&Control)->Result<AttemptOutputReference> {
    control.check()?;
    let path=worker_output_path(&record.inputs,&record.attempt).map_err(anyhow::Error::msg)?;
    let state=Directory::open(&project.join(".state"))?;
    let absent=||AttemptOutputReference{omitted_entries:0,source:path.clone(),digest:None};
    match state.kind(OsStr::new("worker-output"))? {
        None=>{state.matches_path(&project.join(".state"))?;control.check()?;return Ok(absent());},
        Some(NodeKind::Directory)=>{},_=>anyhow::bail!("worker output parent is not a real directory"),
    }
    let output_parent=state.directory(Path::new("worker-output"))?;
    match output_parent.kind(OsStr::new(record.attempt.as_str()))? {
        None=>{
            output_parent.matches_path(&project.join(".state/worker-output"))?;
            state.matches_path(&project.join(".state"))?;control.check()?;return Ok(absent());
        },
        Some(NodeKind::Directory)=>{},_=>anyhow::bail!("worker output source is not a real directory"),
    }
    let source=output_parent.directory(Path::new(record.attempt.as_str()))?;
    let inventory=inventory(&source,Path::new(""),control,0)?;
    let mut priority=inventory.clone();
    priority.sort_by(|a,b|(a.path!="report.md",a.directory,a.bytes,&a.path).cmp(&(b.path!="report.md",b.directory,b.bytes,&b.path)));
    let mut selected=std::collections::BTreeSet::new();let mut remaining=source_tree::BYTE_LIMIT;
    let mut omissions=Vec::new();
    for item in &priority {
        let path=Path::new(&item.path);
        let needed:Vec<_>=path.ancestors().filter(|p|!p.as_os_str().is_empty()).map(Path::to_path_buf).filter(|p|!selected.contains(p)).collect();
        let reason=if selected.len()+needed.len()>source_tree::ENTRY_LIMIT {Some(OutputOmissionReason::EntryLimit)}
            else if item.bytes>remaining {Some(OutputOmissionReason::ByteLimit)}else{None};
        if let Some(reason)=reason {omissions.push(OutputOmission{path:item.path.clone(),bytes:item.bytes,reason});}
        else {remaining-=item.bytes;selected.extend(needed);}
    }
    omissions.retain(|o|!selected.contains(Path::new(&o.path)));
    omissions.sort_by(|a,b|a.path.cmp(&b.path));
    let exclusions:Vec<_>=omissions.iter().map(|o|PathBuf::from(&o.path)).collect();
    let mut entries=Vec::new();let mut blobs=BTreeMap::new();
    scan(&source,Path::new(""),&mut control.budget(),&mut entries,&mut blobs,None,&exclusions)?;
    let mut repeated=Vec::new();
    scan(&source,Path::new(""),&mut control.budget(),&mut repeated,&mut BTreeMap::new(),None,&exclusions)?;
    ensure!(entries==repeated && inventory==self::inventory(&source,Path::new(""),control,0)?,"worker outputs changed during preservation");
    source.matches_path(Path::new(&path))?;
    // Gate release pre-creates the directory (the sandbox binds only existing
    // paths writable); an attempt that wrote nothing into it produced no outputs.
    if entries.is_empty() && omissions.is_empty() {state.matches_path(&project.join(".state"))?;control.check()?;return Ok(absent());}
    let manifest=OutputManifest{version:2,attempt:record.attempt.clone(),source:path.clone(),entries,omissions:omissions.clone()};
    let bytes=serde_json::to_vec(&manifest)?;ensure!(bytes.len()<=4*1024*1024,"worker output manifest exceeds bounds");
    let digest=hash(&bytes);
    let root=state.create_dir(OsStr::new("worker-output-snapshots"))?;
    let parent=root.create_dir(OsStr::new(record.attempt.as_str()))?;
    let inventory=parent.names(&control.budget())?;
    ensure!(inventory.len()<=1024 && (inventory.len()<1024 || parent.kind(OsStr::new(&digest))?.is_some()),"worker output snapshot inventory is full");
    let directory=parent.create_dir(OsStr::new(&digest))?;
    let names=directory.names(&control.staging_cleanup_budget())?;
    ensure!(names.iter().filter(|n|n.as_encoded_bytes().starts_with(b".live-")).count()<16,"worker output temporary inventory is full");
    for (id,body) in blobs {retain(&directory,&id,&body,control)?;}
    source.matches_path(Path::new(&path))?;control.check()?;
    let published=project.join(".state/worker-output-snapshots").join(record.attempt.as_str()).join(&digest);
    state.matches_path(&project.join(".state"))?;
    root.matches_path(&project.join(".state/worker-output-snapshots"))?;
    parent.matches_path(published.parent().unwrap())?;directory.matches_path(&published)?;
    retain(&directory,"manifest.json",&bytes,control)?;
    directory.matches_path(&published)?;control.check()?;
    Ok(AttemptOutputReference{omitted_entries:omissions.len(),source:path,digest:Some(digest)})
}

// Metadata inventory has a separate bounded envelope; no source bytes are read.
fn inventory(directory:&Directory,relative:&Path,control:&Control,depth:usize)->Result<Vec<Entry>> {
    ensure!(depth<=source_tree::DEPTH_LIMIT,"worker output nesting exceeds bounds");
    let before=directory.metadata()?;let mut result=Vec::new();
    for name in directory.names_bounded(&control.budget(),30_000)? {
        control.check()?;let path=relative.join(&name);let text=path.to_str().context("worker output paths must be UTF-8")?.to_owned();
        ensure!(text.len()<=4096,"worker output path exceeds bounds");
        let file=directory.child(&name)?;let metadata=file.metadata()?;
        let is_dir=metadata.is_dir();
        ensure!(is_dir||metadata.nlink()==1,"worker output file must have one link");
        result.push(Entry{path:text,directory:is_dir,executable:!is_dir&&metadata.mode()&0o111!=0,bytes:if is_dir{0}else{metadata.len()},sha256:String::new(),symlink:false});
        if is_dir {result.extend(inventory(&Directory::from_file(file)?,&path,control,depth+1)?);}
        ensure!(result.len()<=30_000,"worker output metadata inventory exceeds bounds");
    }
    directory.unchanged(&before)?;Ok(result)
}
