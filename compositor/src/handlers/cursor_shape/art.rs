//! Small vector silhouettes rasterized at the output's pixel density. Every
//! outline has the same winding and number of boundary samples for interpolation.
use super::Shape;
use des_common::input::{CURSOR_ARROW_POINTS, CURSOR_BASE_SIZE};
#[cfg(test)]
use des_common::input::DEFAULT_CURSOR_COLOR;

pub(super) const SIDE: usize = 48;
pub(super) const HOTSPOT: (i32, i32) = (16, 16);
pub(super) const SAMPLES: usize = 64;
pub(super) type Boundary = [[f32; 2]; SAMPLES];

fn rotate(point: [f32; 2], angle: f32) -> [f32; 2] {
    let (sin, cos) = angle.sin_cos();
    let (x, y) = (point[0] - 16.0, point[1] - 16.0);
    [16.0 + x * cos - y * sin, 16.0 + x * sin + y * cos]
}

fn mix(a: [f32; 2], b: [f32; 2], t: f32) -> [f32; 2] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

fn rounded(vertices: &[[f32; 2]], rounding: f32) -> Vec<[f32; 2]> {
    if rounding == 0.0 { return vertices.to_vec(); }
    let mut path = Vec::with_capacity(vertices.len() * 8);
    for (i, corner) in vertices.iter().copied().enumerate() {
        let a = mix(corner, vertices[(i + vertices.len() - 1) % vertices.len()], rounding);
        let b = mix(corner, vertices[(i + 1) % vertices.len()], rounding);
        for step in 0..8 {
            let t = step as f32 / 7.0;
            path.push(mix(mix(a, corner, t), mix(corner, b, t), t));
        }
    }
    path
}

fn resample(path: &[[f32; 2]]) -> Boundary {
    let lengths: Vec<f32> = path.iter().zip(path.iter().cycle().skip(1)).take(path.len())
        .map(|(a, b)| (b[0] - a[0]).hypot(b[1] - a[1])).collect();
    let perimeter: f32 = lengths.iter().sum();
    let (mut edge, mut passed) = (0, 0.0);
    std::array::from_fn(|i| {
        let at = perimeter * i as f32 / SAMPLES as f32;
        while edge + 1 < path.len() && passed + lengths[edge] < at {
            passed += lengths[edge];
            edge += 1;
        }
        mix(path[edge], path[(edge + 1) % path.len()], (at - passed) / lengths[edge].max(1e-6))
    })
}

fn disk(radius: f32) -> Vec<[f32; 2]> {
    (0..SAMPLES).map(|i| {
        let a = i as f32 * std::f32::consts::TAU / SAMPLES as f32 - std::f32::consts::FRAC_PI_2;
        [16.0 + radius * a.cos(), 16.0 + radius * a.sin()]
    }).collect()
}

/// All artwork stays comfortably inside the transparent canvas border. The
/// default is a convex, solid scalene triangle: no tail or notch is cut into it.
pub(super) fn boundary(shape: Shape, turn: f32) -> Boundary {
    let mut angle = 0.0;
    let path = match shape {
        Shape::Arrow => rounded(&CURSOR_ARROW_POINTS.map(|point| [point[0]+HOTSPOT.0 as f32,point[1]+HOTSPOT.1 as f32]),0.0),
        Shape::Link => rounded(&[[15.0, 15.0], [30.0, 29.0], [19.0, 34.0]], 0.09),
        Shape::Beam | Shape::VerticalBeam => {
            if shape == Shape::VerticalBeam { angle = std::f32::consts::FRAC_PI_2; }
            rounded(&[[15.0, 4.0], [18.0, 4.0], [18.0, 25.0], [19.0, 27.0], [13.0, 27.0], [15.0, 25.0]], 0.025)
        }
        Shape::Grab | Shape::Grabbing => {
            angle = -std::f32::consts::FRAC_PI_4;
            let mut lozenge = Vec::with_capacity(64);
            for (a, b, c, d) in [
                ([16.0, 7.0], [22.0, 12.0], [24.0, 23.0], [16.0, 24.0]),
                ([16.0, 24.0], [8.0, 23.0], [10.0, 12.0], [16.0, 7.0]),
            ] {
                for step in 0..32 {
                    let t = step as f32 / 32.0;
                    let e = mix(a, b, t); let f = mix(b, c, t); let g = mix(c, d, t);
                    let mut point = mix(mix(e, f, t), mix(f, g, t), t);
                    if shape == Shape::Grabbing {
                        point = [16.0+(point[0]-16.0)*0.92, 16.0+(point[1]-16.0)*0.82];
                    }
                    lozenge.push(point);
                }
            }
            lozenge
        }
        Shape::ResizeHorizontal | Shape::ResizeVertical | Shape::ResizeNwSe | Shape::ResizeNeSw => {
            angle = match shape {
                Shape::ResizeVertical => std::f32::consts::FRAC_PI_2,
                Shape::ResizeNwSe => std::f32::consts::FRAC_PI_4,
                Shape::ResizeNeSw => -std::f32::consts::FRAC_PI_4,
                _ => 0.0,
            };
            rounded(&[[3.0, 16.0], [16.0, 13.5], [29.0, 16.0], [16.0, 18.5]], 0.0)
        }
        Shape::Direction(direction) => {
            angle = direction as f32 * std::f32::consts::FRAC_PI_4;
            rounded(&[[29.0, 16.0], [8.0, 19.5], [8.0, 12.5]], 0.0)
        }
        Shape::Working => {
            angle = -std::f32::consts::FRAC_PI_4 + turn;
            let mut seed = Vec::with_capacity(64);
            for (a, b, c, d) in [
                ([16.0, 4.5], [25.0, 12.0], [27.0, 25.0], [16.0, 26.0]),
                ([16.0, 26.0], [5.0, 25.0], [7.0, 12.0], [16.0, 4.5]),
            ] {
                for step in 0..32 {
                    let t = step as f32 / 32.0;
                    let e = mix(a, b, t); let f = mix(b, c, t); let g = mix(c, d, t);
                    seed.push(mix(mix(e, f, t), mix(f, g, t), t));
                }
            }
            seed
        }
        Shape::Forbidden | Shape::ZoomIn | Shape::ZoomOut => disk(10.0),
        Shape::Copy | Shape::Alias | Shape::Help | Shape::Menu => {
            rounded(&[[16.0, 4.0], [26.0, 14.0], [24.0, 27.0], [8.0, 27.0], [6.0, 14.0]], 0.20)
        }
        Shape::Crosshair | Shape::Move => {
            let w = if shape == Shape::Move { 2.6 } else { 1.6 };
            rounded(&[[16.0-w, 4.0], [16.0+w, 4.0], [16.0+w, 16.0-w], [28.0, 16.0-w], [28.0, 16.0+w], [16.0+w, 16.0+w], [16.0+w, 28.0], [16.0-w, 28.0], [16.0-w, 16.0+w], [4.0, 16.0+w], [4.0, 16.0-w], [16.0-w, 16.0-w]], 0.0)
        }
        Shape::Cell => rounded(&[[7.0, 7.0], [25.0, 7.0], [25.0, 25.0], [7.0, 25.0]], 0.08),
    };
    let mut result = resample(&path);
    if angle != 0.0 { result.iter_mut().for_each(|p| *p = rotate(*p, angle)); }
    result
}

/// Match nearby perimeter points before a morph. Without cyclic alignment,
/// opposite directional silhouettes pair each tip with its antipode and collapse
/// to a point halfway through an otherwise perfectly valid interpolation.
pub(super) fn alignment(from: &Boundary, to: &Boundary) -> usize {
    (0..SAMPLES).min_by(|a, b| {
        let cost = |offset: usize| from.iter().enumerate().map(|(i, p)| {
            let q = to[(i+offset)%SAMPLES];
            (p[0]-q[0]).powi(2)+(p[1]-q[1]).powi(2)
        }).sum::<f32>();
        cost(*a).total_cmp(&cost(*b))
    }).unwrap_or(0)
}

pub(super) fn interpolate(from: &Boundary, to: &Boundary, t: f32) -> Boundary {
    std::array::from_fn(|i| mix(from[i], to[i], t))
}

/// Per-pass coefficients keep divisions and endpoint setup out of the coverage
/// loop. The expanded bounds reject segments too far away to affect either ink
/// threshold; distances beyond one canonical pixel all mean plain body fill.
#[derive(Clone,Copy)]
struct Edge {
    ax:f32, ay:f32, by:f32, dx:f32, dy:f32,
    length_squared:f32, inverse_length_squared:f32, inverse_dy:f32,
    min_x:f32, max_x:f32, min_y:f32, max_y:f32,
}

fn edges(boundary:&Boundary)->[Edge;SAMPLES] {
    std::array::from_fn(|i| {
        let (a,b)=(boundary[i],boundary[(i+1)%SAMPLES]);
        let (dx,dy)=(b[0]-a[0],b[1]-a[1]);
        let length_squared=(dx*dx+dy*dy).max(1e-8);
        Edge {ax:a[0],ay:a[1],by:b[1],dx,dy,length_squared,
            inverse_length_squared:1.0/length_squared,
            inverse_dy:if dy==0.0 {0.0}else{1.0/dy},
            min_x:a[0].min(b[0])-1.0,max_x:a[0].max(b[0])+1.0,
            min_y:a[1].min(b[1])-1.0,max_y:a[1].max(b[1])+1.0}
    })
}

const GRID_SIDE:usize=SIDE/2;

struct Coverage {
    edges:[Edge;SAMPLES],
    nearby:Vec<Vec<usize>>,
}

impl Coverage {
    fn new(boundary:&Boundary)->Self {
        let edges=edges(boundary);
        let mut nearby=vec![Vec::new();GRID_SIDE*GRID_SIDE];
        let cell=|v:f32| ((v*0.5).floor().max(0.0) as usize).min(GRID_SIDE-1);
        for (index,edge) in edges.iter().enumerate() {
            for y in cell(edge.min_y)..=cell(edge.max_y) {
                for x in cell(edge.min_x)..=cell(edge.max_x) {nearby[y*GRID_SIDE+x].push(index);}
            }
        }
        Self {edges,nearby}
    }

    fn scanline(&self,y:f32)->Vec<(f32,f32)> {
        self.edges.iter().filter(|edge|(edge.ay>y)!=(edge.by>y)).map(|edge| {
            let product=edge.dx*(y-edge.ay);
            // One exact crossing per edge/row retains reference rounding; the
            // reciprocal result handles ordinary samples in the hot loop.
            (product*edge.inverse_dy+edge.ax,product/edge.dy+edge.ax)
        }).collect()
    }
}

#[inline(always)]
fn inside_distance(p:[f32;2],coverage:&Coverage,crossings:&[(f32,f32)])->Option<f32> {
    let mut inside=false;
    for &(approximate,exact) in crossings {
        let crossing=if (p[0]-approximate).abs()<0.0001 {exact}else{approximate};
        if p[0]<crossing {inside=!inside;}
    }
    if !inside {return None;}
    let x=((p[0]*0.5).floor().max(0.0) as usize).min(GRID_SIDE-1);
    let y=((p[1]*0.5).floor().max(0.0) as usize).min(GRID_SIDE-1);
    let mut distance=1.0_f32;
    for &index in &coverage.nearby[y*GRID_SIDE+x] {
        let edge=&coverage.edges[index];
        if p[0]<edge.min_x||p[0]>edge.max_x||p[1]<edge.min_y||p[1]>edge.max_y {continue;}
        let (px,py)=(p[0]-edge.ax,p[1]-edge.ay);
        let projection=px*edge.dx+py*edge.dy;
        let mut t=(projection*edge.inverse_length_squared).clamp(0.0,1.0);
        let (mut x,mut y)=(p[0]-(edge.ax+edge.dx*t),p[1]-(edge.ay+edge.dy*t));
        let mut squared=x*x+y*y;
        // Reciprocal multiplication differs by a few ULPs from division. Only
        // distances touching an ink threshold need the reference arithmetic.
        if (squared-0.0225).abs()<0.0001||(squared-0.7225).abs()<0.0001||(squared-0.9025).abs()<0.0001 {
            t=(projection/edge.length_squared).clamp(0.0,1.0);
            x=p[0]-(edge.ax+edge.dx*t);y=p[1]-(edge.ay+edge.dy*t);
            squared=x*x+y*y;
        }
        distance=distance.min(squared);
    }
    Some(distance)
}

/// Dark details are drawn inside a single opaque silhouette, never as detached
/// badges. They preserve forbidden, copy, alias and zoom semantics without color.
fn detail(p: [f32; 2], shape: Shape) -> bool {
    let (x, y) = (p[0] - 16.0, p[1] - 16.0);
    let horizontal = y.abs() < 0.85 && x.abs() < 5.0;
    let vertical = x.abs() < 0.85 && y.abs() < 5.0;
    match shape {
        Shape::Forbidden => (x+y).abs() < 1.4,
        Shape::Copy | Shape::ZoomIn | Shape::Cell => horizontal || vertical,
        Shape::ZoomOut => horizontal,
        Shape::Alias => (y-2.0).abs() < 0.9 && (-5.0..=4.0).contains(&x)
            || (x-4.0).abs() < 0.9 && (-4.0..=2.0).contains(&y)
            || ((y+4.0).abs() < 0.85 && (1.0..=4.0).contains(&x)),
        Shape::Menu => x.abs() < 4.5 && [-3.0_f32, 0.0, 3.0].iter().any(|line| (y-line).abs() < 0.7),
        Shape::Help => ((x*x + (y+2.0).powi(2)).sqrt() - 3.0).abs() < 0.8 && y < -1.0
            || (x.abs() < 0.8 && (-2.0..=1.0).contains(&y)) || (x*x+(y-4.0).powi(2) < 1.0),
        _ => false,
    }
}

/// The canonical click point maps exactly to the integer hotspot returned to
/// rendering backends. Scaling relative offsets avoids fractional-anchor drift.
pub(super) fn logical_point(point: [f32;2], size: u32) -> [f32;2] {
    let factor=size as f32/CURSOR_BASE_SIZE as f32;
    let hotspot=(HOTSPOT.0 as f32*factor).round();
    point.map(|coordinate| hotspot+(coordinate-HOTSPOT.0 as f32)*factor)
}

/// Four coverage samples per physical pixel, premultiplied ARGB8888. Work is
/// limited to the silhouette's bounding box; the rest stays fully transparent.
#[cfg(test)]
pub(super) fn raster(boundary: &Boundary, shape: Shape, density: usize, detail_alpha: f32) -> Vec<u8> {
    raster_at(boundary, shape, density as f32, detail_alpha)
}

#[cfg(test)]
pub(super) fn raster_at(boundary: &Boundary, shape: Shape, density: f32, detail_alpha: f32) -> Vec<u8> {
    raster_custom(boundary,shape,density,CURSOR_BASE_SIZE,DEFAULT_CURSOR_COLOR,detail_alpha)
}

pub(super) fn raster_custom(boundary: &Boundary, shape: Shape, density: f32, size: u32, color: [u8;3], detail_alpha: f32) -> Vec<u8> {
    let coverage=Coverage::new(boundary);
    let factor=size as f32/CURSOR_BASE_SIZE as f32;
    let hotspot=(HOTSPOT.0 as f32*factor).round();
    let side=(size as f32*2.0*density).ceil() as usize;
    let mut pixels=vec![0;side*side*4];
    let logical=boundary.map(|point|logical_point(point,size));
    let min_x=(logical.iter().map(|p|p[0]).fold(f32::INFINITY,f32::min).max(0.0)*density).floor() as usize;
    let max_x=((logical.iter().map(|p|p[0]).fold(0.0,f32::max)*density).ceil() as usize).min(side);
    let min_y=(logical.iter().map(|p|p[1]).fold(f32::INFINITY,f32::min).max(0.0)*density).floor() as usize;
    let max_y=((logical.iter().map(|p|p[1]).fold(0.0,f32::max)*density).ceil() as usize).min(side);
    let dark_fill=0.2126*color[0] as f32+0.7152*color[1] as f32+0.0722*(color[2] as f32)<128.0;
    let contrast=if dark_fill {[247.0,246.0,242.0]}else{[29.0,31.0,33.0]};
    for y in min_y..max_y {
        let sample_y=[((y as f32+0.25)/density-hotspot)/factor+16.0,((y as f32+0.75)/density-hotspot)/factor+16.0];
        let crossings=[coverage.scanline(sample_y[0]),coverage.scanline(sample_y[1])];
        for x in min_x..max_x {
            let mut sum=[0.0;4];
            for (sx,row) in [(0.25,0_usize),(0.75,0),(0.25,1),(0.75,1)] {
                let p=[((x as f32+sx)/density-hotspot)/factor+16.0,sample_y[row]];
                if let Some(distance)=inside_distance(p,&coverage,&crossings[row]) {
                    // A dark fill gets a light keyline plus a restrained dark
                    // outer hint. The two edges retain contrast on either ground.
                    let outer=dark_fill&&distance<0.15*0.15;
                    let ink=if distance<if dark_fill {0.95*0.95}else{0.85*0.85} {1.0}else if detail(p,shape){detail_alpha}else{0.0};
                    for channel in 0..3 {
                        let body=color[2-channel] as f32;
                        sum[channel]+=if outer {[29.0,31.0,33.0][2-channel]}else{body+(contrast[2-channel]-body)*ink};
                    }
                    sum[3]+=255.0;
                }
            }
            let at=(y*side+x)*4;
            for channel in 0..4 {pixels[at+channel]=(sum[channel]/4.0).round() as u8;}
        }
    }
    pixels
}

#[cfg(test)]
mod raster_tests {
    use super::*;
fn reference_segment_distance_squared(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let d = [b[0] - a[0], b[1] - a[1]];
    let t = (((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / (d[0]*d[0] + d[1]*d[1]).max(1e-8)).clamp(0.0, 1.0);
    let near = mix(a, b, t);
    (p[0] - near[0]).powi(2) + (p[1] - near[1]).powi(2)
}

fn reference_inside_distance(p: [f32; 2], boundary: &Boundary) -> Option<f32> {
    let mut inside = false;
    let mut distance = f32::MAX;
    for i in 0..SAMPLES {
        let (a, b) = (boundary[i], boundary[(i+1)%SAMPLES]);
        if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < (b[0]-a[0]) * (p[1]-a[1]) / (b[1]-a[1]) + a[0] {
            inside = !inside;
        }
        distance = distance.min(reference_segment_distance_squared(p, a, b));
    }
    inside.then_some(distance)
}

fn reference_raster(boundary: &Boundary, shape: Shape, density: f32, size: u32, color: [u8;3], detail_alpha: f32) -> Vec<u8> {
    let factor=size as f32/CURSOR_BASE_SIZE as f32;
    let hotspot=(HOTSPOT.0 as f32*factor).round();
    let side=(size as f32*2.0*density).ceil() as usize;
    let mut pixels=vec![0;side*side*4];
    let logical=boundary.map(|point|logical_point(point,size));
    let min_x=(logical.iter().map(|p|p[0]).fold(f32::INFINITY,f32::min).max(0.0)*density).floor() as usize;
    let max_x=((logical.iter().map(|p|p[0]).fold(0.0,f32::max)*density).ceil() as usize).min(side);
    let min_y=(logical.iter().map(|p|p[1]).fold(f32::INFINITY,f32::min).max(0.0)*density).floor() as usize;
    let max_y=((logical.iter().map(|p|p[1]).fold(0.0,f32::max)*density).ceil() as usize).min(side);
    let dark_fill=0.2126*color[0] as f32+0.7152*color[1] as f32+0.0722*(color[2] as f32)<128.0;
    let contrast=if dark_fill {[247.0,246.0,242.0]}else{[29.0,31.0,33.0]};
    for y in min_y..max_y {
        for x in min_x..max_x {
            let mut sum=[0.0;4];
            for (sx,sy) in [(0.25,0.25),(0.75,0.25),(0.25,0.75),(0.75,0.75)] {
                let p=[((x as f32+sx)/density-hotspot)/factor+16.0,((y as f32+sy)/density-hotspot)/factor+16.0];
                if let Some(distance)=reference_inside_distance(p,boundary) {
                    // A dark fill gets a light keyline plus a restrained dark
                    // outer hint. The two edges retain contrast on either ground.
                    let outer=dark_fill&&distance<0.15*0.15;
                    let ink=if distance<if dark_fill {0.95*0.95}else{0.85*0.85} {1.0}else if detail(p,shape){detail_alpha}else{0.0};
                    for channel in 0..3 {
                        let body=color[2-channel] as f32;
                        sum[channel]+=if outer {[29.0,31.0,33.0][2-channel]}else{body+(contrast[2-channel]-body)*ink};
                    }
                    sum[3]+=255.0;
                }
            }
            let at=(y*side+x)*4;
            for channel in 0..4 {pixels[at+channel]=(sum[channel]/4.0).round() as u8;}
        }
    }
    pixels
}

    #[test]
    fn precomputed_edges_preserve_reference_raster_bytes_and_report_cost() {
        for (shape,size,density,color) in [
            (Shape::Arrow,24,1.0,[247,246,242]),
            (Shape::Beam,32,2.0,[0,0,0]),
            (Shape::Forbidden,16,3.0,[255,255,255]),
            (Shape::Grab,40,1.0,[12,99,202]),
            (Shape::ResizeNeSw,24,2.0,[255,0,128]),
        ] {
            let outline=boundary(shape,0.0);
            assert_eq!(raster_custom(&outline,shape,density,size,color,1.0),reference_raster(&outline,shape,density,size,color,1.0),"{shape:?} {size} {density}");
        }
        let outline=boundary(Shape::Arrow,0.0);
        let began=std::time::Instant::now();
        let reference=reference_raster(&outline,Shape::Arrow,4.0,64,DEFAULT_CURSOR_COLOR,1.0);
        let old_time=began.elapsed();
        let began=std::time::Instant::now();
        let optimized=raster_custom(&outline,Shape::Arrow,4.0,64,DEFAULT_CURSOR_COLOR,1.0);
        let new_time=began.elapsed();
        assert_eq!(optimized,reference);
        eprintln!("64px arrow at 4x: reference {old_time:?}, optimized {new_time:?}, {:.2}x speedup",old_time.as_secs_f64()/new_time.as_secs_f64());
    }
}
