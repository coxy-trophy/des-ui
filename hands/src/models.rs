//! The two models, run with tract. Both are MediaPipe's, as converted to ONNX for
//! OpenCV's model zoo, and are built into the program.

use tract_onnx::prelude::*;

use crate::{hand, palm};

type Plan = Arc<TypedRunnableModel>;

static PALM_MODEL: &[u8] = include_bytes!("../models/palm_detection_mediapipe_2023feb.onnx");
static HAND_MODEL: &[u8] = include_bytes!("../models/handpose_estimation_mediapipe_2023feb.onnx");

pub struct Models {
    palm: Plan,
    hand: Plan,
}

/// What a model gave back: each output's name, shape and values.
pub struct Answer(Vec<(String, Vec<usize>, Vec<f32>)>);

impl Answer {
    /// The first output with `count` values in all.
    fn sized(&self, count: usize) -> Option<&[f32]> {
        self.0.iter().find(|(_, _, values)| values.len() == count).map(|(_, _, values)| values.as_slice())
    }

    /// The outputs with `count` values in all, in order.
    fn all_sized(&self, count: usize) -> Vec<&[f32]> {
        self.0.iter().filter(|(_, _, values)| values.len() == count).map(|(_, _, values)| values.as_slice()).collect()
    }

    pub fn describe(&self) -> String {
        self.0.iter().map(|(name, shape, _)| format!("{name} {shape:?}")).collect::<Vec<_>>().join(", ")
    }
}

fn load(bytes: &[u8], size: usize) -> TractResult<Plan> {
    tract_onnx::onnx()
        .model_for_read(&mut &*bytes)?
        .with_input_fact(0, f32::fact([1, size, size, 3]).into())?
        .into_optimized()?
        .into_runnable()
}

fn run(plan: &Plan, size: usize, input: &[f32]) -> TractResult<Answer> {
    let tensor = Tensor::from_shape(&[1, size, size, 3], input)?;
    let outputs = plan.run(tvec!(tensor.into()))?;
    let names: Vec<String> = plan
        .model()
        .output_outlets()?
        .iter()
        .map(|outlet| plan.model().node(outlet.node).name.clone())
        .collect();
    let mut answer = Vec::new();
    for (index, output) in outputs.iter().enumerate() {
        let values = output.try_as_plain_ram()?.as_slice::<f32>()?.to_vec();
        answer.push((names.get(index).cloned().unwrap_or_default(), output.shape().to_vec(), values));
    }
    Ok(Answer(answer))
}

impl Models {
    pub fn load() -> TractResult<Models> {
        Ok(Models { palm: load(PALM_MODEL, palm::SIZE)?, hand: load(HAND_MODEL, hand::SIZE)? })
    }

    pub fn palm(&self, input: &[f32]) -> TractResult<Answer> {
        run(&self.palm, palm::SIZE, input)
    }

    pub fn hand(&self, input: &[f32]) -> TractResult<Answer> {
        run(&self.hand, hand::SIZE, input)
    }
}

/// The palm detector's boxes and scores, told apart by their sizes.
pub fn palm_outputs(answer: &Answer) -> Option<(&[f32], &[f32])> {
    Some((answer.sized(2016 * 18)?, answer.sized(2016)?))
}

/// The hand model's joints, how sure it is a hand, and which hand: the joints are the
/// first of the two outputs of 63 values, the other being in metres; presence comes
/// before handedness.
pub fn hand_outputs(answer: &Answer) -> Option<(&[f32], f32, f32)> {
    let joints = *answer.all_sized(63).first()?;
    let singles = answer.all_sized(1);
    Some((joints, *singles.first()?.first()?, *singles.get(1)?.first()?))
}
