//! Render the self-contained ClipBridge three-colour circle icon at build time.
//! No Python/Pillow, network, or external icon converter is required.
//! The ICO contains 32-bit BMP frames for standard Windows taskbar/tray sizes.
use std::{env,fs,path::{Path,PathBuf},process::Command,f64::consts::PI};

const SIZES:&[u32]=&[16,20,24,32,40,48,64,128,256];
const COLOURS:[[u8;3];3]=[
    [248,48,107], // pink
    [27,117,244], // blue
    [255,194,35], // amber
];

fn write_u16(target:&mut Vec<u8>,v:u16){target.extend_from_slice(&v.to_le_bytes());}
fn write_u32(target:&mut Vec<u8>,v:u32){target.extend_from_slice(&v.to_le_bytes());}
fn color_at(x:f64,y:f64,n:f64)->Option<[u8;3]>{
    let dx=x-n/2.0;
    let dy=y-n/2.0;
    let radius=(dx*dx+dy*dy).sqrt();
    let max_r=n*0.463;
    if radius>=max_r{return None;}
    let angle=(dy.atan2(dx)+PI/2.0+0.46*(1.0-radius/max_r)).rem_euclid(2.0*PI);
    let sector=((angle/(2.0*PI/3.0)).floor() as usize).min(2);
    Some(COLOURS[sector])
}
fn icon_frame(n:u32)->Vec<u8>{
    let mut output=Vec::<u8>::new();
    let mask_row_bytes=(n.div_ceil(32)*4) as usize;
    let xor_bytes=n*n*4;
    let mask_bytes=mask_row_bytes as u32*n;
    write_u32(&mut output,40);                 // BITMAPINFOHEADER
    write_u32(&mut output,n);                  // width
    write_u32(&mut output,n*2);                // doubled height for XOR+AND mask
    write_u16(&mut output,1);                  // planes
    write_u16(&mut output,32);                 // bits
    write_u32(&mut output,0);                  // BI_RGB
    write_u32(&mut output,xor_bytes+mask_bytes);
    write_u32(&mut output,0);write_u32(&mut output,0);
    write_u32(&mut output,0);write_u32(&mut output,0);
    // Four subpixel samples provide antialiasing without adding a fourth colour.
    let samples=[(0.25,0.25),(0.75,0.25),(0.25,0.75),(0.75,0.75)];
    let mut transparent_mask=vec![0u8;mask_bytes as usize];
    for row in 0..n {
        let y=n-row-1;
        for x in 0..n {
            let mut alpha=0u32;
            let mut rgb=[0u32;3];
            for &(sx,sy) in &samples {
                if let Some(c)=color_at(x as f64+sx,y as f64+sy,n as f64){
                    alpha+=1;
                    for k in 0..3 {rgb[k]+=c[k] as u32;}
                }
            }
            let (red,green,blue,opacity)=if alpha==0{
                let mask_byte=row as usize*mask_row_bytes+x as usize/8;
                transparent_mask[mask_byte]|=1<<(7-x%8);
                (0,0,0,0)
            } else {
                ((rgb[0]/alpha) as u8,(rgb[1]/alpha) as u8,
                 (rgb[2]/alpha) as u8,(alpha*255/4) as u8)
            };
            output.extend_from_slice(&[blue,green,red,opacity]);
        }
    }
    output.extend_from_slice(&transparent_mask);
    output
}
fn write_icon(path:&Path)->std::io::Result<()> {
    let frames:Vec<_>=SIZES.iter().map(|n|icon_frame(*n)).collect();
    let count=frames.len() as u16;
    let mut ico=Vec::<u8>::new();
    write_u16(&mut ico,0);write_u16(&mut ico,1);write_u16(&mut ico,count);
    let mut offset=6+16*frames.len() as u32;
    for (&n,frame) in SIZES.iter().zip(frames.iter()){
        ico.push(if n==256{0}else{n as u8});
        ico.push(if n==256{0}else{n as u8});
        ico.push(0);ico.push(0); // color count and reserved
        write_u16(&mut ico,1);write_u16(&mut ico,32);
        write_u32(&mut ico,frame.len() as u32);
        write_u32(&mut ico,offset);
        offset+=frame.len() as u32;
    }
    for frame in frames {ico.extend_from_slice(&frame);}
    fs::write(path,ico)
}
fn main(){
    println!("cargo:rerun-if-changed=build.rs");
    if env::var("CARGO_CFG_TARGET_OS").as_deref()!=Ok("windows"){return;}
    let out=PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR missing"));
    let icon=out.join("clipbridge-three-colour.ico");
    write_icon(&icon).expect("Cannot generate the ClipBridge circular icon");
    let resource=out.join("generated-app.rc");
    let icon_string=icon.display().to_string().replace('\\',"/");
    fs::write(&resource,format!("1 ICON \"{icon_string}\"\r\n"))
        .expect("Cannot generate icon resource declaration");
    let gnu=env::var("CARGO_CFG_TARGET_ENV").as_deref()==Ok("gnu");
    let output=out.join(if gnu{"app-icon.o"}else{"app-icon.res"});
    let mut cmd=Command::new(if gnu{"windres.exe"}else{"rc.exe"});
    if gnu{
        cmd.args(["-i"]).arg(&resource).arg("-o").arg(&output);
    }else{
        cmd.arg("/nologo").arg("/fo").arg(&output).arg(&resource);
    }
    let status=cmd.status().expect("Windows resource compiler is required");
    assert!(status.success(),"Windows icon resource compiler failed");
    println!("cargo:rustc-link-arg-bin=clipbridge={}",output.display());
}
