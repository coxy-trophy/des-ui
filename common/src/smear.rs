//! Native critically damped cursor deformation. No raster history or cursor copies.
#[derive(Default)]
pub struct Smear {
    time: Option<f64>,
    target: [[f64; 2]; 4],
    corners: [[f64; 2]; 4],
    velocity: [[f64; 2]; 4],
    rates: [f64; 4],
    active: bool,
}
impl Smear {
    pub fn clear(&mut self) { *self=Self::default(); }
    pub fn active(&self)->bool {self.active}
    pub fn quad(&mut self, now:f64, target:[[f64;2];4], enabled:bool)->[[f64;2];4] {
        let invalid=!now.is_finite() || !target.iter().flatten().all(|v|v.is_finite());
        if invalid {self.clear();return target;}
        let jump=(target[0][0]-self.target[0][0]).hypot(target[0][1]-self.target[0][1]);
        let dt=now-self.time.unwrap_or(now);
        if !enabled || self.time.is_none() || !(0.0..=0.25).contains(&dt) || jump>2048.0 {
            self.time=Some(now);self.target=target;self.corners=target;self.velocity=[[0.0;2];4];self.active=false;self.rates=[32.0;4];return target;
        }
        // Exact solution of x'' + 2*w*x' + w*w*(x-target) = 0.
        // Integrate the previous target before applying this frame's new target.
        for i in 0..4 {for axis in 0..2 {
            let w=self.rates[i];let error=self.corners[i][axis]-self.target[i][axis];
            let b=self.velocity[i][axis]+w*error;let decay=(-w*dt).exp();
            self.corners[i][axis]=self.target[i][axis]+(error+b*dt)*decay;
            self.velocity[i][axis]=(self.velocity[i][axis]-w*b*dt)*decay;
        }}
        if self.target!=target {
            let center=[target.iter().map(|p|p[0]).sum::<f64>()/4.0,target.iter().map(|p|p[1]).sum::<f64>()/4.0];
            let distances=self.corners.map(|p|(p[0]-center[0]).hypot(p[1]-center[1]));
            let min=distances.into_iter().fold(f64::INFINITY,f64::min);let max=distances.into_iter().fold(0.0,f64::max);
            for (i,d) in distances.into_iter().enumerate(){let weight=if max-min>0.001{(d-min)/(max-min)}else{0.0};self.rates[i]=46.0-22.0*weight;}
        }
        self.target=target;self.time=Some(now);
        self.active=false;
        for i in 0..4 {for axis in 0..2 {
            // Bound deformation even when a terminal jumps across a large grid.
            self.corners[i][axis]=self.corners[i][axis].clamp(target[i][axis]-384.0,target[i][axis]+384.0);
            if (self.corners[i][axis]-target[i][axis]).abs()>0.04 || self.velocity[i][axis].abs()>0.4 {self.active=true;}
        }}
        if !self.active {self.corners=target;self.velocity=[[0.0;2];4];}
        self.corners
    }
}
#[cfg(test)] mod tests {
 use super::*;
 fn rect(x:f64)->[[f64;2];4]{[[x,0.0],[x+10.0,0.0],[x+10.0,20.0],[x,20.0]]}
 #[test] fn moves_continuously_and_settles_without_idle_work(){let mut s=Smear::default();s.quad(0.0,rect(0.0),true);assert_eq!(s.quad(0.01,rect(100.0),true),rect(0.0));let q=s.quad(0.026,rect(100.0),true);assert!(q[0][0]>0.0&&q[0][0]<100.0);assert!(s.active());for i in 1..100{s.quad(0.026+i as f64/60.0,rect(100.0),true);}assert!(!s.active());}
 #[test] fn corners_stretch_instead_of_producing_cursor_copies(){let mut s=Smear::default();s.quad(0.0,rect(0.0),true);s.quad(0.01,rect(100.0),true);let q=s.quad(0.04,rect(100.0),true);assert!(q[1][0]-q[0][0]>10.0);}
 #[test] fn frame_rate_does_not_change_the_motion(){let run=|hz:u32|{let mut s=Smear::default();s.quad(0.0,rect(0.0),true);s.quad(0.01,rect(100.0),true);let mut q=rect(0.0);for i in 1..=hz/10{q=s.quad(0.01+i as f64/hz as f64,rect(100.0),true);}q};let a=run(60);let b=run(120);for i in 0..4{for j in 0..2{assert!((a[i][j]-b[i][j]).abs()<0.0001);}}}
 #[test] fn disable_time_gaps_and_teleports_snap_cleanly(){let mut s=Smear::default();s.quad(0.0,rect(0.0),true);assert_eq!(s.quad(0.01,rect(100.0),false),rect(100.0));assert!(!s.active());assert_eq!(s.quad(2.0,rect(200.0),true),rect(200.0));assert_eq!(s.quad(2.01,rect(10000.0),true),rect(10000.0));}
}
