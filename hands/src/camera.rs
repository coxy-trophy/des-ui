//! The webcam, through Video4Linux: 640 by 480, as Motion JPEG where the camera sends
//! it, which most do at full speed, or else as raw YUYV.

use std::io;

use v4l::{FourCC, buffer::Type, io::mmap::Stream, io::traits::CaptureStream, video::Capture};

use crate::geometry::Picture;

const WIDTH: u32 = 640;
const HEIGHT: u32 = 480;

/// Opens the camera at `path` and hands each picture to `each` until it returns false.
pub fn run(path: &str, mut each: impl FnMut(&Picture) -> bool) -> io::Result<()> {
    let device = v4l::Device::with_path(path)?;
    let offered: Vec<FourCC> = device.enum_formats()?.into_iter().map(|format| format.fourcc).collect();
    let mjpeg = FourCC::new(b"MJPG");
    let yuyv = FourCC::new(b"YUYV");
    let wanted = if offered.contains(&mjpeg) {
        mjpeg
    } else if offered.contains(&yuyv) {
        yuyv
    } else {
        return Err(io::Error::other(format!("the camera offers neither MJPG nor YUYV, only {offered:?}")));
    };
    let mut format = device.format()?;
    (format.width, format.height, format.fourcc) = (WIDTH, HEIGHT, wanted);
    let format = device.set_format(&format)?;
    if format.fourcc != wanted {
        return Err(io::Error::other(format!("the camera would not send {wanted}")));
    }
    let (width, height) = (format.width as usize, format.height as usize);
    // As many pictures a second as it offers at this size, up to 30.
    let rates: Vec<f64> = device
        .enum_frameintervals(format.fourcc, format.width, format.height)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|interval| match interval.interval {
            v4l::frameinterval::FrameIntervalEnum::Discrete(fraction) => Some(fraction.denominator as f64 / fraction.numerator.max(1) as f64),
            v4l::frameinterval::FrameIntervalEnum::Stepwise(step) => Some(step.min.denominator as f64 / step.min.numerator.max(1) as f64),
        })
        .collect();
    let best = rates.iter().copied().filter(|rate| *rate <= 30.5).fold(0.0, f64::max);
    if best > 0.0 {
        let _ = device.set_params(&v4l::video::capture::Parameters::with_fps(best.round() as u32));
    }
    let params = device.params()?;
    let rate = params.interval.denominator as f64 / params.interval.numerator.max(1) as f64;
    eprintln!("camera {path}: {width}x{height} {} at {rate:.0} a second (offers {rates:?})", format.fourcc);
    let mut stream = Stream::with_buffers(&device, Type::VideoCapture, 4)?;
    let mut picture = Picture { width, height, rgb: vec![0; width * height * 3] };
    loop {
        let (data, meta) = stream.next()?;
        let data = &data[..(meta.bytesused as usize).min(data.len())];
        let ready = if format.fourcc == mjpeg { from_jpeg(data, &mut picture) } else { from_yuyv(data, &mut picture) };
        // A frame that came damaged is passed over.
        if ready && !each(&picture) {
            return Ok(());
        }
    }
}

fn from_jpeg(data: &[u8], picture: &mut Picture) -> bool {
    let Ok(decoded) = image::load_from_memory_with_format(data, image::ImageFormat::Jpeg) else {
        return false;
    };
    let rgb = decoded.into_rgb8();
    if (rgb.width() as usize, rgb.height() as usize) != (picture.width, picture.height) {
        return false;
    }
    picture.rgb = rgb.into_raw();
    true
}

/// Two pixels in four bytes: their brightnesses and the colour they share.
fn from_yuyv(data: &[u8], picture: &mut Picture) -> bool {
    if data.len() < picture.width * picture.height * 2 {
        return false;
    }
    for (pair, out) in data.chunks_exact(4).zip(picture.rgb.chunks_exact_mut(6)) {
        let (u, v) = (pair[1] as f32 - 128.0, pair[3] as f32 - 128.0);
        for (half, y) in [pair[0], pair[2]].into_iter().enumerate() {
            let y = (y as f32 - 16.0) * 1.164;
            let rgb = [y + 1.596 * v, y - 0.392 * u - 0.813 * v, y + 2.017 * u];
            for (channel, value) in rgb.into_iter().enumerate() {
                out[half * 3 + channel] = value.round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_yuyv() {
        let mut picture = Picture { width: 2, height: 1, rgb: vec![0; 6] };
        // Black, then white, with no colour.
        assert!(from_yuyv(&[16, 128, 235, 128], &mut picture));
        assert_eq!(picture.rgb, [0, 0, 0, 255, 255, 255]);
        assert!(!from_yuyv(&[16, 128], &mut picture), "too short");
    }
}
