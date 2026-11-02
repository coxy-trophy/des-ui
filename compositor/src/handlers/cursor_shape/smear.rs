//! A single antialiased GPU quad driven by the shared corner springs.
use super::*;
use smithay::backend::renderer::{element::Kind,gles::{GlesRenderer,Uniform,UniformName,UniformType,UniformValue,element::PixelShaderElement}};
use smithay::utils::Rectangle;
const SHADER:&str=r#"
precision highp float;
varying vec2 v_coords;
uniform vec2 extent;
uniform vec2 p0; uniform vec2 p1; uniform vec2 p2; uniform vec2 p3;
uniform vec3 rgb; uniform float alpha; uniform float softness;
float cross2(vec2 a,vec2 b){return a.x*b.y-a.y*b.x;}
float edge(vec2 p,vec2 a,vec2 b,float signum){return signum*cross2(b-a,p-a)/max(length(b-a),0.001);}
float triangle(vec2 p,vec2 a,vec2 b,vec2 c){float s=cross2(b-a,c-a)>=0.0?1.0:-1.0;return min(edge(p,a,b,s),min(edge(p,b,c,s),edge(p,c,a,s)));}
void main(){vec2 p=v_coords*extent;float first=smoothstep(-softness,softness,triangle(p,p0,p1,p2));float second=smoothstep(-softness,softness,triangle(p,p0,p2,p3));float a=min(1.0,first+second)*alpha;gl_FragColor=vec4(rgb*a,a);}
"#;
impl Cursors {
 pub fn smear_element(&mut self,renderer:&mut GlesRenderer,position:Point<f64,Logical>,origin:Point<f64,Logical>,now:Instant,enabled:bool,scale:f64)->Option<PixelShaderElement>{
    let width=self.preferences.size as f64*0.55;
    let x=position.x+2.0;let y=position.y+2.0;
    let mut quad=self.smear.quad(now.saturating_duration_since(self.smear_epoch).as_secs_f64(),[[x,y],[x+width,y],[x+width,y+width],[x,y+width]],enabled);
    if !enabled || self.smear_shader_failed{return None;}
    if self.smear_program.is_none(){
        let names=[UniformName::new("extent",UniformType::_2f),UniformName::new("p0",UniformType::_2f),UniformName::new("p1",UniformType::_2f),UniformName::new("p2",UniformType::_2f),UniformName::new("p3",UniformType::_2f),UniformName::new("rgb",UniformType::_3f),UniformName::new("softness",UniformType::_1f)];
        match renderer.compile_custom_pixel_shader(SHADER,&names){Ok(p)=>self.smear_program=Some(p),Err(e)=>{tracing::warn!("cursor smear shader unavailable: {e}");self.smear_shader_failed=true;return None;}}
    }
    if !self.smear.active() {return None;}
    // Keep the leading edge attached to the real pointer; only the trailing edge
    // lags. A moving mouse must leave a connected smear, not a chasing box.
    attach_head(&mut quad, [[x,y],[x+width,y],[x+width,y+width],[x,y+width]]);
    let left=quad.iter().map(|p|p[0]-origin.x).fold(f64::INFINITY,f64::min).floor() as i32-2;
    let top=quad.iter().map(|p|p[1]-origin.y).fold(f64::INFINITY,f64::min).floor() as i32-2;
    let right=quad.iter().map(|p|p[0]-origin.x).fold(f64::NEG_INFINITY,f64::max).ceil() as i32+2;
    let bottom=quad.iter().map(|p|p[1]-origin.y).fold(f64::NEG_INFINITY,f64::max).ceil() as i32+2;
    let area=Rectangle::new((left,top).into(),((right-left).max(1),(bottom-top).max(1)).into());
    let points=quad.map(|p|[(p[0]-origin.x-left as f64) as f32,(p[1]-origin.y-top as f64) as f32]);
    let c=self.preferences.color.map(|v|v as f32/255.0);
    let distance=quad.iter().enumerate().map(|(i,p)|{let target=if i==0{[x,y]}else if i==1{[x+width,y]}else if i==2{[x+width,y+width]}else{[x,y+width]};(p[0]-target[0]).hypot(p[1]-target[1])}).fold(0.0,f64::max);
    let opacity=(distance/width).clamp(0.0,0.55) as f32;
    Some(PixelShaderElement::new(self.smear_program.clone()?,area,None,opacity,vec![Uniform::new("extent",UniformValue::_2f(area.size.w as f32,area.size.h as f32)),Uniform::new("p0",UniformValue::_2f(points[0][0],points[0][1])),Uniform::new("p1",UniformValue::_2f(points[1][0],points[1][1])),Uniform::new("p2",UniformValue::_2f(points[2][0],points[2][1])),Uniform::new("p3",UniformValue::_2f(points[3][0],points[3][1])),Uniform::new("rgb",UniformValue::_3f(c[0],c[1],c[2])),Uniform::new("softness",UniformValue::_1f((1.0/scale.max(1.0)) as f32))],Kind::Cursor))
 }
}

fn attach_head(quad:&mut [[f64;2];4],target:[[f64;2];4]) {
    let direction=[target.iter().map(|p|p[0]).sum::<f64>()-quad.iter().map(|p|p[0]).sum::<f64>(),target.iter().map(|p|p[1]).sum::<f64>()-quad.iter().map(|p|p[1]).sum::<f64>()];
    let mut order=[0,1,2,3];
    order.sort_by(|a,b| {let dot=|i:usize|target[i][0]*direction[0]+target[i][1]*direction[1];dot(*b).total_cmp(&dot(*a))});
    for i in &order[..2] {quad[*i]=target[*i];}
}
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn mouse_smear_remains_attached_to_the_pointer() {
        let mut quad=[[0.0,0.0],[10.0,0.0],[10.0,10.0],[0.0,10.0]];
        let target=[[100.0,0.0],[110.0,0.0],[110.0,10.0],[100.0,10.0]];
        attach_head(&mut quad,target);
        assert_eq!(quad[1],target[1]);assert_eq!(quad[2],target[2]);
        assert_eq!(quad[0],[0.0,0.0]);assert_eq!(quad[3],[0.0,10.0]);
    }
}
