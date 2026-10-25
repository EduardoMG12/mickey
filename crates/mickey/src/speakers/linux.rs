//! Linux (PulseAudio, or PipeWire speaking its protocol): a sink's monitor
//! follows its mute, so muting the speakers would also silence what Mickey
//! records. Instead Mickey adds a silent sink, `mickey`, and makes it the
//! default: what plays moves there at once, and Mickey records its monitor.
//! The sink notes which output was the default before
//! (`mickey.restore=<name>` in its properties), so putting it back needs no
//! memory of Mickey's own and works after a crash too (`clean_up`).

use libpulse_binding::callbacks::ListResult;
use libpulse_binding::context::{Context, FlagSet, State};
use libpulse_binding::mainloop::standard::{IterateResult, Mainloop};
use libpulse_binding::operation::{Operation, State as OpState};
use std::cell::RefCell;
use std::rc::Rc;

/// The silent sink's name; its monitor is `mickey.monitor`.
pub const SINK: &str = "mickey";
const RESTORE: &str = "mickey.restore=";

/// A connection to the sound server, made for a few requests.
struct Pulse {
    // Fields drop in this order: the context must go before the loop it
    // runs on (the other way round crashed).
    context: Context,
    mainloop: Mainloop,
}

impl Pulse {
    fn connect() -> Result<Pulse, String> {
        let mainloop = Mainloop::new().ok_or("cannot start a sound-server loop")?;
        let mut context =
            Context::new(&mainloop, "Mickey").ok_or("cannot reach the sound server")?;
        context
            .connect(None, FlagSet::NOAUTOSPAWN, None)
            .map_err(|err| format!("cannot reach the sound server: {err}"))?;
        let mut pulse = Pulse { context, mainloop };
        loop {
            match pulse.context.get_state() {
                State::Ready => return Ok(pulse),
                State::Failed | State::Terminated => {
                    return Err("the sound server refused the connection".into());
                }
                _ => pulse.turn()?,
            }
        }
    }

    fn turn(&mut self) -> Result<(), String> {
        match self.mainloop.iterate(true) {
            IterateResult::Success(_) => Ok(()),
            _ => Err("lost the sound server".into()),
        }
    }

    /// Runs the loop until `operation` is done.
    fn wait<F: ?Sized>(&mut self, operation: Operation<F>) -> Result<(), String> {
        while operation.get_state() == OpState::Running {
            self.turn()?;
        }
        Ok(())
    }

    fn default_sink(&mut self) -> Result<Option<String>, String> {
        let found = Rc::new(RefCell::new(None));
        let into = Rc::clone(&found);
        let operation = self.context.introspect().get_server_info(move |info| {
            *into.borrow_mut() = info.default_sink_name.as_ref().map(|name| name.to_string());
        });
        self.wait(operation)?;
        Ok(found.take())
    }

    fn set_default_sink(&mut self, name: &str) -> Result<bool, String> {
        let done = Rc::new(RefCell::new(false));
        let into = Rc::clone(&done);
        let operation = self
            .context
            .set_default_sink(name, move |ok| *into.borrow_mut() = ok);
        self.wait(operation)?;
        Ok(done.take())
    }

    /// Every module that made a `mickey` sink: its index, and the output
    /// it says to restore.
    fn ours(&mut self) -> Result<Vec<(u32, Option<String>)>, String> {
        let found = Rc::new(RefCell::new(Vec::new()));
        let into = Rc::clone(&found);
        let operation = self.context.introspect().get_module_info_list(move |item| {
            if let ListResult::Item(module) = item
                && module.name.as_deref() == Some("module-null-sink")
            {
                let argument = module.argument.as_deref().unwrap_or("");
                if argument
                    .split_whitespace()
                    .any(|word| word == format!("sink_name={SINK}"))
                {
                    into.borrow_mut().push((module.index, restore_of(argument)));
                }
            }
        });
        self.wait(operation)?;
        Ok(found.take())
    }

    fn load_sink(&mut self, restore: &str) -> Result<u32, String> {
        let index = Rc::new(RefCell::new(u32::MAX));
        let into = Rc::clone(&index);
        let argument = format!(
            "sink_name={SINK} sink_properties=\"device.description=Mickey {RESTORE}{restore}\""
        );
        let operation =
            self.context
                .introspect()
                .load_module("module-null-sink", &argument, move |loaded| {
                    *into.borrow_mut() = loaded
                });
        self.wait(operation)?;
        match index.take() {
            u32::MAX => Err("the sound server would not add Mickey's sink".into()),
            loaded => Ok(loaded),
        }
    }

    fn unload(&mut self, index: u32) -> Result<(), String> {
        let operation = self.context.introspect().unload_module(index, |_| {});
        self.wait(operation)
    }
}

/// The output to put back, from a `mickey` sink's module argument.
fn restore_of(argument: &str) -> Option<String> {
    let start = argument.find(RESTORE)? + RESTORE.len();
    let name: String = argument[start..]
        .chars()
        .take_while(|c| !c.is_whitespace() && *c != '"')
        .collect();
    (!name.is_empty()).then_some(name)
}

/// The sound taken off the speakers; given back when dropped.
pub struct Diverted;

/// Makes `mickey` the default output, after putting back any left over.
pub fn divert() -> Result<Diverted, String> {
    clean_up()?;
    let mut pulse = Pulse::connect()?;
    let speakers = pulse.default_sink()?.ok_or("this PC has no sound output")?;
    pulse.load_sink(&speakers)?;
    if !pulse.set_default_sink(SINK)? {
        clean_up()?;
        return Err("the sound server would not switch to Mickey's sink".into());
    }
    Ok(Diverted)
}

impl Drop for Diverted {
    fn drop(&mut self) {
        if let Err(err) = clean_up() {
            eprintln!("mickey: cannot give the sound back to the speakers: {err}");
        }
    }
}

/// Removes every `mickey` sink, putting back the output it noted as the
/// default (if it is still there; else the sound server picks one). Run
/// at start, when sending stops, and on a stop signal.
pub fn clean_up() -> Result<(), String> {
    let mut pulse = Pulse::connect()?;
    let ours = pulse.ours()?;
    if ours.is_empty() {
        return Ok(());
    }
    if pulse.default_sink()?.as_deref() == Some(SINK)
        && let Some(speakers) = ours.iter().find_map(|(_, restore)| restore.clone())
    {
        pulse.set_default_sink(&speakers)?;
    }
    for (index, _) in ours {
        pulse.unload(index)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_output_to_restore_is_read_back_from_the_sink() {
        let argument = "sink_name=mickey sink_properties=\"device.description=Mickey mickey.restore=alsa_output.pci-0000_00_1f.3.analog-stereo\"";
        assert_eq!(
            super::restore_of(argument).as_deref(),
            Some("alsa_output.pci-0000_00_1f.3.analog-stereo")
        );
        assert_eq!(super::restore_of("sink_name=mickey"), None);
    }

    /// Run by hand on a Linux desktop: only reads the default output.
    #[test]
    #[ignore = "needs a sound server"]
    fn the_default_output_is_read() {
        for _ in 0..3 {
            let default = super::Pulse::connect().unwrap().default_sink().unwrap();
            println!("default: {default:?}");
        }
    }

    /// Run by hand on a Linux desktop: takes the sound off the speakers for
    /// two seconds, then gives it back. Prints the default output each time.
    #[test]
    #[ignore = "switches this PC's sound output for two seconds"]
    fn divert_and_give_back() {
        let default = || super::Pulse::connect().unwrap().default_sink().unwrap();
        println!("before: {:?}", default());
        let diverted = super::divert().unwrap();
        println!("diverted: {:?}", default());
        std::thread::sleep(std::time::Duration::from_secs(2));
        drop(diverted);
        println!("after: {:?}", default());
    }
}
