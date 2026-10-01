//! Offline detector correction. Keeps censored samples distinct from missing rays.
//! No device access; measured references define noise and fixed screen support.
use super::*;

pub(super) struct ReferenceStats { pub mean: Vec<f32>, pub variance: Vec<f32> }

pub(super) fn references(scan: &Path, frames: &[ReferenceFrame], map: &ScreenMap,
    mirror: bool, progress: &Mutex<ReconstructionProgress>, completed: &mut usize) -> Result<ReferenceStats,String> {
    let mut mean=vec![0f32;DETECTOR_N*DETECTOR_N];
    let mut variance=mean.clone();
    for (k,frame) in frames.iter().enumerate() {
        let image=decode_blue(&verified_path(scan,&frame.path,frame.bytes,&frame.sha256)?)?;
        let sampled=remap(&image,map,mirror)?;
        for i in 0..mean.len() {
            let delta=sampled[i]-mean[i]; mean[i]+=delta/(k+1) as f32;
            variance[i]+=delta*(sampled[i]-mean[i]);
        }
        *completed+=1;
        set_progress(progress,"running",(10+*completed*10/(REFERENCE_COUNT*2)) as u8,"处理校正帧与噪声统计");
    }
    for value in &mut variance {*value/=frames.len().saturating_sub(1).max(1) as f32;}
    Ok(ReferenceStats {mean,variance})
}

#[derive(Default,Serialize)]
#[serde(rename_all="camelCase")]
pub(super) struct Quality {
    pub index:u32, pub samples:usize, pub weak_gain:usize, pub saturated:usize,
    pub nonpositive:usize, pub low_signal:usize, pub low_gain:usize,
}

pub(super) struct Correction { pub projection:Vec<f32>, pub flags:Vec<u8>, pub quality:Quality }

pub(super) fn support_threshold(dark:&ReferenceStats,flat:&ReferenceStats)->f32 {
    let mut gains:Vec<_>=flat.mean.iter().zip(&dark.mean).enumerate().filter_map(|(i,(f,d))| {
        let u=((i%DETECTOR_N) as f64+0.5)/DETECTOR_N as f64*2.-1.;
        let v=((i/DETECTOR_N) as f64+0.5)/DETECTOR_N as f64*2.-1.;
        (u*u+v*v<0.8*0.8 && (f-d).is_finite()).then_some(f-d)
    }).collect();
    gains.sort_by(f32::total_cmp);
    gains.get(gains.len()/2).copied().unwrap_or(1.).mul_add(0.2,0.).max(1.)
}

/// Flags: 0 outside screen, 1 measured, 2 noise-censored, 3 reference/support
/// missing, 4 saturated, 5 low gain retained. Quality is not a spatial crop.
pub(super) fn correct(raw:&[f32],white:f32,dark:&ReferenceStats,flat:&ReferenceStats,
    threshold:f32,index:u32)->Result<Correction,String> {
    let n=DETECTOR_N;
    let mut p=vec![0.;n*n];let mut flags=vec![0u8;n*n];let mut weights=vec![0.;n*n];
    let mut q=Quality {index,..Default::default()};
    for i in 0..p.len() {
        let u=((i%n) as f64+0.5)/n as f64*2.-1.;let v=((i/n) as f64+0.5)/n as f64*2.-1.;
        if (u*u+v*v)*SCREEN_CROP_SCALE.powi(2)>1.0 {continue;} q.samples+=1;
        let gain=flat.mean[i]-dark.mean[i];let signal=raw[i]-dark.mean[i];
        if !gain.is_finite() || gain<=1. {q.weak_gain+=1;flags[i]=3;continue;}
        if !raw[i].is_finite() || raw[i]>=white {q.saturated+=1;flags[i]=4;continue;}
        if signal<=0. {q.nonpositive+=1;}
        let floor=(dark.variance[i].max(0.)*(1.+1./REFERENCE_COUNT as f32)).sqrt().max(1.);
        flags[i]=if signal<floor {q.low_signal+=1;2} else {1};
        if gain<threshold {q.low_gain+=1;if flags[i]==1 {flags[i]=5;}}
        p[i]=-(signal.max(floor)/gain).ln();weights[i]=1.;
    }
    // Keep the original gate denominator and invalid count. Excluding fixed
    // support must never disguise excessive raw-data failures.
    if q.samples==0 || q.weak_gain+q.saturated+q.nonpositive>q.samples/5 {
        return Err(format!("too many invalid detector samples in cropped view {}: {} / {}",index,q.weak_gain+q.saturated+q.nonpositive,q.samples));
    }
    p=normalized_smooth(&p,&weights,n);
    for i in 0..p.len() {
        let u=((i%n) as f64+0.5)/n as f64*2.-1.;let v=((i/n) as f64+0.5)/n as f64*2.-1.;
        let taper=((1.0-u.hypot(v)*SCREEN_CROP_SCALE)/0.02).clamp(0.,1.) as f32;
        p[i]*=weights[i]*taper;
    }
    Ok(Correction {projection:p,flags,quality:q})
}

fn normalized_smooth(input:&[f32],weights:&[f32],n:usize)->Vec<f32> {
    let kernel:Vec<f32>=(-2i32..=2).map(|x| (-(x*x) as f32/(2.*0.7*0.7)).exp()).collect();
    let convolve=|input:&[f32]| {
        let mut current=input.to_vec();
        for vertical in [false,true] {
            let mut next=vec![0.;n*n];
            for y in 0..n {for x in 0..n {for (k,w) in kernel.iter().enumerate() {
                let xx=x as isize+if vertical {0}else{k as isize-2};
                let yy=y as isize+if vertical {k as isize-2}else{0};
                if xx>=0 && yy>=0 && xx<n as isize && yy<n as isize {next[y*n+x]+=w*current[yy as usize*n+xx as usize];}
            }}}
            current=next;
        } current
    };
    let weighted:Vec<_>=input.iter().zip(weights).map(|(a,w)|a*w).collect();
    convolve(&weighted).iter().zip(convolve(weights)).map(|(a,w)|if w>1e-6 {a/w}else{0.}).collect()
}

#[derive(Serialize)]
#[serde(rename_all="camelCase")]
pub(super) struct AxisEstimate {
    pub recorded_offset_mm:f64,pub applied_offset_mm:f64,pub candidate_offset_mm:f64,
    pub fit_error_before:f64,pub fit_error_after:f64,pub held_error_before:f64,pub held_error_after:f64,
    pub paired_rays:usize,pub accepted:bool,pub reason:String,
}

/// Conjugate fan rays only at the source plane. Fixed common ray set across
/// candidates, disjoint fit/held angles, bounded search and improvement gate.
pub(super) fn estimate_axis(stack:&[f32],angles:&[f64],pitch:f64,sdd:f64,u0:f64,v0:f64)->AxisEstimate {
    let n=DETECTOR_N;let count=angles.len();let half=(n-1) as f64/2.;
    let mut result=AxisEstimate {recorded_offset_mm:u0,applied_offset_mm:u0,candidate_offset_mm:u0,
        fit_error_before:0.,fit_error_after:0.,held_error_before:0.,held_error_after:0.,paired_rays:0,
        accepted:false,reason:"insufficient conjugate-ray evidence".into()};
    if count<60 || stack.len()!=count*n*n {return result;}
    let row=half+v0/pitch;
    if row<1. || row>(n-2) as f64 {return result;}
    let step=angles[1]-angles[0];
    let candidates:Vec<_>=(-16..=16).map(|k|u0+k as f64*pitch).collect();
    let sample=|view:usize,col:f64|sample_detector(&stack[view*n*n..(view+1)*n*n],col,row) as f64;
    let mut errors=vec![(0.,0.,0usize,0usize);candidates.len()];
    for view in 0..count {for col in n/4..n*3/4 {
        let observed=sample(view,col as f64);
        if !(0.05..2.5).contains(&observed) {continue;}
        let mut matches=Vec::with_capacity(candidates.len());
        for &du in &candidates {
            let u=(col as f64-half)*pitch-du;
            let counterpart=(-u+du)/pitch+half;
            let beta=angles[view]+PI-2.*(u/sdd).atan();
            let position=((beta-angles[0])/step).rem_euclid(count as f64);
            let a=position.floor() as usize;let t=position-a as f64;
            let value=sample(a,counterpart)*(1.-t)+sample((a+1)%count,counterpart)*t;
            matches.push(value);
        }
        // Select rays by the source measurement only. Intersecting all
        // candidates' attenuation ranges preferentially discards edges and can
        // select a wrong axis in truncated/high-attenuation objects.
        if matches.iter().any(|x|!x.is_finite()) {continue;}
        for (error,value) in errors.iter_mut().zip(matches) {
            let e=(value-observed).abs();
            if view%3==2 {error.1+=e;error.3+=1;}else{error.0+=e;error.2+=1;}
        }
    }}
    if errors[16].2<300 || errors[16].3<150 {return result;}
    let scores:Vec<_>=errors.iter().map(|e|(e.0/e.2 as f64,e.1/e.3 as f64)).collect();
    let best=(0..scores.len()).min_by(|a,b|scores[*a].0.total_cmp(&scores[*b].0)).unwrap();
    let held_best=(0..scores.len()).min_by(|a,b|scores[*a].1.total_cmp(&scores[*b].1)).unwrap();
    result.candidate_offset_mm=candidates[best];result.paired_rays=errors[16].2+errors[16].3;
    result.fit_error_before=scores[16].0;result.fit_error_after=scores[best].0;
    result.held_error_before=scores[16].1;result.held_error_after=scores[best].1;
    result.accepted=best>0 && best+1<scores.len() && best.abs_diff(held_best)<=2
        && scores[best].0<scores[16].0*0.9 && scores[best].1<scores[16].1*0.9;
    if result.accepted {result.applied_offset_mm=candidates[best];result.reason="bounded conjugate-ray fit; independent held angles improve >10% and agree within two detector pixels".into();}
    else {result.reason="conjugate-ray confidence gate failed; recorded geometry retained".into();}
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalized_filter_preserves_constant_without_mask_bleed() {
        let mut mask=vec![1.;81];mask[40]=0.;
        let input=vec![3.;81];let out=normalized_smooth(&input,&mask,9);
        assert!(out.iter().all(|v|(*v-3.).abs()<1e-5));
    }
    #[test]
    fn correction_marks_censoring_and_keeps_gate_denominator() {
        let len=DETECTOR_N*DETECTOR_N;
        let dark=ReferenceStats {mean:vec![100.;len],variance:vec![81.;len]};
        let flat=ReferenceStats {mean:vec![400.;len],variance:vec![100.;len]};
        let mut raw=vec![250.;len];raw[len/2+DETECTOR_N/2]=99.;
        let c=correct(&raw,16000.,&dark,&flat,60.,1).unwrap();
        assert_eq!(c.quality.nonpositive,1);assert_eq!(c.quality.low_signal,1);
        assert_eq!(c.flags[len/2+DETECTOR_N/2],2);
        assert!(c.projection.iter().all(|v|v.is_finite() && *v<4.));
        assert!(correct(&vec![99.;len],16000.,&dark,&flat,60.,1).is_err());
    }
    #[test]
    fn empty_axis_data_cannot_change_geometry() {
        let e=estimate_axis(&vec![0.;60*DETECTOR_N*DETECTOR_N],&(0..60).map(|i|-(i as f64)*PI/30.).collect::<Vec<_>>(),0.15625,95.,0.5,0.);
        assert!(!e.accepted);assert_eq!(e.applied_offset_mm,0.5);
    }
    #[test]
    fn low_gain_lower_half_and_outer_disk_are_not_cropped() {
        let n=DETECTOR_N;let len=n*n;
        let dark=ReferenceStats {mean:vec![100.;len],variance:vec![1.;len]};
        let flat=ReferenceStats {mean:vec![140.;len],variance:vec![1.;len]};
        let result=correct(&vec![120.;len],16000.,&dark,&flat,60.,1).unwrap();
        // Low gain (40 < 60) is still usable. Sample above/below the center,
        // including radius .96 formerly erased by the .94 disk mask.
        for row in [6,32,128,224,249] {
            let i=row*n+n/2;
            assert_eq!(result.flags[i],5);
            assert!(result.projection[i]>0.5,"row {row} was lost");
        }
        assert!(result.quality.low_gain>len/2);
    }
    #[test]
    fn axis_recovers_analytic_shift_for_both_rotation_signs() {
        let n=DETECTOR_N;let pitch=40./n as f64;let true_offset=5.*pitch;
        for sign in [-1.,1.] {
            let angles:Vec<_>=(0..90).map(|i|sign*i as f64*2.*PI/90.).collect();
            let mut stack=vec![0f32;90*n*n];
            for (k,theta) in angles.iter().enumerate() {
                let (s,c)=theta.sin_cos();
                for col in 0..n {
                    let u=(col as f64-(n-1) as f64/2.)*pitch-true_offset;
                    let norm=95f64.hypot(u);let dx=(-95.*s+u*c)/norm;let dy=(95.*c+u*s)/norm;
                    let sx=70.*s-2.;let sy=-70.*c-1.;
                    let dot=sx*dx+sy*dy;
                    let discriminant=dot*dot-(sx*sx+sy*sy-36.);
                    let value=0.2*discriminant.max(0.).sqrt();
                    for row in 0..n {stack[(k*n+row)*n+col]=value as f32;}
                }
            }
            let e=estimate_axis(&stack,&angles,pitch,95.,0.,0.);
            assert!(e.accepted,"{} {}",e.reason,e.paired_rays);
            assert!((e.applied_offset_mm-true_offset).abs()<=pitch,"{}",e.applied_offset_mm);
        }
    }
}
