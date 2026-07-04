// SPDX-License-Identifier: GPL-3.0-or-later
//! The gate: replay every golden fixture through the one engine.
//!
//! tests/golden/ carries v3 ground truth (see its README): Python-
//! reference cases compare within the documented contract (coordinates
//! 0.05 px, intensity 5e-3, segment counts exact, call by call);
//! native-v3 cases were captured from the code this crate verbatim-
//! ports, so they are held to the same contract but reported at their
//! actual (much tighter) worst deltas; kit-audio captures are f32 math
//! with f64 accumulators and must be essentially exact.

use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use serde::Deserialize;

use phosphor_dsp::{Computer, KitOp, KitStage, Mode};

const COORDINATE_TOLERANCE: f32 = 0.05;
const INTENSITY_TOLERANCE: f32 = 5e-3;
const KIT_AUDIO_TOLERANCE: f32 = 1e-6;

fn golden_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden")
}

fn read_f32le(path: &Path) -> Vec<f32> {
    let bytes = fs::read(path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
        .collect()
}

#[derive(Deserialize)]
struct CameraSpec {
    yaw: f64,
    pitch: f64,
    dolly: f64,
}

#[derive(Deserialize)]
struct KitSpec {
    stages: Vec<(String, Vec<f64>)>,
}

#[derive(Deserialize)]
struct InputRef {
    file: String,
    floats: usize,
}

#[derive(Deserialize)]
struct Case {
    name: String,
    mode: String,
    sample_rate: u32,
    #[serde(default = "one")]
    oversample: u32,
    gain: f32,
    beam_energy: f32,
    frame_glow_keep: f32,
    width: f32,
    height: f32,
    camera: Option<CameraSpec>,
    kit: Option<KitSpec>,
    chunks: Vec<usize>,
    input: InputRef,
    recorded_calls: Vec<usize>,
    segment_counts_recorded: Vec<usize>,
}

fn one() -> u32 {
    1
}

#[derive(Deserialize)]
struct OutputRef {
    file: String,
    floats: usize,
}

#[derive(Deserialize)]
struct KitAudioCase {
    name: String,
    sample_rate: u32,
    chunks: Vec<usize>,
    input: InputRef,
    kit: KitSpec,
    output: OutputRef,
}

fn kit_stages(spec: &KitSpec) -> Vec<(KitOp, [f64; 4])> {
    spec.stages
        .iter()
        .map(|(name, params)| {
            let op = KitOp::from_name(name)
                .unwrap_or_else(|| panic!("unknown kit op '{name}'"));
            let mut packed = [0.0f64; 4];
            for (slot, value) in packed.iter_mut().zip(params) {
                *slot = *value;
            }
            (op, packed)
        })
        .collect()
}

#[derive(Default)]
struct Worst {
    coordinate: f32,
    intensity: f32,
    site: String,
}

impl Worst {
    fn absorb_coordinate(&mut self, delta: f32, site: &str) {
        if delta > self.coordinate {
            self.coordinate = delta;
            self.site = site.to_string();
        }
    }
}

fn run_case(directory: &Path, case: &Case, worst: &mut Worst) {
    let input_path = golden_root().join("inputs").join(&case.input.file);
    let input = read_f32le(&input_path);
    assert!(input.len() >= case.input.floats,
            "{}: input shorter than declared", case.name);
    let input = &input[..case.input.floats];

    let expected_rows = read_f32le(
        &directory.join(format!("{}.segments.bin", case.name)));
    assert_eq!(expected_rows.len() % 5, 0, "{}: ragged bin", case.name);

    let mut computer = Computer::new();
    computer.mode = Mode::from_str(&case.mode)
        .unwrap_or_else(|error| panic!("{}: {error}", case.name));
    computer.gain = case.gain;
    computer.beam_energy = case.beam_energy;
    computer.frame_glow_keep = case.frame_glow_keep;
    computer.set_sample_rate(case.sample_rate, case.oversample);
    if let Some(camera) = &case.camera {
        computer.set_camera(Some(camera.yaw), Some(camera.pitch),
                            Some(camera.dolly));
    }
    if let Some(kit) = &case.kit {
        computer.set_kit(&kit_stages(kit));
    }

    let mut cursor = 0usize;
    let mut recorded_index = 0usize;
    let mut expected_offset = 0usize;
    for (call_index, &chunk) in case.chunks.iter().enumerate() {
        let piece = &input[cursor..cursor + chunk];
        cursor += chunk;
        let segments = computer.compute(piece, case.width, case.height);
        let is_recorded = case.recorded_calls.contains(&call_index);
        if !is_recorded {
            continue;
        }
        let expected_count = case.segment_counts_recorded[recorded_index];
        assert_eq!(segments.len(), expected_count,
                   "{} call {call_index}: {} segments vs {} recorded",
                   case.name, segments.len(), expected_count);
        for (row_index, segment) in segments.iter().enumerate() {
            let expected =
                &expected_rows[(expected_offset + row_index) * 5
                    ..(expected_offset + row_index) * 5 + 5];
            for column in 0..4 {
                let delta = (segment[column] - expected[column]).abs();
                worst.absorb_coordinate(
                    delta,
                    &format!("{} call {call_index} row {row_index} col {column}",
                             case.name));
                assert!(delta <= COORDINATE_TOLERANCE,
                        "{} call {call_index} row {row_index} column {column}: \
                         {} vs {} (Δ {delta})",
                        case.name, segment[column], expected[column]);
            }
            let delta = (segment[4] - expected[4]).abs();
            worst.intensity = worst.intensity.max(delta);
            assert!(delta <= INTENSITY_TOLERANCE,
                    "{} call {call_index} row {row_index} intensity: \
                     {} vs {} (Δ {delta})",
                    case.name, segment[4], expected[4]);
        }
        expected_offset += expected_count;
        recorded_index += 1;
    }
    assert_eq!(expected_offset * 5, expected_rows.len(),
               "{}: bin rows left over", case.name);
}

fn run_case_directory(directory: &Path) -> (usize, Worst) {
    let mut names: Vec<PathBuf> = fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("{}: {error}", directory.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    names.sort();
    let mut worst = Worst::default();
    let mut count = 0usize;
    for path in names {
        let case: Case =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap_or_else(
                |error| panic!("{}: {error}", path.display()));
        run_case(directory, &case, &mut worst);
        count += 1;
    }
    (count, worst)
}

#[test]
fn python_reference_cases() {
    let (count, worst) = run_case_directory(&golden_root().join("cases"));
    assert!(count >= 91, "expected the full python-reference set, got {count}");
    println!("python-reference: {count} cases, worst coordinate Δ {:.6} px \
              ({}), worst intensity Δ {:.6}",
             worst.coordinate, worst.site, worst.intensity);
}

#[test]
fn native_v3_cases() {
    let directory = golden_root().join("native-v3").join("cases");
    let (count, worst) = run_case_directory(&directory);
    assert!(count >= 26, "expected the full native-v3 set, got {count}");
    println!("native-v3: {count} cases (incl. os2/os4 sinc truth), worst \
              coordinate Δ {:.6} px ({}), worst intensity Δ {:.6}",
             worst.coordinate, worst.site, worst.intensity);
}

#[test]
fn kit_audio_captures() {
    let directory = golden_root().join("kits");
    let mut paths: Vec<PathBuf> = fs::read_dir(&directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    paths.sort();
    let mut worst = 0.0f32;
    let mut count = 0usize;
    for path in paths {
        let case: KitAudioCase =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let input =
            read_f32le(&golden_root().join("inputs").join(&case.input.file));
        let input = &input[..case.input.floats];
        let expected =
            read_f32le(&directory.join(&case.output.file));
        assert_eq!(expected.len(), case.output.floats, "{}: bad out bin",
                   case.name);

        let mut stages: Vec<KitStage> = kit_stages(&case.kit)
            .into_iter()
            .map(|(op, params)| {
                let mut stage = KitStage::new(op, params);
                stage.reset(case.sample_rate as f64);
                stage
            })
            .collect();
        let mut produced: Vec<f32> = Vec::with_capacity(expected.len());
        let mut cursor = 0usize;
        for &chunk in &case.chunks {
            let mut buffer = input[cursor..cursor + chunk].to_vec();
            cursor += chunk;
            for stage in stages.iter_mut() {
                stage.process(&mut buffer, case.sample_rate as f64);
            }
            produced.extend_from_slice(&buffer);
        }
        assert_eq!(produced.len(), expected.len(), "{}: length", case.name);
        for (index, (mine, theirs)) in
            produced.iter().zip(&expected).enumerate()
        {
            let delta = (mine - theirs).abs();
            worst = worst.max(delta);
            assert!(delta <= KIT_AUDIO_TOLERANCE,
                    "{} sample {index}: {mine} vs {theirs} (Δ {delta})",
                    case.name);
        }
        count += 1;
    }
    assert!(count >= 12, "expected 12 kit-audio captures, got {count}");
    println!("kit audio: {count} captures, worst Δ {worst:.2e}");
}
