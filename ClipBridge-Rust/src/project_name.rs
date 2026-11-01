//! Read-only CapCut project name inference for clipboard file naming.
//! No CapCut project files are changed. A manual override is available when
//! the editor title hides the name or multiple drafts were edited recently.
use std::{env,fs,path::{Path,PathBuf},time::{Duration,SystemTime},ptr::null_mut};
use windows_sys::Win32::{Foundation::*,System::Threading::*,UI::WindowsAndMessaging::*};

pub struct ProjectLabel {
    pub name:String,
    pub source:&'static str,
}

pub fn sanitize_project(raw:&str)->String {
    let mut out=String::new();
    let mut last_sep=false;
    for ch in raw.trim().chars() {
        if out.chars().count()>=72 {break;}
        let invalid=ch.is_control()||matches!(ch,'/'|'\\'|':'|'*'|'?'|'"'|'<'|'>'|'|');
        if invalid||ch.is_whitespace(){
            if !last_sep && !out.is_empty(){out.push('_');}
            last_sep=true;
        }else{
            out.push(ch);last_sep=false;
        }
    }
    let out=out.trim_matches(&[' ','_','.'][..]).to_string();
    if out.is_empty(){return "CapCutProject".into();}
    let upper=out.to_ascii_uppercase();
    let reserved=["CON","PRN","AUX","NUL","COM1","COM2","COM3","COM4","LPT1","LPT2","LPT3"];
    if reserved.contains(&upper.as_str()) {format!("Project_{out}")}else{out}
}

fn from_window_title(title:&str)->Option<String> {
    let trimmed=title.trim();
    if trimmed.is_empty(){return None;}
    let lower=trimmed.to_lowercase();
    let program=if lower.contains("capcut") {"capcut"}
        else if lower.contains("jianying") {"jianying"}
        else {return None};
    if lower==program || lower=="capcut pro" || lower=="jianyingpro"{
        return None;
    }
    // Do not assume "CapCut" alone identifies a project. Require an explicit
    // title separator and treat the other side as the draft's display name.
    for sep in [" - "," — "," – "," | "," • "] {
        if let Some((left,right))=trimmed.split_once(sep){
            let l=left.to_lowercase();let r=right.to_lowercase();
            let candidate=if l.contains(program) && !r.contains(program){right}
                else if r.contains(program) && !l.contains(program){left}
                else {continue};
            let name=sanitize_project(candidate);
            if name!="CapCutProject" && name.len()>1{return Some(name);}
        }
    }
    None
}
struct WindowCandidates(Vec<String>);
unsafe extern "system" fn collect_windows(hwnd:HWND,l:LPARAM)->i32 {
    if IsWindowVisible(hwnd)==0 {return 1;}
    let mut pid=0;
    GetWindowThreadProcessId(hwnd,&mut pid);
    if pid==0{return 1;}
    let proc=OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION,0,pid);
    if proc.is_null(){return 1;}
    let mut path=[0u16;512];
    let mut size=path.len() as u32;
    let ok=QueryFullProcessImageNameW(proc,0,path.as_mut_ptr(),&mut size)!=0;
    CloseHandle(proc);
    if !ok{return 1;}
    let exe=String::from_utf16_lossy(&path[..size as usize]).to_ascii_lowercase();
    if !exe.ends_with("\\capcut.exe") && !exe.ends_with("\\jianyingpro.exe") {
        return 1;
    }
    let mut title=[0u16;512];
    let len=GetWindowTextW(hwnd,title.as_mut_ptr(),title.len() as i32);
    if len>0 {
        let list=&mut *(l as *mut WindowCandidates);
        list.0.push(String::from_utf16_lossy(&title[..len as usize]));
    }
    1
}
unsafe fn window_name()->Option<String> {
    let mut found=WindowCandidates(Vec::new());
    EnumWindows(Some(collect_windows),&mut found as *mut _ as isize);
    for title in found.0 {
        if let Some(name)=from_window_title(&title){return Some(name);}
    }
    None
}
fn read_draft_name(meta:&Path)->Option<String>{
    let bytes=fs::read(meta).ok()?;
    if bytes.len()>4_000_000 {return None;}
    let value:serde_json::Value=serde_json::from_slice(&bytes).ok()?;
    let name=value.get("draft_name")?.as_str()?.trim();
    if name.is_empty(){None}else{Some(sanitize_project(name))}
}
fn scan_latest_draft()->Option<String> {
    let base=PathBuf::from(env::var_os("LOCALAPPDATA")?);
    let mut found:Vec<(SystemTime,String)>=Vec::new();
    for sub in ["CapCut","JianyingPro"] {
        let root=base.join(sub).join("User Data").join("Projects")
            .join("com.lveditor.draft");
        let Ok(entries)=fs::read_dir(root) else {continue};
        for entry in entries.flatten().take(2500) {
            let path=entry.path();
            if !path.is_dir(){continue;}
            let Some(name)=read_draft_name(&path.join("draft_meta_info.json")) else {
                continue;
            };
            let mut modified=None;
            for content in ["draft_content.json","draft_info.json","template-2.tmp"] {
                if let Ok(mtime)=fs::metadata(path.join(content))
                    .and_then(|m|m.modified()){
                    if modified.is_none_or(|previous|mtime>previous) {
                        modified=Some(mtime);
                    }
                }
            }
            let Some(modified)=modified else {continue};
            if SystemTime::now().duration_since(modified).unwrap_or(Duration::MAX)>
                Duration::from_secs(60*12){continue;}
            found.push((modified,name));
        }
    }
    found.sort_by(|a,b|b.0.cmp(&a.0));
    let first=found.first()?;
    // If two projects have almost identical modification times, an active
    // draft cannot be inferred reliably. Avoid silently selecting a wrong one.
    if let Some(second)=found.get(1) {
        if first.0.duration_since(second.0).unwrap_or_default()<
            Duration::from_secs(45) {
            return None;
        }
    }
    Some(first.1.clone())
}
pub fn detect_project(manual:&str)->ProjectLabel {
    if !manual.trim().is_empty(){
        return ProjectLabel{name:sanitize_project(manual),source:"manual"};
    }
    if let Some(name)=unsafe{window_name()} {
        return ProjectLabel{name,source:"window-title"};
    }
    if let Some(name)=scan_latest_draft() {
        return ProjectLabel{name,source:"recent-draft"};
    }
    ProjectLabel{name:"CapCutProject".into(),source:"fallback-unknown"}
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn sanitizes_windows_paths_and_vietnamese(){
        assert_eq!(sanitize_project("Spa: Nám / Tháng 11"),"Spa_Nám_Tháng_11");
        assert_eq!(sanitize_project("CON"),"Project_CON");
        assert_eq!(sanitize_project(""),"CapCutProject");
    }
    #[test] fn title_requires_actual_name(){
        assert_eq!(from_window_title("CapCut"),None);
        assert_eq!(from_window_title("My Wedding - CapCut"),Some("My_Wedding".into()));
        assert_eq!(from_window_title("CapCut | Spa: Nám"),Some("Spa_Nám".into()));
    }
}
