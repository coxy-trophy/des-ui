//! Hover intent for the dock's system-panel peek. App tiles never arm expansion.
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn app_icons_never_open_the_system_panel(){let mut h=Hover::default();assert!(!h.update(0.0,true,false,true,true));assert!(!h.update(1.0,true,false,true,true));}
    #[test] fn empty_rail_hover_opens_after_a_delay_and_panel_hover_keeps_it_open(){let mut h=Hover::default();assert!(!h.update(0.0,true,false,false,true));assert!(h.update(0.23,true,false,false,true));assert!(h.update(1.0,false,true,false,true));}
    #[test] fn leaving_or_entering_an_icon_closes_after_a_grace_period(){let mut h=Hover::default();h.update(0.0,true,false,false,true);h.update(0.3,true,false,false,true);assert!(h.update(0.4,false,false,true,true));assert!(!h.update(0.7,false,false,true,true));}
    #[test] fn blocked_modes_close_immediately_and_forget_pending_hover(){let mut h=Hover::default();h.update(0.0,true,false,false,true);assert!(!h.update(0.3,true,false,false,false));assert!(!h.update(1.0,true,false,false,true));}
}

#[derive(Default)]
pub struct Hover { open:bool, entered:Option<f64>, left:Option<f64> }
impl Hover {
    pub fn is_open(&self)->bool {self.open}
    pub fn pending(&self)->bool {self.entered.is_some() || self.left.is_some()}
    pub fn update(&mut self,now:f64,trigger:bool,inside:bool,icon:bool,allowed:bool)->bool {
        if !allowed || !now.is_finite() {*self=Self::default();return false;}
        if !icon && (trigger || (self.open&&inside)) {
            self.left=None;
            if !self.open {let at=*self.entered.get_or_insert(now);if now-at>=0.22 {self.open=true;self.entered=None;}}
        } else {
            self.entered=None;
            if self.open {let at=*self.left.get_or_insert(now);if now-at>=0.25 {self.open=false;self.left=None;}}
        }
        self.open
    }
}
