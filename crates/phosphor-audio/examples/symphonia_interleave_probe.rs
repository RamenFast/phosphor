//! Throwaway probe: verify Symphonia's copy_interleaved_ref channel order for 5.1.
//! Feed it a 5.1 file where each channel carries a distinct solo sine
//! (FL=300, FR=600, FC=900, LFE=60, RL=1200, RR=1500 Hz) and estimate the
//! dominant frequency per interleave slot via zero-crossing counting.
use std::env;
use std::fs::File;
use symphonia::core::audio::{AudioBufferRef, Signal};
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

fn main() {
    let path = env::args().nth(1).expect("usage: probe <file>");
    let file = File::open(&path).unwrap();
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let probed = symphonia::default::get_probe()
        .format(&Hint::new(), mss, &FormatOptions::default(), &MetadataOptions::default())
        .unwrap();
    let mut format = probed.format;
    let track = format.default_track().unwrap();
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .unwrap();

    let mut chans: Vec<Vec<f32>> = Vec::new();
    let mut rate = 0u32;
    let mut spec_printed = false;
    while let Ok(packet) = format.next_packet() {
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(_) => continue,
        };
        let spec = *decoded.spec();
        if !spec_printed {
            println!("spec.channels = {:?} (bits {:#x}), rate = {}", spec.channels, spec.channels.bits(), spec.rate);
            spec_printed = true;
            rate = spec.rate;
            chans = vec![Vec::new(); spec.channels.count()];
        }
        // Use copy_interleaved_ref path: convert to f32 interleaved.
        let mut sbuf = symphonia::core::audio::SampleBuffer::<f32>::new(
            decoded.capacity() as u64,
            spec,
        );
        sbuf.copy_interleaved_ref(decoded.clone());
        let n = spec.channels.count();
        for (i, s) in sbuf.samples().iter().enumerate() {
            chans[i % n].push(*s);
        }
        // Also cross-check planar access order via chan()
        if let AudioBufferRef::F32(buf) = &decoded {
            let _ = buf.chan(0);
        }
    }

    for (i, c) in chans.iter().enumerate() {
        // zero-crossing frequency estimate + rms
        let mut zc = 0usize;
        for w in c.windows(2) {
            if (w[0] >= 0.0) != (w[1] >= 0.0) {
                zc += 1;
            }
        }
        let secs = c.len() as f32 / rate as f32;
        let freq = zc as f32 / 2.0 / secs;
        let rms = (c.iter().map(|s| s * s).sum::<f32>() / c.len() as f32).sqrt();
        println!("slot {}: ~{:.0} Hz, rms {:.3}", i, freq, rms);
    }
}
