//! Lightweight visual verification of a newly imported CapCut Media tile.
//! Ported as an independent Rust algorithm, based on the public MIT Jiandan
//! reference (https://github.com/SKYCHENV/Jiandan).
//! Never moves the pointer, writes screenshots, or scans the whole desktop.
use std::{mem::{size_of,zeroed},ptr::null_mut};
use windows_sys::Win32::{Foundation::*,Graphics::Gdi::*,UI::WindowsAndMessaging::*};

#[derive(Debug,Clone,Copy,PartialEq,Eq)]
pub struct MediaBox {pub left:i32,pub top:i32,pub right:i32,pub bottom:i32}
impl MediaBox {
    pub(crate) fn overlaps(self, other:Self)->bool {
        self.left<other.right && other.left<self.right &&
        self.top<other.bottom && other.top<self.bottom
    }
}
pub struct Shot {w:usize,h:usize,bytes:Vec<u8>}
impl Shot {
    pub fn width(&self)->usize {self.w}
    pub fn height(&self)->usize {self.h}
    pub(crate) fn pixel(&self,x:usize,y:usize)->(u8,u8,u8) {
        let p=(y*self.w+x)*4;
        (self.bytes[p+2],self.bytes[p+1],self.bytes[p])
    }
    fn cyan(&self,x:usize,y:usize)->bool {
        let (r,g,b)=self.pixel(x,y);
        r<110 && g>120 && b>120
    }
}
#[cfg(test)]
impl Shot {
    pub(crate) fn for_test(w:usize,h:usize)->Self {
        Self{w,h,bytes:vec![0;w*h*4]}
    }
    pub(crate) fn paint_for_test(&mut self,b:MediaBox,rgb:(u8,u8,u8)){
        for y in b.top.max(0) as usize..b.bottom.max(0).min(self.h as i32) as usize{
            for x in b.left.max(0) as usize..b.right.max(0).min(self.w as i32) as usize{
                let p=(y*self.w+x)*4;
                self.bytes[p..p+4].copy_from_slice(&[rgb.2,rgb.1,rgb.0,255]);
            }
        }
    }
}
pub unsafe fn capture_media(hwnd: HWND)->Result<Shot,String> {
    let mut rect:RECT=zeroed();
    if GetWindowRect(hwnd,&mut rect)==0 {return Err("Cannot read CapCut window bounds".into());}
    let width=rect.right-rect.left;
    let height=rect.bottom-rect.top;
    if width<700 || height<600 {return Err("CapCut window too small for media detection".into());}
    let w=width.min(700);
    let h=height.min(595);
    let screen=GetDC(null_mut());
    if screen.is_null() {return Err("GetDC failed".into());}
    let memory=CreateCompatibleDC(screen);
    if memory.is_null() {ReleaseDC(null_mut(),screen);return Err("CreateCompatibleDC failed".into());}
    let mut info:BITMAPINFO=zeroed();
    info.bmiHeader.biSize=size_of::<BITMAPINFOHEADER>() as u32;
    info.bmiHeader.biWidth=w;
    info.bmiHeader.biHeight=-h;
    info.bmiHeader.biPlanes=1;
    info.bmiHeader.biBitCount=32;
    info.bmiHeader.biCompression=BI_RGB;
    let mut raw:*mut std::ffi::c_void=null_mut();
    let bitmap=CreateDIBSection(screen,&info,DIB_RGB_COLORS,&mut raw,null_mut(),0);
    if bitmap.is_null() || raw.is_null() {
        DeleteDC(memory);ReleaseDC(null_mut(),screen);
        return Err("CreateDIBSection failed".into());
    }
    let original=SelectObject(memory,bitmap as HGDIOBJ);
    let copied=BitBlt(memory,0,0,w,h,screen,rect.left,rect.top,SRCCOPY|CAPTUREBLT);
    let bytes=if copied!=0 {
        std::slice::from_raw_parts(raw as *const u8,(w as usize)*(h as usize)*4).to_vec()
    } else {Vec::new()};
    SelectObject(memory,original);
    DeleteObject(bitmap as HGDIOBJ);
    DeleteDC(memory);
    ReleaseDC(null_mut(),screen);
    if copied==0 {return Err("Media capture BitBlt failed".into());}
    Ok(Shot {w:w as usize,h:h as usize,bytes})
}

pub fn selected_media(s:&Shot)->Option<MediaBox> {
    let x1=105usize.min(s.w);
    let y1=145usize.min(s.h);
    let x2=680usize.min(s.w);
    let y2=575usize.min(s.h);
    let mut edges:Vec<(i32,i32,i32)>=Vec::new();
    for y in y1..y2 {
        let mut first=None;
        let mut last=0usize;
        let mut hits=0;
        for x in x1..x2 {
            if s.cyan(x,y) {
                if first.is_none(){first=Some(x);}
                last=x;hits+=1;
            }
        }
        if let Some(left)=first {
            if hits>=48 && last.saturating_sub(left)>=70 {
                edges.push((y as i32,left as i32,last as i32));
            }
        }
    }
    for &(top,left,right) in &edges {
        for &(bottom,bl,br) in edges.iter().rev() {
            if !(60..=102).contains(&(bottom-top)) {continue;}
            if (left-bl).abs()>10 || (right-br).abs()>10 {continue;}
            return Some(MediaBox {
                left:left.min(bl).saturating_sub(12),
                top, right:right.max(br)+4, bottom,
            });
        }
    }
    None
}

// Media cards are laid out in uniform grid cells on this CapCut/Jianying
// profile. A selected cyan outline is not always on the freshly imported
// item, especially AFTER the first import, so never use selection alone to
// decide which thumbnail to add to the timeline.
pub(crate) fn media_slots(s:&Shot)->Vec<MediaBox> {
    let panel_right=s.w.min(680);
    let cols=((panel_right.saturating_sub(105))/136).clamp(2,4);
    let mut slots=Vec::with_capacity(cols*4);
    for row in 0..4usize {
        for col in 0..cols {
            let x=102+col*136;
            let y=145+row*112;
            if x+134>panel_right || y+100>s.h.min(590){continue;}
            slots.push(MediaBox{
                left:(x+18) as i32,top:(y+5) as i32,
                right:(x+134) as i32,bottom:(y+83) as i32,
            });
        }
    }
    slots
}
pub(crate) fn box_score(before:&Shot,after:&Shot,b:MediaBox)->f64 {
    let mut total=0u64;
    let mut count=0u64;
    for y in (b.top as usize..b.bottom as usize).step_by(4) {
        for x in (b.left as usize..b.right as usize).step_by(4) {
            let (r1,g1,b1)=before.pixel(x,y);
            let (r2,g2,b2)=after.pixel(x,y);
            total+=r1.abs_diff(r2) as u64+
                g1.abs_diff(g2) as u64+b1.abs_diff(b2) as u64;
            count+=3;
        }
    }
    total as f64/count.max(1) as f64
}
fn ranked_media_changes(before:&Shot,after:&Shot)->Vec<(MediaBox,f64)> {
    if before.w!=after.w || before.h!=after.h {return Vec::new();}
    let mut diffs:Vec<_>=media_slots(after).into_iter()
        .map(|b|(b,box_score(before,after,b))).collect();
    diffs.sort_by(|a,b|b.1.total_cmp(&a.1));
    diffs
}
pub fn changed_media(before:&Shot,after:&Shot)->Option<(MediaBox,f64)> {
    ranked_media_changes(before,after).into_iter().next()
        .filter(|(_,score)|*score>=5.0)
}
// Infer the offset of CapCut's selected outline compared to its enclosing
// grid cell, and transfer that offset to a NEW cell. This keeps a known-good
// Add (+) hotspot when CapCut keeps the OLD selected item after importing B.
fn translate_selection_to_new_slot(
    selected:MediaBox,
    slots:&[MediaBox],
    target:MediaBox,
)->Option<MediaBox>{
    let anchor=slots.iter()
        .filter(|s|s.overlaps(selected))
        .min_by_key(|s|{
            (s.left-selected.left).abs()+
            (s.top-selected.top).abs()+
            (s.right-selected.right).abs()+
            (s.bottom-selected.bottom).abs()
        })?;
    let deltas=[
        selected.left-anchor.left,selected.top-anchor.top,
        selected.right-anchor.right,selected.bottom-anchor.bottom,
    ];
    if deltas.iter().any(|d|d.abs()>28){return None;}
    Some(MediaBox{
        left:target.left+deltas[0],top:target.top+deltas[1],
        right:target.right+deltas[2],bottom:target.bottom+deltas[3],
    })
}

/// Only return a tile when its pixels changed significantly and the changed
/// slot is distinct from other animated regions. If the newly selected tile
/// matches, trust the real selected geometry. Otherwise calibrate the new
/// slot against the old selected card, NOT against the stale card itself.
pub fn find_imported_media(before:&Shot,after:&Shot)->Option<(MediaBox,f64)> {
    let ranked=ranked_media_changes(before,after);
    let (changed,score)=*ranked.first()?;
    let selected_after=selected_media(after);
    let selected_before=selected_media(before);
    if score<5.0 {return None;}
    // Fresh selection is strong evidence even if the appearance is subtle.
    if let Some(selected)=selected_after {
        if selected.overlaps(changed) {
            return Some((selected,score));
        }
    }
    // Unselected new tile: require change concentrated in one slot, not a
    // general UI redraw (which might otherwise click unrelated Media).
    let runner_up=ranked.get(1).map(|(_,score)|*score).unwrap_or(0.0);
    if score<7.0 || score<runner_up*1.55+2.0 {
        return None;
    }
    let slots=media_slots(after);
    if let Some(selected)=selected_after.or(selected_before) {
        if let Some(calibrated)=translate_selection_to_new_slot(selected,&slots,changed) {
            return Some((calibrated,score));
        }
    }
    // No known selected outline to calibrate against; use only aligned grid
    // geometry when the visual evidence is unusually strong.
    if score>=22.0 && score>=runner_up*2.0+2.0 {
        Some((changed,score))
    }else{None}
}
// When importing a second image, CapCut may keep highlighting A.
// The new B thumbnail must be selected before using the (+) button.
// Unselected targets receive the selection-outline offset inferred from a
// visible selected card. This does not identify a source image by itself;
// the PNG matcher in media_reference.rs supplies that evidence.
pub(crate) fn selected_geometry_for_slot(
    target:MediaBox,after:&Shot
)->MediaBox {
    let Some(selected)=selected_media(after) else {return target};
    if selected.overlaps(target){return selected;}
    translate_selection_to_new_slot(selected,&media_slots(after),target)
        .unwrap_or(target)
}
pub fn needs_select_new_tile(target:MediaBox,current_selection:Option<MediaBox>)->bool{
    current_selection.is_some_and(|s|!s.overlaps(target))
}
pub fn media_diagnostic(before:&Shot,after:&Shot)->String {
    let diffs=ranked_media_changes(before,after);
    let top=diffs.iter().take(3)
        .map(|(b,s)|format!("slot=({},{}..{},{}),score={s:.1}",
                b.left,b.top,b.right,b.bottom))
        .collect::<Vec<_>>().join(" | ");
    format!("before_selection={:?} after_selection={:?} top_changes=[{}]",
        selected_media(before),selected_media(after),top)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn blank()->Shot {Shot{w:700,h:590,bytes:vec![25;700*590*4]}}
    fn draw_rect(shot:&mut Shot,box_:MediaBox,colour:(u8,u8,u8)){
        for y in box_.top as usize..box_.bottom as usize {
            for x in box_.left as usize..box_.right as usize{
                let p=(y*shot.w+x)*4;
                shot.bytes[p]=colour.2;shot.bytes[p+1]=colour.1;shot.bytes[p+2]=colour.0;
            }
        }
    }
    #[test] fn detects_imported_tile_without_cursor() {
        let before=blank(); let mut after=blank();
        draw_rect(&mut after,MediaBox{left:125,top:160,right:241,bottom:238},(180,35,20));
        let found=find_imported_media(&before,&after);
        assert!(found.is_some());
        assert!(found.unwrap().1>=20.0);
    }
    fn draw_outline(shot:&mut Shot,rect:MediaBox){
        // Approximate a cyan CapCut selection border.
        for x in rect.left..rect.right {
            for y in [rect.top,rect.bottom-1]{
                let p=((y as usize*shot.w)+x as usize)*4;
                shot.bytes[p]=220;shot.bytes[p+1]=200;shot.bytes[p+2]=0;
            }
        }
    }
    #[test] fn reuses_same_selected_slot_after_old_media_was_removed(){
        let mut before=blank();
        let mut after=blank();
        let selected=MediaBox {left:120,top:150,right:236,bottom:228};
        draw_rect(&mut before,selected,(45,75,75));
        draw_rect(&mut after,selected,(45,107,75)); // average RGB delta ~10.7
        draw_outline(&mut before,selected);
        draw_outline(&mut after,selected);
        let detected=find_imported_media(&before,&after);
        assert!(detected.is_some(),
            "new media in the same slot must not require a moved outline");
        let (actual,score)=detected.unwrap();
        assert!(score>=8.0 && score<20.0,"score={score}");
        assert!(actual.overlaps(selected));
    }
    #[test] fn no_deletion_no_pixel_change_means_no_click(){
        let mut shot=blank();
        let rect=MediaBox{left:120,top:150,right:236,bottom:228};
        draw_rect(&mut shot,rect,(45,107,75));
        draw_outline(&mut shot,rect);
        assert!(find_imported_media(&shot,&shot).is_none());
    }
    #[test] fn selected_thumbnail_geometry_is_used_for_first_import(){
        let before=blank();
        let mut after=blank();
        let tile=MediaBox{left:120,top:150,right:236,bottom:228};
        draw_rect(&mut after,tile,(210,40,30));
        draw_outline(&mut after,tile);
        let selected=selected_media(&after).expect("selected tile must be visible");
        let (got,_)=find_imported_media(&before,&after)
            .expect("new media tile must be detected");
        assert_eq!(got,selected,
            "v0.7.5 regression: grid rectangle displaced the (+) hit target");
    }
    #[test] fn second_import_uses_second_tile_button_not_old_selection(){
        let mut before=blank();
        let mut after=blank();
        let first=MediaBox {left:120,top:150,right:236,bottom:228};
        let second=MediaBox {left:256,top:150,right:372,bottom:228};
        draw_rect(&mut before,first,(150,50,50));
        draw_rect(&mut after,first,(150,50,50));
        draw_rect(&mut after,second,(40,210,90));
        // First tile stays selected; the second newly imported tile is
        // detected by its own grid slot, not the stale selected rectangle.
        draw_outline(&mut before,first);
        draw_outline(&mut after,first);
        let (got,score)=find_imported_media(&before,&after).unwrap();
        assert!(score>20.0,"score={score}");
        assert!(got.overlaps(second));
        assert!(!got.overlaps(first), "must not click the old selected tile");
        assert!((got.right-12-second.right).abs()<20);
        assert!((got.bottom-12-second.bottom).abs()<20);
    }
    #[test] fn second_copy_with_low_contrast_is_detected_despite_old_selection(){
        let mut before=blank();
        let mut after=blank();
        let first=MediaBox{left:120,top:150,right:236,bottom:228};
        let second=MediaBox{left:256,top:150,right:372,bottom:228};
        draw_rect(&mut before,first,(50,70,90));
        draw_rect(&mut after,first,(50,70,90));
        // Brightness differs by only 10 units; prior code required >=22
        // unless the cyan border moved to the second card.
        draw_rect(&mut after,second,(35,35,35));
        draw_outline(&mut before,first);
        draw_outline(&mut after,first);
        let (got,score)=find_imported_media(&before,&after)
            .expect("new low-contrast slot B should not be rejected");
        assert!(score>=7.0 && score<22.0,"score={score}");
        assert!(got.overlaps(second));
        assert!(!got.overlaps(first));
        assert!((got.right-12-second.right).abs()<20);
    }
    #[test] fn selects_new_tile_only_when_old_tile_remains_selected(){
        let a=MediaBox{left:120,top:150,right:236,bottom:228};
        let b=MediaBox{left:256,top:150,right:372,bottom:228};
        assert!(needs_select_new_tile(b,Some(a)));
        assert!(!needs_select_new_tile(b,Some(b)));
        assert!(!needs_select_new_tile(b,None));
    }
    #[test] fn calibrated_selected_outline_moves_to_second_item(){
        let slots=media_slots(&blank());
        let original=MediaBox{left:110,top:149,right:240,bottom:231};
        let b=MediaBox{left:256,top:150,right:372,bottom:228};
        let predicted=translate_selection_to_new_slot(original,&slots,b).unwrap();
        assert_eq!(predicted.left,246);
        assert_eq!(predicted.top,149);
        assert_eq!(predicted.right,376);
        assert_eq!(predicted.bottom,231);
    }
    #[test] fn generic_ui_redraw_does_not_choose_ambiguous_tile(){
        let mut before=blank();
        let mut after=blank();
        let a=MediaBox{left:120,top:150,right:236,bottom:228};
        let b=MediaBox{left:256,top:150,right:372,bottom:228};
        draw_rect(&mut before,a,(25,25,25));
        draw_rect(&mut before,b,(25,25,25));
        draw_rect(&mut after,a,(50,50,50));
        draw_rect(&mut after,b,(50,50,50));
        assert!(find_imported_media(&before,&after).is_none());
    }
    #[test] fn unchanged_media_is_not_clicked() {
        let s=blank(); assert!(find_imported_media(&s,&s).is_none());
    }
}
