//! Phosphor: what is drawn passes once more through a shader before it reaches the
//! display. It can glow (bright pixels bleed light tinted toward the theme's accent),
//! show faint scanlines, and warm the colours at night.
//!
//! Each display's frame is drawn off screen first, then shown through the shader in a
//! single pass. With no glow, no scanlines and daylight colours there is nothing to
//! do, and the pass is skipped altogether: by default it costs nothing. Screenshots and
//! recordings draw the display afresh without it, so captures stay clean.

use std::collections::HashMap;
use std::time::Instant;

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            Bind, Offscreen, Renderer,
            damage::OutputDamageTracker,
            element::{Id, Kind, RenderElement, texture::TextureRenderElement},
            gles::{
                GlesRenderer, GlesTexProgram, GlesTexture, Uniform, UniformName, UniformType, UniformValue,
                element::TextureShaderElement,
            },
        },
    },
    utils::{Logical, Physical, Size, Transform},
};

/// Colours as they are by day: the night light leaves them alone.
pub const DAYLIGHT: f32 = 6500.0;
/// How long the night light takes to come on or go off, in seconds.
const RAMP_SECONDS: f32 = 60.0;

/// The one pass: the frame, plus light bled from its bright parts, darkened every other
/// line, then multiplied by the night light's colour. The layout follows Smithay's own
/// texture shader, whose `//_DEFINES_`, `tex`, `alpha` and `v_coords` it relies on.
pub const SHADER: &str = r#"#version 100

//_DEFINES_

#if defined(EXTERNAL)
#extension GL_OES_EGL_image_external : require
#endif

precision mediump float;
#if defined(EXTERNAL)
uniform samplerExternalOES tex;
#else
uniform sampler2D tex;
#endif

uniform float alpha;
varying vec2 v_coords;

#if defined(DEBUG_FLAGS)
uniform float tint;
#endif

uniform float glow;
uniform float scanlines;
uniform vec3 temperature;
uniform vec3 accent;
uniform vec2 texel;
uniform float line_px;

vec3 bright(vec2 offset) {
    vec3 c = texture2D(tex, v_coords + offset * texel).rgb;
    float luma = dot(c, vec3(0.2126, 0.7152, 0.0722));
    return c * smoothstep(0.55, 0.9, luma);
}

void main() {
    vec4 color = texture2D(tex, v_coords);

    if (glow > 0.0) {
        // Two rings of eight taps around the pixel: a cheap, soft halo.
        vec3 halo = bright(vec2(2.0, 0.0)) + bright(vec2(-2.0, 0.0))
            + bright(vec2(0.0, 2.0)) + bright(vec2(0.0, -2.0))
            + bright(vec2(1.4, 1.4)) + bright(vec2(-1.4, 1.4))
            + bright(vec2(1.4, -1.4)) + bright(vec2(-1.4, -1.4));
        halo += 0.5 * (bright(vec2(5.0, 0.0)) + bright(vec2(-5.0, 0.0))
            + bright(vec2(0.0, 5.0)) + bright(vec2(0.0, -5.0))
            + bright(vec2(3.5, 3.5)) + bright(vec2(-3.5, 3.5))
            + bright(vec2(3.5, -3.5)) + bright(vec2(-3.5, -3.5)));
        halo /= 12.0;
        // Half the halo keeps its own colour, half takes the accent's.
        vec3 tinted = mix(halo, accent * dot(halo, vec3(0.3333)), 0.5);
        color.rgb = min(color.rgb + tinted * glow, vec3(1.0));
    }

    if (scanlines > 0.0) {
        float row = mod(floor(gl_FragCoord.y / line_px), 2.0);
        color.rgb *= 1.0 - 0.12 * row;
    }

    color.rgb *= temperature;

#if defined(NO_ALPHA)
    color = vec4(color.rgb, 1.0) * alpha;
#else
    color = color * alpha;
#endif

#if defined(DEBUG_FLAGS)
    if (tint == 1.0)
        color = vec4(0.0, 0.2, 0.0, 0.2) + color * 0.8;
#endif

    gl_FragColor = color;
}
"#;

/// When the night light is on: from `start` to `end`, minutes after midnight, at
/// `kelvin`. The window may run past midnight.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NightLight {
    pub start: u32,
    pub end: u32,
    pub kelvin: f32,
}

/// What the user asked for, from the shared configuration file.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// 0 for none, 1 for the most.
    pub glow: f32,
    pub scanlines: bool,
    pub night: Option<NightLight>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { glow: 0.0, scanlines: false, night: None }
    }
}

/// `HH:MM` as minutes after midnight.
fn clock(text: &str) -> Option<u32> {
    let (hours, minutes) = text.trim().split_once(':')?;
    let (hours, minutes) = (hours.parse::<u32>().ok()?, minutes.parse::<u32>().ok()?);
    (hours < 24 && minutes < 60).then_some(hours * 60 + minutes)
}

/// `19:00-07:00 4500`: on from seven in the evening to seven in the morning, at 4500 K.
pub fn parse_night(text: &str) -> Option<NightLight> {
    let (window, kelvin) = text.trim().split_once(char::is_whitespace)?;
    let (start, end) = window.split_once('-')?;
    let (start, end) = (clock(start)?, clock(end)?);
    let kelvin = kelvin.trim().trim_end_matches(['K', 'k']).parse::<f32>().ok()?;
    (start != end && kelvin.is_finite()).then_some(NightLight { start, end, kelvin: kelvin.clamp(1000.0, DAYLIGHT) })
}

/// Reads `glow`, `scanlines` and `night_light` out of the configuration file's text;
/// the last of each wins, and what cannot be read is left at its default.
pub fn parse(text: &str) -> Settings {
    let mut settings = Settings::default();
    for line in text.lines() {
        let Some((key, value)) = line.split('#').next().unwrap_or("").split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "glow" => {
                if let Ok(glow) = value.parse::<f32>().map(|glow| if glow > 1.0 { glow / 100.0 } else { glow }) {
                    if glow.is_finite() {
                        settings.glow = glow.clamp(0.0, 1.0);
                    }
                }
            }
            "scanlines" => settings.scanlines = value == "on",
            "night_light" => settings.night = parse_night(value),
            _ => {}
        }
    }
    settings
}

/// What is saved, or the defaults when nothing is.
pub fn load() -> Settings {
    des_common::input::config_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map_or_else(Settings::default, |text| parse(&text))
}

/// The `accent = r, g, b` override the shell writes; the last such line counts.
pub fn custom_accent(text: &str) -> Option<[u8; 3]> {
    text.lines()
        .filter_map(|line| line.split('#').next()?.split_once('='))
        .filter(|(key, _)| key.trim() == "accent")
        .last()
        .and_then(|(_, value)| {
            let channels: Vec<u8> = value.split(',').map(|part| part.trim().parse().ok()).collect::<Option<_>>()?;
            channels.try_into().ok()
        })
}

/// Whether `now` (minutes after midnight) falls in the window from `start` to `end`.
pub fn in_window(now: u32, start: u32, end: u32) -> bool {
    if start < end { (start..end).contains(&now) } else { now >= start || now < end }
}

/// The colour of light at a temperature, by Tanner Helland's fit, unscaled.
fn helland(kelvin: f32) -> [f32; 3] {
    let t = kelvin.clamp(1000.0, 40000.0) / 100.0;
    let red = if t <= 66.0 { 255.0 } else { 329.698_73 * (t - 60.0).powf(-0.133_204_76) };
    let green = if t <= 66.0 { 99.470_8 * t.ln() - 161.119_57 } else { 288.122_16 * (t - 60.0).powf(-0.075_514_85) };
    let blue = if t >= 66.0 { 255.0 } else if t <= 19.0 { 0.0 } else { 138.517_73 * (t - 10.0).ln() - 305.044_8 };
    [red, green, blue].map(|channel| channel.clamp(0.0, 255.0))
}

/// What to multiply each channel by to show colours at `kelvin`: one each at daylight,
/// with blue and then green falling away as it warms.
pub fn kelvin_rgb(kelvin: f32) -> [f32; 3] {
    let day = helland(DAYLIGHT);
    let at = helland(kelvin);
    [0, 1, 2].map(|i| (at[i] / day[i]).clamp(0.0, 1.0))
}

/// One step of the night light coming on or going off: `current` moves toward
/// `target`, covering `span` kelvin in [`RAMP_SECONDS`].
pub fn ramp(current: f32, target: f32, span: f32, seconds: f32) -> f32 {
    let most = (span.abs().max(1.0) * seconds / RAMP_SECONDS).max(0.0);
    current + (target - current).clamp(-most, most)
}

/// Whether the pass would change nothing, and so need not run.
pub fn is_identity(glow: f32, scanlines: bool, kelvin: f32) -> bool {
    glow <= 0.001 && !scanlines && (kelvin - DAYLIGHT).abs() < 1.0
}

/// What a display's frame is to be, this time round.
pub enum Pass {
    /// Draw the scene as it is: the pass has nothing to do.
    Direct,
    /// Nothing has changed since the last frame: there is nothing to draw.
    Unchanged,
    /// Draw this one element, the processed frame, in place of the scene.
    Draw(TextureShaderElement),
}

/// One display's off-screen frame.
struct Target {
    texture: GlesTexture,
    tracker: OutputDamageTracker,
    size: Size<i32, Physical>,
    scale: f64,
    /// Whether the texture holds a frame yet, so that damage can be relied on.
    filled: bool,
    /// The uniforms it was last shown with.
    shown: Option<[f32; 8]>,
}

pub struct Phosphor {
    settings: Settings,
    accent: [f32; 3],
    /// The night light's temperature now, which ramps toward what the clock wants.
    kelvin: f32,
    last: Option<Instant>,
    program: Option<GlesTexProgram>,
    /// Set once the shader failed to compile: the pass then stays off.
    broken: bool,
    targets: HashMap<String, Target>,
}

impl Phosphor {
    pub fn new() -> Phosphor {
        let mut phosphor = Phosphor {
            settings: Settings::default(),
            accent: [0.31, 0.89, 0.85],
            kelvin: DAYLIGHT,
            last: None,
            program: None,
            broken: false,
            targets: HashMap::new(),
        };
        phosphor.reload();
        // At start the night light is simply where the clock says, with no ramp.
        phosphor.kelvin = phosphor.wanted_kelvin();
        phosphor
    }

    /// Reads the settings again; the night light ramps to any new temperature.
    pub fn reload(&mut self) {
        self.settings = load();
        // The user's own accent, when there is one, before the theme's.
        let text = des_common::input::config_path().and_then(|path| std::fs::read_to_string(path).ok()).unwrap_or_default();
        let accent = custom_accent(&text).unwrap_or_else(crate::splash::chosen_accent);
        self.accent = accent.map(|channel| channel as f32 / 255.0);
    }

    fn wanted_kelvin(&self) -> f32 {
        let Some(night) = self.settings.night else { return DAYLIGHT };
        use chrono::Timelike;
        let now = chrono::Local::now();
        if in_window(now.hour() * 60 + now.minute(), night.start, night.end) { night.kelvin } else { DAYLIGHT }
    }

    /// Moves the night light along; called once per frame, before any display is drawn.
    pub fn begin_frame(&mut self) {
        let now = Instant::now();
        let seconds = self.last.map_or(0.0, |last| now.duration_since(last).as_secs_f32());
        self.last = Some(now);
        let span = DAYLIGHT - self.settings.night.map_or(DAYLIGHT, |night| night.kelvin);
        self.kelvin = ramp(self.kelvin, self.wanted_kelvin(), span, seconds);
    }

    /// Forgets displays that are gone, so their textures are let go.
    pub fn retain(&mut self, names: &[String]) {
        self.targets.retain(|name, _| names.contains(name));
    }

    /// Draws `elements` off screen for the display `name` and hands back the processed
    /// frame to show instead. `size` is the frame's size in pixels, the way the display
    /// stands; `logical` its size in the desktop's coordinates. With `always`, a frame
    /// is drawn even when nothing changed, for a backend that has to present one.
    pub fn process<E: RenderElement<GlesRenderer>>(
        &mut self,
        renderer: &mut GlesRenderer,
        name: &str,
        size: Size<i32, Physical>,
        logical: Size<i32, Logical>,
        scale: f64,
        elements: &[E],
        always: bool,
    ) -> Pass {
        let temperature = kelvin_rgb(self.kelvin);
        if self.broken || is_identity(self.settings.glow, self.settings.scanlines, self.kelvin) || size.w <= 0 || size.h <= 0 {
            self.targets.remove(name);
            return Pass::Direct;
        }
        if self.program.is_none() {
            let uniforms = [
                UniformName::new("glow", UniformType::_1f),
                UniformName::new("scanlines", UniformType::_1f),
                UniformName::new("temperature", UniformType::_3f),
                UniformName::new("accent", UniformType::_3f),
                UniformName::new("texel", UniformType::_2f),
                UniformName::new("line_px", UniformType::_1f),
            ];
            match renderer.compile_custom_texture_shader(SHADER, &uniforms) {
                Ok(program) => self.program = Some(program),
                Err(e) => {
                    tracing::error!("cannot compile the phosphor shader, leaving frames as they are: {e:?}");
                    self.broken = true;
                    return Pass::Direct;
                }
            }
        }

        let stale = self.targets.get(name).is_none_or(|target| target.size != size || target.scale != scale);
        if stale {
            let texture = match renderer.create_buffer(Fourcc::Abgr8888, (size.w, size.h).into()) {
                Ok(texture) => texture,
                Err(e) => {
                    tracing::warn!("no room for the phosphor frame: {e}");
                    return Pass::Direct;
                }
            };
            let tracker = OutputDamageTracker::new(size, scale, Transform::Normal);
            self.targets.insert(name.to_string(), Target { texture, tracker, size, scale, filled: false, shown: None });
        }
        let Some(target) = self.targets.get_mut(name) else { return Pass::Direct };

        let changed = {
            let mut framebuffer = match renderer.bind(&mut target.texture) {
                Ok(framebuffer) => framebuffer,
                Err(e) => {
                    tracing::warn!("cannot draw the phosphor frame: {e}");
                    return Pass::Direct;
                }
            };
            // Once the texture holds a frame it is one frame old: only damage is drawn.
            let age = if target.filled { 1 } else { 0 };
            match target.tracker.render_output(renderer, &mut framebuffer, age, elements, [0.0, 0.0, 0.0, 1.0]) {
                Ok(result) => result.damage.is_some_and(|damage| !damage.is_empty()),
                Err(e) => {
                    tracing::warn!("cannot draw the phosphor frame: {e:?}");
                    target.filled = false;
                    return Pass::Direct;
                }
            }
        };
        let fresh = !target.filled;
        target.filled = true;

        let uniforms_now = [
            self.settings.glow,
            if self.settings.scanlines { 1.0 } else { 0.0 },
            temperature[0],
            temperature[1],
            temperature[2],
            self.accent[0],
            self.accent[1],
            self.accent[2],
        ];
        let restyled = target.shown != Some(uniforms_now);
        target.shown = Some(uniforms_now);
        if !(changed || fresh || restyled || always) {
            return Pass::Unchanged;
        }

        let Some(program) = self.program.clone() else { return Pass::Direct };
        let inner = TextureRenderElement::from_static_texture(
            Id::new(),
            renderer.context_id(),
            (0.0, 0.0),
            target.texture.clone(),
            1,
            Transform::Normal,
            None,
            None,
            Some(logical),
            None,
            Kind::Unspecified,
        );
        let uniforms = vec![
            Uniform::new("glow", UniformValue::_1f(self.settings.glow)),
            Uniform::new("scanlines", UniformValue::_1f(uniforms_now[1])),
            Uniform::new("temperature", UniformValue::_3f(temperature[0], temperature[1], temperature[2])),
            Uniform::new("accent", UniformValue::_3f(self.accent[0], self.accent[1], self.accent[2])),
            Uniform::new("texel", UniformValue::_2f(1.0 / size.w as f32, 1.0 / size.h as f32)),
            Uniform::new("line_px", UniformValue::_1f(scale.max(1.0) as f32)),
        ];
        Pass::Draw(TextureShaderElement::new(inner, program, uniforms))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_user_s_accent_wins_and_bad_values_are_ignored() {
        assert_eq!(custom_accent("theme = blade\naccent = 10, 20, 30\n"), Some([10, 20, 30]));
        assert_eq!(custom_accent("accent = 1, 2, 3\naccent = 4, 5, 6"), Some([4, 5, 6]));
        assert_eq!(custom_accent("accent = 300, 2, 3"), None);
        assert_eq!(custom_accent("accent = 1, 2"), None);
        assert_eq!(custom_accent("# accent = 1, 2, 3\ntheme = signal"), None);
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.02
    }

    #[test]
    fn daylight_is_left_alone_and_night_warms() {
        assert_eq!(kelvin_rgb(DAYLIGHT), [1.0, 1.0, 1.0]);
        let evening = kelvin_rgb(4500.0);
        assert!(close(evening[0], 1.0));
        assert!(evening[1] < 1.0 && evening[2] < evening[1], "{evening:?}");
        let candle = kelvin_rgb(1900.0);
        assert!(close(candle[0], 1.0));
        assert!(candle[2] < 0.05, "{candle:?}");
        // Known points of the fit, against daylight.
        assert!(close(kelvin_rgb(3000.0)[1], 0.70), "{:?}", kelvin_rgb(3000.0));
        assert!(close(kelvin_rgb(3000.0)[2], 0.44), "{:?}", kelvin_rgb(3000.0));
    }

    #[test]
    fn the_night_window_wraps_past_midnight() {
        let (seven_pm, seven_am) = (19 * 60, 7 * 60);
        assert!(in_window(23 * 60, seven_pm, seven_am));
        assert!(in_window(0, seven_pm, seven_am));
        assert!(in_window(6 * 60 + 59, seven_pm, seven_am));
        assert!(!in_window(seven_am, seven_pm, seven_am));
        assert!(!in_window(12 * 60, seven_pm, seven_am));
        assert!(in_window(13 * 60, 12 * 60, 14 * 60));
        assert!(!in_window(14 * 60, 12 * 60, 14 * 60));
    }

    #[test]
    fn reads_the_settings() {
        assert_eq!(parse(""), Settings::default());
        let settings = parse("glow = 0.4\nscanlines = on\nnight_light = 19:00-07:00 4500 # evenings\n");
        assert_eq!(settings.glow, 0.4);
        assert!(settings.scanlines);
        assert_eq!(settings.night, Some(NightLight { start: 19 * 60, end: 7 * 60, kelvin: 4500.0 }));
        assert_eq!(parse("glow = 40").glow, 0.4);
        assert_eq!(parse("glow = 7").glow, 0.07);
        assert_eq!(parse("glow = -3").glow, 0.0);
        assert_eq!(parse("glow = nan").glow, 0.0);
        assert_eq!(parse("night_light = 22:00-06:30 900K").night.map(|night| night.kelvin), Some(1000.0));
        for bad in ["25:00-07:00 4500", "19:00-19:00 4500", "19:00 4500", "19:00-07:00 warm", "off"] {
            assert_eq!(parse(&format!("night_light = {bad}")).night, None, "{bad}");
        }
    }

    #[test]
    fn the_pass_is_skipped_when_it_would_change_nothing() {
        assert!(is_identity(0.0, false, DAYLIGHT));
        assert!(!is_identity(0.2, false, DAYLIGHT));
        assert!(!is_identity(0.0, true, DAYLIGHT));
        assert!(!is_identity(0.0, false, 4500.0));
    }

    #[test]
    fn the_night_light_ramps_over_a_minute() {
        let span = DAYLIGHT - 4500.0;
        let mut kelvin = DAYLIGHT;
        for _ in 0..30 {
            kelvin = ramp(kelvin, 4500.0, span, 1.0);
        }
        assert!(close(kelvin, 5500.0), "halfway after thirty seconds: {kelvin}");
        for _ in 0..40 {
            kelvin = ramp(kelvin, 4500.0, span, 1.0);
        }
        assert_eq!(kelvin, 4500.0);
        assert_eq!(ramp(4500.0, DAYLIGHT, span, 600.0), DAYLIGHT);
        assert_eq!(ramp(5000.0, 5000.0, span, 1.0), 5000.0);
    }

    #[test]
    fn the_shader_declares_what_it_is_given() {
        for uniform in ["uniform float glow;", "uniform float scanlines;", "uniform vec3 temperature;", "uniform vec3 accent;", "uniform vec2 texel;", "uniform float line_px;"] {
            assert!(SHADER.contains(uniform), "{uniform}");
        }
        // What Smithay's texture program fills in or binds itself.
        for needed in ["//_DEFINES_", "uniform sampler2D tex;", "uniform float alpha;", "varying vec2 v_coords;", "uniform float tint;"] {
            assert!(SHADER.contains(needed), "{needed}");
        }
        assert_eq!(SHADER.matches('{').count(), SHADER.matches('}').count());
    }
}
