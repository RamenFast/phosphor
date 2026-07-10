// SPDX-License-Identifier: GPL-3.0-or-later
//! Runtime receipt for va-gap-runtime-thread-probe: which OS thread does
//! each PipeWire `.process` closure actually run on?
//!
//! Replicates the engine's two stream setups exactly:
//!   - capture: Input, AUTOCONNECT|MAP_BUFFERS (NO RT_PROCESS) — engine.rs
//!     build_capture_stream (~1371-1376).
//!   - playback: Output, AUTOCONNECT|MAP_BUFFERS|RT_PROCESS — engine.rs
//!     build_playback_stream (~1496-1507).
//!
//! Logs gettid + /proc/self/task/<tid>/comm + scheduler policy/priority once
//! from inside each process closure, plus the same for the mainloop thread.
//!
//! Usage: cargo run -p phosphor-audio --example thread_probe

use pipewire as pw;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

fn gettid() -> i64 {
    unsafe { libc_gettid() }
}
unsafe fn libc_gettid() -> i64 {
    // SYS_gettid = 186 on x86_64
    unsafe { syscall_gettid() }
}
unsafe fn syscall_gettid() -> i64 {
    unsafe extern "C" {
        fn syscall(num: i64, ...) -> i64;
    }
    unsafe { syscall(186) }
}

fn thread_report(tag: &str) {
    let tid = gettid();
    let comm = std::fs::read_to_string(format!("/proc/self/task/{tid}/comm"))
        .unwrap_or_default()
        .trim()
        .to_string();
    // scheduler policy + priority via /proc stat fields 41 (rt_priority) and 42 (policy)
    let stat = std::fs::read_to_string(format!("/proc/self/task/{tid}/stat")).unwrap_or_default();
    let after = stat.rsplit(')').next().unwrap_or("");
    let fields: Vec<&str> = after.split_whitespace().collect();
    // after ')' field index: state=0 ... rt_priority=37, policy=38
    let rt_prio = fields.get(37).copied().unwrap_or("?");
    let policy = fields.get(38).copied().unwrap_or("?");
    let policy_name = match policy {
        "0" => "SCHED_OTHER",
        "1" => "SCHED_FIFO",
        "2" => "SCHED_RR",
        _ => policy,
    };
    println!("[{tag}] tid={tid} comm={comm:?} policy={policy_name} rt_priority={rt_prio}");
}

fn main() {
    pw::init();

    // Name this thread the same as the engine does for its pw loop thread,
    // so comm comparison is meaningful. The engine spawns "phosphor-audio-pw"
    // (engine.rs:132-133); here main IS the loop thread, so rename it.
    let name = std::ffi::CString::new("phosphor-audio-pw").unwrap();
    unsafe {
        unsafe extern "C" {
            fn pthread_self() -> u64;
            fn pthread_setname_np(t: u64, n: *const std::os::raw::c_char) -> i32;
        }
        pthread_setname_np(pthread_self(), name.as_ptr());
    }

    let mainloop = pw::main_loop::MainLoopRc::new(None).expect("main loop");
    let context = pw::context::ContextRc::new(&mainloop, None).expect("context");
    let core = context.connect_rc(None).expect("connect");

    thread_report("mainloop-thread(before run)");

    // ---- capture stream (engine flags: no RT_PROCESS) ----------------------
    use pw::properties::properties;
    let cap_props = properties! {
        *pw::keys::MEDIA_TYPE => "Audio",
        *pw::keys::MEDIA_CATEGORY => "Capture",
        *pw::keys::MEDIA_ROLE => "Music",
        *pw::keys::NODE_NAME => "thread-probe-capture",
        *pw::keys::STREAM_CAPTURE_SINK => "true",
    };
    let cap_stream =
        pw::stream::StreamRc::new(core.clone(), "thread-probe-capture", cap_props).expect("cap");
    static CAP_LOGGED: AtomicBool = AtomicBool::new(false);
    let _cap_listener = cap_stream
        .add_local_listener::<()>()
        .process(move |stream, _| {
            if !CAP_LOGGED.swap(true, Ordering::Relaxed) {
                thread_report("capture .process");
            }
            while let Some(_b) = stream.dequeue_buffer() {}
        })
        .register()
        .expect("cap listener");

    let mut audio_info = pw::spa::param::audio::AudioInfoRaw::new();
    audio_info.set_format(pw::spa::param::audio::AudioFormat::F32LE);
    audio_info.set_rate(48_000);
    audio_info.set_channels(2);
    let object = pw::spa::pod::Object {
        type_: pw::spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
        id: pw::spa::param::ParamType::EnumFormat.as_raw(),
        properties: audio_info.into(),
    };
    let values: Vec<u8> = pw::spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &pw::spa::pod::Value::Object(object),
    )
    .unwrap()
    .0
    .into_inner();
    let mut cap_params = [pw::spa::pod::Pod::from_bytes(&values).unwrap()];
    cap_stream
        .connect(
            pw::spa::utils::Direction::Input,
            None,
            pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
            &mut cap_params,
        )
        .expect("cap connect");

    // ---- playback stream (engine flags: RT_PROCESS) -------------------------
    let pb_props = properties! {
        *pw::keys::MEDIA_TYPE => "Audio",
        *pw::keys::MEDIA_CATEGORY => "Playback",
        *pw::keys::MEDIA_ROLE => "Music",
        *pw::keys::NODE_NAME => "thread-probe-playback",
    };
    let pb_stream =
        pw::stream::StreamRc::new(core.clone(), "thread-probe-playback", pb_props).expect("pb");
    static PB_LOGGED: AtomicBool = AtomicBool::new(false);
    let _pb_listener = pb_stream
        .add_local_listener::<()>()
        .process(move |stream, _| {
            if !PB_LOGGED.swap(true, Ordering::Relaxed) {
                thread_report("playback .process (RT_PROCESS)");
            }
            // emit silence
            if let Some(mut buffer) = stream.dequeue_buffer() {
                let datas = buffer.datas_mut();
                if let Some(data) = datas.first_mut()
                    && let Some(slice) = data.data()
                {
                    for b in slice.iter_mut().take(4096) {
                        *b = 0;
                    }
                    let chunk = data.chunk_mut();
                    *chunk.offset_mut() = 0;
                    *chunk.stride_mut() = 8;
                    *chunk.size_mut() = 4096;
                }
            }
        })
        .register()
        .expect("pb listener");

    let mut pb_info = pw::spa::param::audio::AudioInfoRaw::new();
    pb_info.set_format(pw::spa::param::audio::AudioFormat::F32LE);
    pb_info.set_rate(48_000);
    pb_info.set_channels(2);
    let mut position = [0u32; 64];
    position[0] = pw::spa::sys::SPA_AUDIO_CHANNEL_FL;
    position[1] = pw::spa::sys::SPA_AUDIO_CHANNEL_FR;
    pb_info.set_position(position);
    let pb_object = pw::spa::pod::Object {
        type_: pw::spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
        id: pw::spa::param::ParamType::EnumFormat.as_raw(),
        properties: pb_info.into(),
    };
    let pb_values: Vec<u8> = pw::spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &pw::spa::pod::Value::Object(pb_object),
    )
    .unwrap()
    .0
    .into_inner();
    let mut pb_params = [pw::spa::pod::Pod::from_bytes(&pb_values).unwrap()];
    pb_stream
        .connect(
            pw::spa::utils::Direction::Output,
            None,
            pw::stream::StreamFlags::AUTOCONNECT
                | pw::stream::StreamFlags::MAP_BUFFERS
                | pw::stream::StreamFlags::RT_PROCESS,
            &mut pb_params,
        )
        .expect("pb connect");

    // run the loop for ~2s using a timer to quit
    let loop_ = mainloop.loop_();
    let mainloop_quit = mainloop.clone();
    let timer = loop_.add_timer(move |_| {
        thread_report("mainloop-thread(timer cb)");
        mainloop_quit.quit();
    });
    timer
        .update_timer(
            Some(Duration::from_secs(2)),
            None,
        )
        .into_result()
        .expect("timer");

    let started = Instant::now();
    mainloop.run();
    println!("ran {:?}", started.elapsed());

    // list all threads in the process for context
    println!("-- all task comms --");
    if let Ok(entries) = std::fs::read_dir("/proc/self/task") {
        for e in entries.flatten() {
            let tid = e.file_name().to_string_lossy().to_string();
            let comm = std::fs::read_to_string(e.path().join("comm")).unwrap_or_default();
            let stat = std::fs::read_to_string(e.path().join("stat")).unwrap_or_default();
            let after = stat.rsplit(')').next().unwrap_or("");
            let f: Vec<&str> = after.split_whitespace().collect();
            println!(
                "  tid={tid} comm={} policy={} rt_prio={}",
                comm.trim(),
                f.get(38).copied().unwrap_or("?"),
                f.get(37).copied().unwrap_or("?")
            );
        }
    }
}
