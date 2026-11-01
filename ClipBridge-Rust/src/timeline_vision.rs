//! Version-sensitive, read-only CapCut timeline detection.
//! The Media (+) operation does NOT tell ClipBridge where the playhead is.
//! Playhead-aware placement requires detecting the actual marker in a full
//! editor screenshot. Never infer its time from the Media asset coordinates.
//! This is UI automation, not a CapCut API and must be validated on target UI.
use crate::capcut_vision::Shot;

#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct Playhead {
    pub x:i32,
    pub score:u32,
}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct DropTarget {
    pub x:i32,
    pub y:i32,
    pub playhead_x:i32,
}
fn neutral_bright(rgb:(u8,u8,u8))->bool {
    let min=rgb.0.min(rgb.1).min(rgb.2);
    let max=rgb.0.max(rgb.1).max(rgb.2);
    min>175 && max-min<24
}
fn marker_line(rgb:(u8,u8,u8))->bool {
    let min=rgb.0.min(rgb.1).min(rgb.2);
    let max=rgb.0.max(rgb.1).max(rgb.2);
    min>=110 && max<=215 && max-min<35
}
pub fn find_playhead(s:&Shot)->Option<Playhead>{
    let w=s.width();
    let h=s.height();
    if w<900||h<650{return None;}
    let ruler_from=h*56/100;
    let ruler_until=h*63/100;
    let line_from=h*57/100;
    let line_until=h.saturating_sub(35);
    if ruler_until<=ruler_from||line_until<=line_from{return None;}
    let mut peaks:Vec<(u32,usize)>=Vec::new();
    // Match both the bright ruler handle AND a long neutral vertical line,
    // to avoid detecting the bright border of a selected timeline clip.
    for x in 145..w.saturating_sub(20){
        let mut marker=0u32;
        for y in ruler_from..ruler_until{
            if neutral_bright(s.pixel(x,y)){marker+=1;}
        }
        if marker<2 {continue;}
        let mut longest=0usize;
        let mut current=0usize;
        for y in line_from..line_until {
            if marker_line(s.pixel(x,y)){
                current+=1;longest=longest.max(current);
            }else{current=0;}
        }
        if longest<45 {continue;}
        let score=marker*8+longest as u32;
        if score>=61 {peaks.push((score,x));}
    }
    peaks.sort_by(|a,b|b.0.cmp(&a.0));
    let &(best_score,best_x)=peaks.first()?;
    // Several pixels of the same vertical marker are okay. A separate
    // marker with comparable strength is ambiguous, so DO NOT drop.
    if peaks.iter().skip(1).any(|(score,x)|
        x.abs_diff(best_x)>12 && *score>=best_score*78/100){
        return None;
    }
    if best_x<145||best_x>=w-20{return None;}
    Some(Playhead{x:best_x as i32,score:best_score})
}
fn patch_brightness(s:&Shot,x:i32,y:i32)->f64 {
    let mut total=0u32;
    let mut samples=0u32;
    // Look to the right of the insertion point, not on the bright playhead.
    // Light/noisy regions may be occupied clips or UI chrome; favor a
    // darker, more likely empty video band for drag-and-drop.
    for yy in (y-12..=y+12).step_by(4){
        for xx in (x+16..=x+94).step_by(6){
            if xx<145||xx>=s.width() as i32||yy<0||yy>=s.height() as i32{
                continue;
            }
            let (r,g,b)=s.pixel(xx as usize,yy as usize);
            total+=r.max(g).max(b) as u32;samples+=1;
        }
    }
    if samples==0 {return 255.0;}
    total as f64/samples as f64
}
pub fn timeline_drop_target(s:&Shot,p:Playhead)->Option<DropTarget>{
    let w=s.width() as i32;
    let h=s.height() as i32;
    // The target must be on the visible timeline, not the preview, effects
    // panel, toolbar or the left track-controls area.
    if p.x<145||p.x+105>=w-14 {return None;}
    let mut options=Vec::new();
    for percent in [81,76,85,72] {
        let y=h*percent/100;
        if y<=h*68/100||y>=h-55{continue;}
        options.push((patch_brightness(s,p.x,y),y));
    }
    options.sort_by(|a,b|a.0.total_cmp(&b.0));
    let (brightness,y)=*options.first()?;
    // The destination appears occupied or not a safely empty track.
    // Refuse to drop rather than overwrite a clip at the playhead.
    if brightness>73.0{return None;}
    Some(DropTarget{x:p.x,y,playhead_x:p.x})
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::capcut_vision::MediaBox;
    fn editor_at(x:usize)->Shot {
        let mut s=Shot::for_test(1400,900);
        for y in 900*56/100..900*63/100{
            s.paint_for_test(MediaBox{
                left:x as i32,top:y as i32,right:x as i32+1,bottom:y as i32+1
            },(230,230,230));
        }
        for y in 900*57/100..865 {
            // Neutral gray line - overrides top marker but still leaves the
            // top ruler handle present in a separate area of the ruler.
            s.paint_for_test(MediaBox{
                left:x as i32,top:y as i32,right:x as i32+1,bottom:y as i32+1
            },(170,170,170));
        }
        s
    }
    #[test] fn finds_real_playhead_then_targets_it(){
        let shot=editor_at(650);
        let p=find_playhead(&shot).unwrap();
        assert_eq!(p.x,650);
        let target=timeline_drop_target(&shot,p).unwrap();
        assert_eq!(target.x,650);
        assert!(target.y>900*68/100);
        assert!(target.y<845);
    }
    #[test] fn rejects_two_simultaneously_similar_markers(){
        let mut shot=editor_at(650);
        let alt=editor_at(950);
        for y in 900*56/100..865 {
            let colour=alt.pixel(950,y);
            shot.paint_for_test(MediaBox{
                left:950,top:y as i32,right:951,bottom:y as i32+1
            },colour);
        }
        assert_eq!(find_playhead(&shot),None);
    }
    #[test] fn empty_dark_editor_does_not_invent_playhead(){
        let shot=Shot::for_test(1400,900);
        assert_eq!(find_playhead(&shot),None);
    }
    #[test] fn occupied_target_must_not_be_overwritten(){
        let mut shot=editor_at(650);
        for y in 600..850{
            shot.paint_for_test(MediaBox{left:666,top:y,right:760,bottom:y+1},(180,180,180));
        }
        let p=find_playhead(&shot).unwrap();
        assert_eq!(timeline_drop_target(&shot,p),None);
    }
}
