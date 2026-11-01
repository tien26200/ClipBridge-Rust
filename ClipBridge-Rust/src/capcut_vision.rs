//! Lightweight visual verification of a newly imported CapCut Media tile.
//! Ported as an independent Rust algorithm, based on the public MIT Jiandan
//! reference (https://github.com/SKYCHENV/Jiandan).
//! Never moves the pointer, writes screenshots, or scans the whole desktop.
use std::{mem::{size_of,zeroed},ptr::null_mut};
use windows_sys::Win32::{Foundation::*,Graphics::Gdi::*,UI::WindowsAndMessaging::*};

#[derive(Debug,Clone,Copy,PartialEq,Eq)]
pub struct MediaBox {pub left:i32,pub top:i32,pub right:i32,pub bottom:i32}
impl MediaBox {
    fn overlaps(self, other:Self)->bool {
        self.left<other.right && other.left<self.right &&
        self.top<other.bottom && other.top<self.bottom
    }
}
pub struct Shot {w:usize,h:usize,bytes:Vec<u8>}
impl Shot {
    pub fn width(&self)->usize {self.w}
    pub fn height(&self)->usize {self.h}
    fn pixel(&self,x:usize,y:usize)->(u8,u8,u8) {
        let p=(y*self.w+x)*4;
        (self.bytes[p+2],self.bytes[p+1],self.bytes[p])
    }
    fn cyan(&self,x:usize,y:usize)->bool {
        let (r,g,b)=self.pixel(x,y);
        r<110 && g>120 && b>120
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

pub fn changed_media(before:&Shot,after:&Shot)->Option<(MediaBox,f64)> {
    if before.w!=after.w || before.h!=after.h {return None;}
    let mut best:(f64,Option<MediaBox>)=(0.0,None);
    let x_max=after.w.saturating_sub(25);
    let y_max=after.h.saturating_sub(20);
    let mut y=145usize;
    while y+86<y_max {
        let mut x=102usize;
        while x+116<x_max {
            let mut total=0u64;
            let mut count=0u64;
            for yy in (y..y+78).step_by(4) {
                for xx in (x..x+116).step_by(4) {
                    let (r1,g1,b1)=before.pixel(xx,yy);
                    let (r2,g2,b2)=after.pixel(xx,yy);
                    total+=(r1.abs_diff(r2) as u64)
                         +(g1.abs_diff(g2) as u64)
                         +(b1.abs_diff(b2) as u64);
                    count+=3;
                }
            }
            let score=total as f64 / count.max(1) as f64;
            if score>best.0 {
                best=(score,Some(MediaBox {
                    left:x as i32, top:y as i32,
                    right:(x+116) as i32,bottom:(y+78) as i32
                }));
            }
            x+=34;
        }
        y+=28;
    }
    if best.0>=8.0 {best.1.map(|b|(b,best.0))} else {None}
}

pub fn find_imported_media(before:&Shot,after:&Shot)->Option<(MediaBox,f64)> {
    let before_selected=selected_media(before);
    let selected=selected_media(after);
    let changed=changed_media(before,after);
    if let Some(s)=selected {
        if (before_selected!=Some(s) && changed.is_some_and(|(b,_)|b.overlaps(s)))
            || (before_selected!=Some(s) && changed.is_none())
        {
            return Some((s,changed.map(|(_,score)|score).unwrap_or(0.0)));
        }
    }
    // Conservative fallback: strong thumbnail change is safer than clicking
    // the old selected tile. Never click when visual evidence is weak.
    changed.filter(|(_,score)|*score>=20.0)
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
    #[test] fn unchanged_media_is_not_clicked() {
        let s=blank(); assert!(find_imported_media(&s,&s).is_none());
    }
}
