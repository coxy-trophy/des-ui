//! Bounded cursor motion history, shared by terminal and desktop renderers.
#[cfg(test)] mod tests {
 use super::*;
 #[test] fn idle_and_disabled_have_no_trail(){let mut s=Smear::default();assert!(s.update(0.0,[0.0,0.0],true).is_empty());assert!(!s.update(0.01,[20.0,0.0],true).is_empty());assert!(s.update(1.0,[20.0,0.0],true).is_empty());assert!(s.update(1.01,[40.0,0.0],false).is_empty());assert!(s.update(1.02,[40.0,0.0],true).is_empty());}
 #[test] fn history_and_alpha_are_bounded(){let mut s=Smear::default();for i in 0..1000{let g=s.update(i as f64*0.001,[i as f64,0.0],true);assert!(g.len()<=8);assert!(g.iter().all(|g|g.alpha>0.0&&g.alpha<=0.35));}}
 #[test] fn teleports_invalid_time_and_invalid_positions_clear_history(){let mut s=Smear::default();s.update(0.0,[0.0,0.0],true);assert!(s.update(0.01,[10000.0,0.0],true).is_empty());assert!(s.update(f64::NAN,[0.0,0.0],true).is_empty());assert!(s.update(1.0,[f64::NAN,0.0],true).is_empty());}
}

#[derive(Clone, Copy, Debug)]
pub struct Ghost { pub position: [f64; 2], pub alpha: f32 }
#[derive(Default)]
pub struct Smear { last: Option<([f64; 2], f64)>, points: std::collections::VecDeque<([f64; 2], f64)> }
impl Smear {
    pub fn clear(&mut self) { self.last=None; self.points.clear(); }
    pub fn update(&mut self, now:f64, position:[f64;2], enabled:bool)->Vec<Ghost> {
        if !enabled || !now.is_finite() || !position.iter().all(|v|v.is_finite()) {self.clear();return vec![];}
        if let Some((old,time))=self.last {
            let d=(old[0]-position[0]).hypot(old[1]-position[1]);
            if now<time || now-time>0.25 || d>512.0 {self.points.clear();}
            else if d>=1.0 {
                let steps=(d/6.0).ceil().clamp(1.0,8.0) as usize;
                for i in 0..steps {
                    let t=i as f64/steps as f64;
                    let point=[old[0]+(position[0]-old[0])*t,old[1]+(position[1]-old[1])*t];
                    if self.points.len()==8 {self.points.pop_front();}
                    self.points.push_back((point,now-(1.0-t)*0.03));
                }
            }
        }
        self.last=Some((position,now));
        self.points.retain(|(_,time)|now-time<0.12);
        self.points.iter().map(|(position,time)|Ghost{position:*position,alpha:(0.35*(1.0-(now-time)/0.12).powi(2)) as f32}).collect()
    }
}

#[cfg(test)] mod movement_tests {
    use super::*;
    #[test] fn a_cursor_jump_leaves_samples_along_the_path() {
        let mut s=Smear::default();s.update(0.0,[0.0,0.0],true);
        let ghosts=s.update(0.01,[80.0,0.0],true);
        assert!(ghosts.len()>1);assert!(ghosts.iter().any(|g|g.position[0]>0.0&&g.position[0]<80.0));
    }
}
