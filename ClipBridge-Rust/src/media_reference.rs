//! Verify a CapCut thumbnail against the exact PNG that ClipBridge imported.
//! The v0.7.7 "second image" bug arises when old selection outlines stay
//! unchanged, and the full Media panel repaints. A whole-panel diff cannot
//! distinguish the new PNG from an older Media item. Use a low-memory colour
//! histogram of the actual source file, plus a slot-change guard.
//!
//! No screenshot pixels, user images, or RGB histograms are written to disk.
//! Verification is approximate: it fails closed if thumbnails are ambiguous.
use crate::capcut_vision::{
    box_score,media_slots,media_search_slots,selected_geometry_for_slot,MediaBox,Shot,
};
use std::{fs::File,io::BufReader,path::Path};

const BINS:usize=8;
#[derive(Debug)]
pub struct Reference {
    aspect:f64,
    bins:[f64;BINS*3],
}
#[derive(Clone,Copy,Debug)]
pub struct MatchingTile {
    pub rect:MediaBox,
    pub slot:MediaBox,
    pub loss:f64,
    pub runner_up_loss:f64,
    pub change_score:f64,
    pub row_shift:i32,
}
fn add_pixel(hist:&mut [f64;BINS*3],rgb:(u8,u8,u8)){
    for (channel,value) in [rgb.0,rgb.1,rgb.2].iter().enumerate(){
        hist[channel*BINS+(*value as usize/32).min(7)]+=1.0;
    }
}
fn normalize(hist:&mut [f64;BINS*3],samples:usize){
    if samples==0{return;}
    for b in hist { *b/=samples as f64; }
}
fn loss(a:&[f64;BINS*3],b:&[f64;BINS*3])->f64{
    a.iter().zip(b.iter()).map(|(a,b)|(a-b).abs()).sum()
}
// Scan offsets of one physical thumbnail are highly correlated. Without
// non-maximum suppression, a source match at y=234 and y=248 is treated as
// two competing Media files and incorrectly rejected due low margin.
fn same_card(a:MediaBox,b:MediaBox)->bool {
    let ax=(a.left+a.right)/2;
    let bx=(b.left+b.right)/2;
    let ay=(a.top+a.bottom)/2;
    let by=(b.top+b.bottom)/2;
    (ax-bx).abs()<=65 && (ay-by).abs()<=75
}
fn confidence_runner_up(
    candidates:&[(f64,MediaBox,f64)]
)->Option<(f64,MediaBox,f64)>{
    let (_,best,_) = *candidates.first()?;
    candidates.iter().copied().find(|(_,b,_)|!same_card(*b,best))
}
fn nearest_row_shift(slot:MediaBox,anchors:&[MediaBox])->i32{
    anchors.iter()
        .filter(|b|(b.left-slot.left).abs()<=8)
        .map(|b|slot.top-b.top)
        .min_by_key(|d|d.abs())
        .unwrap_or(0)
}
impl Reference {
    pub fn from_png(path:&Path)->Result<Self,String>{
        let file=File::open(path).map_err(|e|e.to_string())?;
        let mut decoder=png::Decoder::new(BufReader::new(file));
        decoder.set_transformations(png::Transformations::EXPAND|png::Transformations::STRIP_16);
        let mut reader=decoder.read_info().map_err(|e|e.to_string())?;
        let (w,h)=(reader.info().width as usize,reader.info().height as usize);
        if w==0||h==0||w.saturating_mul(h)>90_000_000 {
            return Err("PNG dimensions outside matcher limits".into());
        }
        let (colour,depth)=reader.output_color_type();
        if depth!=png::BitDepth::Eight {
            return Err("PNG source matcher only supports decoded 8-bit colour".into());
        }
        let channels=match colour {
            png::ColorType::Rgb=>3,
            png::ColorType::Rgba=>4,
            png::ColorType::Grayscale=>1,
            png::ColorType::GrayscaleAlpha=>2,
            _=>return Err("Unsupported source PNG colour type".into()),
        };
        let x_step=(w/42).max(1);
        let y_step=(h/30).max(1);
        let mut bins=[0f64;BINS*3];
        let mut samples=0usize;
        for y in 0..h {
            let row=reader.next_row().map_err(|e|e.to_string())?
                .ok_or("PNG decoder returned fewer rows than expected")?;
            if y%y_step!=y_step/2 {continue;}
            let data=row.data();
            for x in (x_step/2..w).step_by(x_step) {
                let p=x*channels;
                if p+channels>data.len(){continue;}
                let (r,g,b,a)=match channels {
                    4=>(data[p],data[p+1],data[p+2],data[p+3]),
                    3=>(data[p],data[p+1],data[p+2],255),
                    2=>(data[p],data[p],data[p],data[p+1]),
                    _=>(data[p],data[p],data[p],255)
                };
                let composite=|c:u8|(((c as u16*a as u16)+
                    29*(255-a as u16))/255) as u8;
                add_pixel(&mut bins,(composite(r),composite(g),composite(b)));
                samples+=1;
            }
        }
        if samples<24 {return Err("PNG too small for source fingerprint".into());}
        normalize(&mut bins,samples);
        Ok(Self{aspect:w as f64/h as f64,bins})
    }
    fn tile_histogram(&self,s:&Shot,b:MediaBox)->[f64;BINS*3] {
        let mut bins=[0.0;BINS*3];
        let width=(b.right-b.left).max(1) as f64;
        let height=(b.bottom-b.top).max(1) as f64;
        let ratio=self.aspect.clamp(0.03,30.0);
        let (image_w,image_h)=if width/height>ratio {
            (height*ratio,height)
        }else{(width,width/ratio)};
        let x0=b.left as f64+(width-image_w)/2.0;
        let y0=b.top as f64+(height-image_h)/2.0;
        let mut count=0usize;
        // Reject border pixels and filename text overlays at the bottom.
        for j in 1..15usize{
            for i in 1..21usize{
                let x=(x0+image_w*(i as f64/21.0)).round() as i32;
                let y=(y0+image_h*(j as f64/16.0)).round() as i32;
                if x<0||y<0||x>=s.width() as i32||y>=s.height() as i32{continue;}
                add_pixel(&mut bins,s.pixel(x as usize,y as usize));
                count+=1;
            }
        }
        normalize(&mut bins,count);
        bins
    }
    /// Only numeric location/confidence evidence is logged; never source
    /// pixels or screenshot data. Useful when a new row changes layout.
    pub fn diagnostic(&self,after:&Shot)->String {
        let mut candidates:Vec<(f64,MediaBox,f64)>=media_search_slots(after)
            .into_iter().map(|slot|{
                (loss(&self.tile_histogram(after,slot),&self.bins),slot,0.0)
            }).collect();
        candidates.sort_by(|a,b|a.0.total_cmp(&b.0));
        let Some((best,slot,_))=candidates.first().copied() else{
            return "no-visible-media-slots".into();
        };
        let second=confidence_runner_up(&candidates).map(|c|c.0).unwrap_or(6.0);
        let shift=nearest_row_shift(slot,&media_slots(after));
        format!(
            "best_loss={best:.2} second_loss={second:.2} x={} y={} row_shift={} unique={}",
            slot.left,slot.top,shift,best<=1.25 && second-best>=0.36
        )
    }
    /// Compare each grid slot with the real source PNG, then require:
    /// - a plausible colour-histogram match;
    /// - no equally good second tile;
    /// - a visible change OR source-consistent selected-card evidence.
    pub fn locate(&self,before:Option<&Shot>,after:&Shot)->Option<MatchingTile>{
        let anchors=media_slots(after);
        let mut candidates:Vec<(f64,MediaBox,f64)>=media_search_slots(after).into_iter()
            .map(|box_|{
                let hist=self.tile_histogram(after,box_);
                let l=loss(&hist,&self.bins);
                let change=before.map(|old|box_score(old,after,box_)).unwrap_or(0.0);
                (l,box_,change)
            }).collect();
        candidates.sort_by(|a,b|a.0.total_cmp(&b.0));
        let (best,slot,changed)=*candidates.first()?;
        // Compare against another PHYSICAL card, not neighboring scan
        // offsets of the same thumbnail. This is crucial when CapCut
        // auto-scrolls to the first item of the next row.
        let second=confidence_runner_up(&candidates).map(|c|c.0).unwrap_or(6.0);
        // 0 means an exact visual match, 6 the maximum colour disagreement.
        // Similar-looking media should fail rather than click the wrong card.
        if best>1.25 || second-best<0.36 {return None;}
        // In reuse mode B was already imported on the prior attempt, so a
        // before/after pixel difference is not expected. Confidence must
        // then come from the source's distinct visual match, not UI motion.
        if before.is_some() && changed<3.5 {return None;}
        let chosen=selected_geometry_for_slot(slot,after);
        let row_shift=nearest_row_shift(slot,&anchors);
        Some(MatchingTile{
            rect:chosen,
            slot,
            loss:best,
            runner_up_loss:second,
            change_score:changed,
            row_shift,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs,time::SystemTime};
    fn synthetic()->(Shot,Shot,MediaBox){
        let mut before=Shot::for_test(700,590);
        let mut after=Shot::for_test(700,590);
        let old=MediaBox{left:120,top:150,right:236,bottom:228};
        let new=MediaBox{left:256,top:150,right:372,bottom:228};
        before.paint_for_test(old,(220,55,55));
        after.paint_for_test(old,(220,55,55));
        after.paint_for_test(new,(20,70,220));
        (before,after,new)
    }
    fn png_for_test()->std::path::PathBuf{
        let path=std::env::temp_dir().join(format!("clipbridge-fingerprint-{:?}.png",
            SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        let file=File::create(&path).unwrap();
        let mut encoder=png::Encoder::new(file,116,78);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer=encoder.write_header().unwrap();
        writer.write_image_data(&[20u8,70,220].repeat(116*78)).unwrap();
        path
    }
    #[test] fn matching_png_finds_new_tile_not_old(){
        let path=png_for_test();
        let reference=Reference::from_png(&path).unwrap();
        let (before,after,new)=synthetic();
        let located=reference.locate(Some(&before),&after).expect("blue source is new Media tile");
        assert_eq!(located.rect,new);
        assert!(located.change_score>10.0);
        fs::remove_file(path).unwrap();
    }
    #[test] fn reuse_existing_source_does_not_require_new_import_diff(){
        let path=png_for_test();
        let reference=Reference::from_png(&path).unwrap();
        let (_,after,slot)=synthetic();
        let found=reference.locate(None,&after)
            .expect("a previously imported picture should be found without reimport");
        assert_eq!(found.slot,slot);
        assert!(found.loss<0.1,"source image should match target thumbnail");
        fs::remove_file(path).unwrap();
    }
    #[test] fn first_thumbnail_of_new_row_after_scroll_matches_source(){
        let png=png_for_test();
        let reference=Reference::from_png(&png).unwrap();
        let mut before=Shot::for_test(700,590);
        let mut after=Shot::for_test(700,590);
        let colour_old=(220,55,55);
        // Three old first-column cards visibly move when Media reflows.
        let old_rows=[
            MediaBox{left:120,top:150,right:236,bottom:228},
            MediaBox{left:120,top:262,right:236,bottom:340},
            MediaBox{left:120,top:374,right:236,bottom:452},
        ];
        for old in old_rows {before.paint_for_test(old,colour_old);}
        let shifted_old=[
            MediaBox{left:120,top:178,right:236,bottom:256},
            MediaBox{left:120,top:290,right:236,bottom:368},
        ];
        for old in shifted_old {after.paint_for_test(old,colour_old);}
        // Newly imported B is first thumbnail in the third displayed row,
        // with a y=+28 reflow relative to the old fixed grid.
        let new_b=MediaBox{left:120,top:402,right:236,bottom:480};
        after.paint_for_test(new_b,(20,70,220));
        let matched=reference.locate(Some(&before),&after)
            .expect("should identify B despite changes across first-column rows");
        assert!(matched.slot.overlaps(new_b));
        assert!((matched.slot.top-new_b.top).abs()<=14,
            "dynamic scan should find actual shifted row");
        assert!((matched.slot.left-new_b.left).abs()<=1);
        assert!(matched.change_score>=3.5);
        fs::remove_file(png).unwrap();
    }
    #[test] fn adjacent_scroll_offsets_are_same_physical_card(){
        let a=MediaBox{left:120,top:290,right:236,bottom:368};
        let b=MediaBox{left:120,top:304,right:236,bottom:382};
        let c=MediaBox{left:256,top:290,right:372,bottom:368};
        assert!(same_card(a,b));
        assert!(!same_card(a,c));
        let scored=[(0.1,a,50.0),(0.11,b,40.0),(2.0,c,10.0)];
        assert_eq!(confidence_runner_up(&scored).unwrap().0,2.0);
    }
    #[test] fn first_tile_fourth_row_with_negative_scroll_offset(){
        let png=png_for_test();
        let reference=Reference::from_png(&png).unwrap();
        let before=Shot::for_test(700,590);
        let mut after=Shot::for_test(700,590);
        let new_b=MediaBox{left:120,top:458,right:236,bottom:536};
        after.paint_for_test(new_b,(20,70,220));
        let found=reference.locate(Some(&before),&after)
            .expect("fourth row first column must be in visible search");
        assert!((found.slot.top-458).abs()<=14);
        fs::remove_file(png).unwrap();
    }
    #[test] fn unchanged_screen_does_not_claim_new_import(){
        let path=png_for_test();
        let reference=Reference::from_png(&path).unwrap();
        let (_,shot,_)=synthetic();
        assert!(reference.locate(Some(&shot),&shot).is_none());
        fs::remove_file(path).unwrap();
    }
}
