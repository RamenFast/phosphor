// SPDX-License-Identifier: GPL-3.0-or-later
//! A0 smoke: prove pipewire-rs builds against libpipewire 1.0.5 and can
//! enumerate the live graph. Prints every node `list_capture_targets`
//! will care about (playing apps, sink monitors, mics) then exits after
//! one registry round-trip (core sync → done).

use pipewire as pw;
use pw::types::ObjectType;
use std::cell::Cell;
use std::rc::Rc;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&mainloop, None)?;
    let core = context.connect_rc(None)?;
    let registry = core.get_registry()?;

    // One round-trip: everything already in the registry is delivered
    // before the `done` for this sync arrives, so quitting on `done`
    // means the enumeration is complete, not racy.
    let done = Rc::new(Cell::new(false));
    let done_clone = done.clone();
    let loop_clone = mainloop.clone();
    let pending = core.sync(0)?;

    let _core_listener = core
        .add_listener_local()
        .done(move |id, seq| {
            if id == pw::core::PW_ID_CORE && seq == pending {
                done_clone.set(true);
                loop_clone.quit();
            }
        })
        .register();

    let _registry_listener = registry
        .add_listener_local()
        .global(|global| {
            if global.type_ != ObjectType::Node {
                return;
            }
            let Some(props) = global.props else { return };
            let class = props.get("media.class").unwrap_or("");
            if !matches!(class, "Stream/Output/Audio" | "Audio/Sink" | "Audio/Source") {
                return;
            }
            println!(
                "id={:<4} serial={:<4} {:<20} node.name={:<44} app={:?} media={:?} desc={:?}",
                global.id,
                props.get("object.serial").unwrap_or("?"),
                class,
                props.get("node.name").unwrap_or(""),
                props.get("application.name"),
                props.get("media.name"),
                props.get("node.description"),
            );
        })
        .register();

    while !done.get() {
        mainloop.run();
    }
    Ok(())
}
